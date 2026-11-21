import assert from "node:assert/strict";

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
const [base, workflow = "policy-workflow"] = process.argv.slice(2);
try {
  for (const width of [390, 1200]) {
    const page = await browser.newPage({viewport:{width,height:850}});
    const filters = "?" + new URLSearchParams({workflow,kind:"shared_identifier"});
    const [card] = (await (await page.request.get(base + "/api/sidecar/findings" + filters)).json()).rows;
    const current = {trace:card.evidence.trace_id,call:card.evidence.call_id};
    for (const [label, call] of [["Inspect this call",current],["Inspect earlier call",card.evidence.detail.peer]]) {
      await page.goto(base + "/" + filters);
      const question = page.getByRole("textbox",{name:"Question about workflow evidence"});
      await question.fill("Show an example of References from shared text");
      await question.press("Enter");
      const answer = page.getByRole("region",{name:"Answer canvas"});
      await answer.getByText(card.evidence.detail.matches[0].input_excerpt,{exact:true}).first().waitFor({timeout:5000});
      await answer.getByRole("button",{name:label,exact:true}).click();
      const reference = answer.locator(".canvas-reference");
      await reference.waitFor({state:"attached"});
      assert.equal(await reference.getAttribute("data-trace"),call.trace);
      assert.equal(await reference.getAttribute("data-call"),call.call);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    }
    await page.close();
  }
} finally {
  await browser.close();
}
