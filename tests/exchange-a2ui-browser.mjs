import assert from "node:assert/strict";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
const base = new URL(process.argv[2]);
try {
  for (const width of [390, 1200]) {
    const page = await browser.newPage({viewport:{width,height:850}});
    const errors = [];
    page.on("pageerror", e => errors.push(e.message));
    await page.goto(base.href);
    const themes = page.getByRole("region",{name:"Recurring themes"});
    await themes.getByRole("button",{name:/Rule conflict/}).click();
    const canvas = page.getByRole("region",{name:"Answer canvas"});
    const card = (await (await page.request.get(new URL("/api/sidecar/findings?kind=conflict",base).href)).json()).rows[0];
    assert.equal(card.threads,2);
    await canvas.locator('[data-group="' + card.id + '"]').waitFor();
    assert.equal(await canvas.locator("[data-group]").getAttribute("data-trace"),card.evidence.trace_id);
    await page.getByRole("navigation",{name:"Workflows"}).getByRole("link",{name:"Exchange agent",exact:true}).click();
    const input = page.getByLabel("Question about workflow evidence");
    await input.fill("What did the customer ask for, and did the approval check change the outcome?");
    await input.press("Enter");
    await canvas.locator('[data-view="receipt"]').getByText("exchange requested",{exact:true}).waitFor();
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    assert.deepEqual(errors,[]);
    await page.close();
  }
} finally {
  await browser.close();
}
