import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const dir = new URL("../examples/runtime-comparison/pending-routing/",import.meta.url);
const fixture = async file => JSON.parse(await readFile(new URL(file,dir),"utf8"));
const request = await fixture("request.json");
const shadow = await fixture("shadow-report.json");
const catalogSize = 1200;
const large = Array.from({length:catalogSize}, (_,i) => ({name:"lookup_"+i}));
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const width of [360,390,1152]) {
    const page = await browser.newPage({viewport:{width,height:900}});
    const errors = [];
    page.on("pageerror",e=>errors.push(e.message));
    await page.goto(process.argv[2]+"/?view=preview");
    const catalog = page.getByRole("region",{name:"Tool catalog scope"});
    const load = data => page.locator('input[type="file"]').setInputFiles({name:"request.json",mimeType:"application/json",buffer:Buffer.from(JSON.stringify(data))});
    await page.getByRole("button",{name:"Try a measured tool choice"}).click();
    await page.getByRole("link",{name:"Inspect proposals under a stock-only scope"}).click();
    const condition = catalog.getByLabel("Proposal trial condition");
    await condition.waitFor();
    const change = catalog.getByRole("region",{name:"Observed proposal change"});
    await change.getByRole("heading",{name:"What changed for price?"}).waitFor();
    await change.getByText("The model proposed this function in 0/1 original trials and 1/1 after editing request-type.").waitFor();
    await change.getByText("Compare all edits").click();
    const comparisons = change.getByRole("row");
    assert.deepEqual(await comparisons.allTextContents(), [
      "Source editProposedCompleted","Original request0/11/1","request-type1/11/1",
      "store-name0/11/1","request-type + store-name1/11/1"]);
    assert.ok(await catalog.evaluate(el => el.scrollWidth <= el.clientWidth));
    await change.getByText("Compare all edits").click();
    const row = catalog.getByRole("listitem");
    await row.filter({hasText:"price"}).getByText("Proposed in 0/1 trials",{exact:true}).waitFor();
    await condition.selectOption({label:"Edit request-type"});
    await row.filter({hasText:"price"}).getByText("Original 0/1 · Edited 1/1",{exact:true}).waitFor();
    const provenance = catalog.getByText("Completed trials · 1/1",{exact:true});
    await provenance.click();
    await catalog.getByText("/messages/1/content",{exact:true}).waitFor();
    await provenance.click();
    assert.ok(await catalog.evaluate(el => el.scrollWidth <= el.clientWidth));
    if (process.env.CATALOG_CAPTURE && width === 1152) await catalog.screenshot({path:process.env.CATALOG_CAPTURE});
    await catalog.getByRole("button",{name:/^Permitted/}).click();
    await row.filter({hasText:"stock"}).getByText("Original 1/1 · Edited 0/1",{exact:true}).waitFor();
    await condition.selectOption({label:"Edit store-name"});
    await row.filter({hasText:"stock"}).getByText("Original 1/1 · Edited 1/1",{exact:true}).waitFor();
    await condition.selectOption({label:"Original request"});
    await row.filter({hasText:"stock"}).getByText("Proposed in 1/1 trials",{exact:true}).waitFor();
    await load({request,allowed_tools:["stock"],shadow:{...shadow,captured_request_sha256:"f".repeat(64)}});
    await catalog.getByText("No verified proposal trials for this request.").waitFor();
    assert.equal(await condition.count(),0);
    assert.equal(await change.count(),0);
    const body = JSON.stringify({tools:large});
    await load({request:{version:1,attributes:{"a2context.request.body":body}},allowed_tools:["lookup_14"]});
    await catalog.getByRole("button",{name:/Outside scope\s*1199/}).waitFor();
    assert.equal(await row.count(),8);
    assert.ok(await catalog.evaluate(el => el.scrollWidth <= el.clientWidth));
    await catalog.getByRole("button",{name:"Next",exact:true}).click();
    await catalog.getByText("9–16 of 1199").waitFor();
    await catalog.getByRole("searchbox").fill("lookup_1199");
    await row.getByText("/tools/1199").waitFor();
    assert.equal(await row.count(),1);
    await catalog.getByRole("searchbox").fill("nonexistent");
    await catalog.getByText("No matching functions.").waitFor();
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth));
    if (process.env.WATCHLIST_PREVIEW) {
      await load(JSON.parse(await readFile(process.env.WATCHLIST_PREVIEW,"utf8")));
      await change.getByRole("heading",{name:"What changed for get_watchlist?"}).waitFor();
      await change.getByText(/0\/1 original trials and 1\/1 after editing update-message/).waitFor();
      await change.getByText("Compare all edits").click();
      assert.deepEqual(await change.getByRole("row").allTextContents(),[
        "Source editProposedCompleted","Original request0/11/1","lookup-message0/11/1",
        "update-message1/11/1","lookup-message + update-message1/11/1"]);
      assert.ok(await catalog.evaluate(el => el.scrollWidth <= el.clientWidth));
      if (width === 1152 && process.env.WATCHLIST_CAPTURE) await catalog.screenshot({path:process.env.WATCHLIST_CAPTURE});
    }
    assert.deepEqual(errors,[]);
    await page.close();
  }
  console.log("Verified catalog branches, verified original and edited proposals, stale evidence and 1,200 searchable tool entries on desktop and mobile.");
} finally { await browser.close(); }
