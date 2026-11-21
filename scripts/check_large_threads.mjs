// Benchmark scope-preserving navigation against a populated sidecar.
import assert from "node:assert/strict";
import { performance } from "node:perf_hooks";

const [origin, workflow] = process.argv.slice(2);
if (!origin || !workflow) throw new Error("Usage: node scripts/check_large_threads.mjs URL WORKFLOW");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
try {
  for (const width of [1152, 390]) {
    const page = await browser.newPage({ viewport:{width,height:850} });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    const scope = new URL(origin);
    scope.searchParams.set("view", "threads");
    const list = page.locator("#content > .list li a");
    const start = performance.now();
    await page.goto(scope.href, {timeout:120000});
    await list.first().waitFor({timeout:120000});
    const allThreadsMs = Math.round(performance.now() - start);
    assert.equal(await list.count(),15);
    await page.locator("#filters > summary").click();
    await page.getByLabel("Workflow",{exact:true}).fill(workflow);
    const filterStart = performance.now();
    await page.getByRole("button",{name:"Apply filters",exact:true}).click();
    await list.first().waitFor({timeout:120000});
    const filteredMs = Math.round(performance.now() - filterStart);
    assert.equal(new URL(page.url()).searchParams.get("workflow"),workflow);
    assert.equal(await list.count(),15);
    const current = page.getByRole("navigation",{name:"Workspace"}).getByRole("link",{name:"Threads",exact:true});
    assert.equal(await current.getAttribute("aria-current"),"page");

    const next = page.getByRole("navigation",{name:"Pages"}).getByRole("link",{name:"Next",exact:true});
    const pageStart = performance.now();
    await next.click();
    await list.first().waitFor({timeout:120000});
    const nextPageMs = Math.round(performance.now() - pageStart);
    assert.equal(new URL(page.url()).searchParams.get("offset"),"15");
    assert.equal(new URL(page.url()).searchParams.get("workflow"),workflow);

    const threadStart = performance.now();
    await list.first().click();
    const calls = page.locator(".dialogue-step");
    await calls.first().waitFor({timeout:120000});
    const threadMs = Math.round(performance.now() - threadStart);
    assert.ok(new URL(page.url()).searchParams.has("thread"));
    assert.equal(new URL(page.url()).searchParams.get("workflow"),workflow);
    const openCall = calls.first().getByRole("link",{name:"Open call",exact:true});
    const address = new URL(await openCall.getAttribute("href"),origin);
    const focused = new URL(page.url());
    focused.searchParams.set("focus_trace",address.searchParams.get("trace"));
    focused.searchParams.set("focus_call",address.searchParams.get("call"));
    await page.goto(focused.href, {timeout:120000});
    const selected = page.locator("#selected-call");
    await selected.waitFor({timeout:120000});
    assert.equal(new URL(await selected.getByRole("link",{name:"Open call",exact:true}).getAttribute("href"),origin).searchParams.get("call"),
      address.searchParams.get("call"));
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    assert.deepEqual(errors,[]);
    console.log(JSON.stringify({workflow,width,allThreadsMs,filteredMs,nextPageMs,threadMs,callsOnPage:await calls.count()}));
    await page.close();
  }
} finally {
  await browser.close();
}
