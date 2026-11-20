import assert from "node:assert/strict";

const base = new URL(process.argv[2]);
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const width of [1152,390]) {
    const page = await browser.newPage({viewport:{width,height:850}});
    const errors = [];
    page.on("pageerror", err => errors.push(err.message));
    await page.goto(new URL("/?workflow=policy-workflow&user=pat", base).href);
    const card = page.getByRole("region",{name:"Recommendation cards"}).getByRole("article");
    await card.getByRole("heading",{name:"Check sources for Exchange authorization"}).waitFor();
    await card.getByText(/2 conversations/).waitFor();
    await page.getByRole("button",{name:"Show the context map"}).click();
    const map = page.getByRole("region",{name:"Context map"});
    await map.getByLabel("Focus a node").selectOption({label:"finding · Check sources for Exchange authorization"});
    await map.getByRole("region",{name:"Configured alternatives"}).getByText(/Still unconfirmed: latest-customer-message/).waitFor();
    assert.equal(await map.locator(".network-node.unconfirmed").count(),2);
    assert.equal(await map.locator(".network-node.source").count(),1);
    if (width > 1000 && process.env.SOURCE_GAP_CAPTURE) await map.screenshot({path:process.env.SOURCE_GAP_CAPTURE});
    await map.getByRole("button",{name:"unconfirmed · latest-customer-message"}).click();
    await map.getByText("No matching source anchor in this request",{exact:true}).last().waitFor();
    await map.getByRole("link",{name:"Inspect this evidence →"}).click();
    await page.getByText(/Still unconfirmed: latest-customer-message/).first().waitFor();
    assert.equal(new URL(page.url()).searchParams.get("user"),"pat");
    assert.ok(new URL(page.url()).searchParams.get("call"));
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    assert.deepEqual(errors,[]);
    await page.close();
  }
  console.log("Verified source-gap cards, distinct unconfirmed nodes and exact call links at two viewport widths.");
} finally {
  await browser.close();
}
