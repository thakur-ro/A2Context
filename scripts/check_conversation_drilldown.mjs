// Run against a sidecar containing workflows with recorded duplicate inputs.
import assert from "node:assert/strict";
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const base = new URL(process.argv[2]);
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  const page = await browser.newPage();
  const read = async (path,params = {}) => {
    const response = await page.request.get(new URL("/api/sidecar/" + path,base).href,{params});
    assert.ok(response.ok());
    return response.json();
  };
  const {workflows} = await read("workflows");
  const input = page.getByRole("textbox",{name:"Question about workflow evidence"});
  const answer = page.getByRole("region",{name:"Answer canvas"});
  const ask = async question => { await input.fill(question); await input.press("Enter"); };
  let checked = 0;
  await page.goto(base.href);
  for (const workflow of workflows) {
    const params = {workflow,kind:"duplicate",by_thread:"1"};
    const {rows} = await read("occurrences",{...params,limit:"6"});
    if (rows.length < 5) continue;
    await ask("Which conversations had repeated inputs in " + workflow + "?");
    const displayed = answer.getByRole("button",{name:/^Read /});
    for (const start of [0,3]) {
      if (start) await ask("Show more conversations");
      await answer.getByRole("button",{name:"Read " + rows[start].thread,exact:true}).first().waitFor();
      assert.deepEqual((await displayed.allTextContents()).map(text => text.trim()),rows.slice(start,start+3).map(row => "Read " + row.thread));
    }
    const chosen = rows[4];
    const lookup = page.waitForRequest(r => new URL(r.url()).pathname.endsWith("/thread-calls"));
    await ask("Open the second one");
    const query = new URL((await lookup).url()).searchParams;
    for (const [key,value] of Object.entries({workflow,thread:chosen.thread_key,focus_trace:chosen.trace,focus_call:chosen.call})) {
      assert.equal(query.get(key),value);
    }
    await answer.getByRole("heading",{name:chosen.thread,exact:true}).waitFor();
    const note = await answer.getByText(/The finding occurred at call \d+/).innerText();
    await ask("Show call " + /occurred at call (\d+)/.exec(note)[1]);
    const reference = answer.locator(".canvas-reference");
    assert.equal(await reference.getAttribute("data-trace"),chosen.trace);
    assert.equal(await reference.getAttribute("data-call"),chosen.call);
    const summary = await read("summary",{workflow,kind:"duplicate"});
    console.log(JSON.stringify({workflow,conversations:summary.conversations,groups:summary.recommendations,checkedConversation:chosen.thread}));
    checked++;
  }
  assert.ok(checked, "No workflow contained five conversations with repeated inputs");
} finally {
  await browser.close();
}
