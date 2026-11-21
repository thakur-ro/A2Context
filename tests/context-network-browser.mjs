import assert from "node:assert/strict";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const width of [1280,390]) {
    const page = await browser.newPage({viewport:{width,height:850}});
    await page.goto(process.argv[2]);
    await page.getByRole("heading",{name:"What would you like to investigate?"}).waitFor();
    await page.evaluate(async () => {
      const result = await api("findings",{kind:"conflict"});
      const template = result.rows.find(card => card.kind === "conflict");
      const rows = Array.from({length:2400},(_,i) => {
        const card = structuredClone(template);
        card.id = String(i);
        card.evidence.workflow = "workflow-"+(i%60);
        card.evidence.detail.sources[0].source = "policy-"+i;
        return card;
      });
      document.querySelector("#content").replaceChildren(contextNetwork(rows));
    });
    const map = page.getByRole("region",{name:"Context map"});
    const tree = map.getByRole("region",{name:"Findings by workflow"});
    const focus = map.getByLabel("Focus a node");
    assert.equal(await focus.inputValue(),"workflows");
    assert.equal(await focus.locator("option").count(),1);
    assert.equal(await tree.locator("li").count(),8);
    await tree.getByText("2400 findings",{exact:true}).waitFor();
    const workflows = new Set();
    for (let p=0; p<8; p++) {
      if (p) await tree.getByRole("button",{name:"Next",exact:true}).click();
      for (const name of await tree.locator("li button span").allTextContents()) workflows.add(name);
    }
    assert.equal(workflows.size,60);
    await tree.getByRole("button",{name:/^workflow-59\b/}).press("Enter");
    await tree.getByRole("button",{name:/Rule conflict.*40 findings/}).click();
    for (let p=1; p<5; p++) await tree.getByRole("button",{name:"Next",exact:true}).click();
    await tree.locator("li button").last().click();
    assert.equal(await focus.inputValue(),"finding:2399");
    await map.getByRole("link",{name:"Inspect this evidence →"}).waitFor();
    const query = map.getByRole("searchbox");
    await query.fill("policy-");
    await map.getByText(/first 30 listed/).waitFor();
    assert.ok(await focus.locator("option").count() <= 32);
    await query.fill("policy-2399");
    await focus.selectOption({label:"source · policy-2399"});
    assert.equal(await map.locator(".network-readout strong").innerText(),"policy-2399");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await page.close();
  }
  console.log("Verified all 60 workflows are reachable with 2,400 synthetic findings at desktop and mobile widths.");
} finally {
  await browser.close();
}
