import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  const base = process.argv[2];
  const page = await browser.newPage({viewport:{width:390,height:844}});
  await page.goto(base);
  const setup = page.getByRole("region",{name:"Receiver setup"});
  await page.getByRole("heading",{name:"Connect your workflow"}).waitFor();
  const endpoint = base + "/v1/traces";
  assert.equal(await setup.locator("code").innerText(),
    "export OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=" + endpoint);
  await setup.getByRole("link",{name:"Inspect a sample exchange request"}).click();
  await page.getByText(/captured inputs · 1 finding$/).waitFor();
  await page.getByRole("link",{name:"Check another request"}).click();
  await page.locator('input[type="file"]').setInputFiles("examples/bfcl-transfer/mlx/tool-scope/preview.json");
  const catalog = page.getByRole("region",{name:"Tool catalog scope"});
  await catalog.getByRole("heading",{name:"Review tools outside the workflow scope"}).waitFor();
  await catalog.getByText("3 tool definitions · 2 outside scope · 0 unclassified").waitFor();
  assert.deepEqual(await catalog.getByRole("listitem").locator("code").allTextContents(),
    ["/tools/1","/tools/2"]);
  for (const [renderer, status] of [["liquid","Identical input tokens (192 tokens)"],["community","Different input tokens (192 tokens)"]]) {
    await page.locator('input[type="file"]').setInputFiles("examples/local-mlx-tool/rendering/"+renderer+"-preview.json");
    const rendering = page.getByRole("region",{name:"Model input rendering"});
    await rendering.getByText(status,{exact:false}).waitFor();
    await rendering.getByText("/messages/2/tool_calls",{exact:true}).waitFor();
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth));
  }
  await page.goto(base);
  await setup.waitFor();
  const sample = await readFile(new URL("../examples/exchange-approval/exchange.otlp.json",import.meta.url),"utf8");
  const response = await page.request.post(endpoint,{data:sample,headers:{"Content-Type":"application/json"}});
  assert.ok(response.ok());
  await page.reload();
  await page.getByRole("heading",{name:"Telemetry received"}).waitFor();
  await setup.getByText("3 traces · 3 inference spans").waitFor();
  const href = await setup.getByRole("link",{name:"Check receiver status"}).getAttribute("href");
  const health = await page.request.get(new URL(href,base).href);
  assert.equal((await health.json()).content_capture,false);
  assert.equal(await setup.getByRole("link",{name:"Threads"}).count(),0);
  await page.goto(base+"/?workflow=unrelated");
  await page.getByText("No recommendations match these filters.").waitFor();
  assert.equal(await setup.count(),0);
  const attribute = (key, stringValue) => ({key,value:{stringValue}});
  const toolOnly = {resourceSpans:[{
    resource:{attributes:[attribute("service.name","tool-observations")]},
    scopeSpans:[{spans:[{traceId:"c9f482b0390ef6cca6de2d5c08cfff8d",spanId:"a98605bfdcc72819",
      attributes:[attribute("gen_ai.operation.name","execute_tool"),attribute("gen_ai.tool.name","lookup_order"),
        attribute("gen_ai.conversation.id","Order observation")]}]}],
  }]};
  assert.ok((await page.request.post(endpoint,{data:toolOnly})).ok());
  await page.goto(base+"/?workflow=tool-observations");
  await page.getByRole("heading",{name:"Captured activity",exact:true}).waitFor();
  await page.getByRole("heading",{name:"Tool observations",exact:true}).waitFor();
  await page.getByText(/1 conversation with recorded spans/).waitFor();
  const question = page.getByLabel("Question about workflow evidence");
  await question.fill("Show recorded activity");
  await question.press("Enter");
  await page.getByRole("button",{name:"Read Order observation",exact:true}).waitFor();
  await question.fill("Find conversation order observation");
  await question.press("Enter");
  await page.getByRole("heading",{name:"Order observation",exact:true}).waitFor();
  await page.getByText("Message and result text unavailable",{exact:true}).waitFor();
  await question.fill("What happened next?");
  await question.press("Enter");
  await page.getByRole("heading",{name:"End of the captured conversation"}).waitFor();
  assert.equal(new URL(page.url()).searchParams.get("workflow"),"tool-observations");
  assert.equal(await page.getByRole("link",{name:"Browse conversations",exact:true}).count(),0);
  await page.goto(base);
  await page.getByRole("link",{name:"Tool observations",exact:true}).waitFor();
  await page.getByRole("button",{name:"Compare all workflows",exact:true}).waitFor();
  await question.fill("Show recent conversations in Tool observations");
  await question.press("Enter");
  await page.getByText("1 captured conversation in Tool observations",{exact:true}).waitFor();
  await page.getByRole("button",{name:"Read Order observation",exact:true}).waitFor();
  await question.fill("Compare Exchange agent and Tool observations");
  await question.press("Enter");
  await page.getByRole("heading",{name:"Workflow comparison"}).waitFor();
  await page.getByText("0 of 3 captured conversations with findings (0%)",{exact:true}).waitFor();
  await page.getByText("0 of 1 captured conversations with findings (0%)",{exact:true}).waitFor();
  assert.equal(await page.locator("#filters").isVisible(),false);
  assert.equal(await page.locator("a2ui-surface.canvas-evidence-sections a2ui-card").count(),2);
  console.log("Verified setup, recorded activity without findings, and workflow comparison using captured counts.");
} finally {
  await browser.close();
}
