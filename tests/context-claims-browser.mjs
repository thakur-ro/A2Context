import assert from "node:assert/strict";

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const [base, toolsWorkflow, papersWorkflow, contractWorkflow] = process.argv.slice(2);
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const width of [390, 1200]) {
    for (const [workflow, kind, label, action] of [
      [toolsWorkflow, "capability_claim", "Reported tool limitation", "Inspect comparison call"],
      ...(contractWorkflow ? [[contractWorkflow, "missing_tool_argument", "Missing tool argument", "Inspect this call"]] : []),
      [papersWorkflow, "example_citation", "Citation from an example", "Inspect this call"],
    ]) {
      const page = await browser.newPage({viewport:{width,height:850}});
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      const filters = new URLSearchParams({workflow,kind});
      const response = await page.request.get(base + "/api/sidecar/findings?" + filters);
      const [card] = (await response.json()).rows;
      const comparison = card.discovery_example;
      await page.goto(base + "/?" + filters);
      const input = page.getByRole("textbox",{name:"Question about workflow evidence"});
      await input.fill("Show an example of " + label);
      await input.press("Enter");
      const answer = page.getByRole("region",{name:"Answer canvas"});
      await answer.getByRole("button",{name:action,exact:true}).click({timeout:5000});
      const ref = answer.locator(".canvas-reference");
      await ref.waitFor({state:"attached"});
      assert.equal(await ref.getAttribute("data-trace"),comparison?.trace || card.evidence.trace_id);
      assert.equal(await ref.getAttribute("data-call"),comparison?.call || card.evidence.call_id);
      await answer.getByText(comparison?.result || card.evidence.detail.check?.schema_pointer || card.evidence.detail.citation,{exact:true}).first().waitFor();
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      assert.deepEqual(errors,[]);
      await page.close();
    }
  }
  console.log("Recommendations opened the recorded evidence on desktop and mobile.");
} finally {
  await browser.close();
}
