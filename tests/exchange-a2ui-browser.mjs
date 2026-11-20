import assert from "node:assert/strict";

const base = new URL(process.argv[2]);
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
try {
  for (const viewport of [{ width: 1152, height: 740 }, { width: 390, height: 844 }]) {
    const page = await browser.newPage({ viewport });
    const errors = [];
    const externalRequests = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("request", request => {
      if (new URL(request.url()).origin !== base.origin) externalRequests.push(request.url());
    });
    await page.goto(new URL("/?workflow=exchange-agent", base).href);
    const lead = page.getByRole("link", { name: /See what changed at dispatch/ });
    await lead.getByRole("heading", { name: "See what changed at dispatch", exact: true }).waitFor();
    assert.equal(await lead.locator("a2ui-surface").evaluate(surface => surface.surface.protocolVersion), "1.0");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await lead.click();
    await page.getByRole("link", { name: "Review recommendation", exact: true }).click();
    const comparison = page.getByRole("region", { name: "Sandbox dispatch comparison" }).first();
    await comparison.waitFor();

    await page.getByRole("link", { name: "Before inference", exact: true }).click();
    await page.getByRole("link", { name: "Check the next exchange", exact: true }).click();
    const earlier = page.getByRole("region", { name: "Earlier dispatch replays" });
    await earlier.getByRole("heading", { name: "Test the dispatch check on this request", exact: true }).waitFor();
    await earlier.getByText(/2 distinct earlier inputs/).waitFor();
    await page.getByRole("heading", { name: "Conflicting values for exchange authorization", exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await earlier.getByRole("button", { name:"Open earlier sandbox comparison" }).press("Enter");
    await comparison.waitFor();
    const params = new URL(page.url()).searchParams;
    assert.equal(params.get("workflow"), "exchange-agent");
    for (const key of ["recommendation", "factor", "trace", "call", "report"]) assert.ok(params.get(key));
    assert.deepEqual(errors, []);
    assert.deepEqual(externalRequests, []);
    await page.close();
  }
  console.log("Verified A2UI rendering and exact evidence navigation on desktop and mobile.");
} finally {
  await browser.close();
}
