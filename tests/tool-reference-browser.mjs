import assert from "node:assert/strict";

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const base = new URL(process.argv[2]);
const workflow = "handoff-desk";
const attr = (key,stringValue) => ({key,value:{stringValue}});
const result = {role:"tool",tool_call_id:"search",content:"Found a document"};
const invoke = {role:"assistant",tool_calls:[{id:"search",type:"function",function:{name:"search",arguments:"{}"}}]};
const spans = [[invoke,result],[result]].map((messages,i) => ({
  traceId:"f".repeat(31) + i,spanId:"c".repeat(16),name:"Chat",startTimeUnixNano:String(BigInt(Date.now())*1000000n),
  attributes:[attr("gen_ai.operation.name","chat"),attr("gen_ai.request.model","fixture"),
    attr("gen_ai.conversation.id","handoff-" + i),attr("a2context.request.body",JSON.stringify({messages}))],
}));
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  const ingest = await browser.newPage();
  const response = await ingest.request.post(new URL("/v1/traces",base).href,{data:{
    resourceSpans:[{resource:{attributes:[attr("service.name",workflow)]},scopeSpans:[{spans}]}],
  }});
  assert.ok(response.ok());
  await ingest.close();
  for (const width of [390,1200]) {
    const page = await browser.newPage({viewport:{width,height:850}});
    await page.goto(new URL("/?workflow="+workflow,base).href);
    await page.getByText("A tool result refers to an invocation missing from the request",{exact:true}).first().waitFor();
    const input = page.getByRole("textbox",{name:"Question about workflow evidence"});
    const answer = page.getByRole("region",{name:"Answer canvas"});
    await input.fill("Show the tool reference score");
    await input.press("Enter");
    await answer.getByRole("heading",{name:"50% of assessable traces passed"}).waitFor();
    await answer.getByText("100% of known eligible traces assessed",{exact:true}).waitFor();
    await input.fill("Show call 1");
    await input.press("Enter");
    await answer.getByText("/messages/0",{exact:true}).waitFor();
    await answer.getByText("Earlier assistant invocations with this ID: 0",{exact:true}).waitFor();
    assert.equal(await answer.locator(".canvas-reference").getAttribute("data-trace"),"otlp:"+spans[1].traceId);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.close();
  }
  console.log("Verified an automatic tool-reference recommendation, scoped score and exact message on mobile and desktop.");
} finally { await browser.close(); }
