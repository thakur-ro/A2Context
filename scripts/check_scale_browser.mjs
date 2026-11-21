// Measure the opening page and evidence drilldown against an imported corpus.
import assert from "node:assert/strict";
import { performance } from "node:perf_hooks";

const [origin, ...workflows] = process.argv.slice(2);
if (!origin || !workflows.length) throw new Error("Usage: node scripts/check_scale_browser.mjs URL WORKFLOW [WORKFLOW...]");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
try {
  for (const workflow of workflows) {
    for (const width of [1152, 390]) {
      const page = await browser.newPage({ viewport:{width,height:850}, reducedMotion:"reduce" });
      const api = {};
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("requestfinished", request => {
        const endpoint = new URL(request.url()).pathname;
        if (endpoint.startsWith("/api/sidecar/")) api[endpoint.split("/").at(-1)] = Math.round(request.timing().responseEnd);
      });
      const url = new URL(origin);
      url.searchParams.set("workflow", workflow);
      const start = performance.now();
      await page.goto(url.href);
      await page.locator(".canvas-lead h2").waitFor({timeout:60000});
      const readyMs = Math.round(performance.now() - start);
      const openingApi = {...api};
      const browse = page.getByText(/^Browse \d+ (?:recommendations?|repeat groups?)$/);
      let evidenceMs = null;
      let matchingLocations = false;
      if (await browse.isVisible()) {
        await browse.click();
        const open = performance.now();
        await page.getByRole("region",{name:"Recommendation cards"}).getByRole("button",{name:"Inspect",exact:true}).first().click();
        const evidence = page.getByRole("link",{name:"Open exact evidence →"});
        await evidence.waitFor();
        matchingLocations = await page.getByText("Two matching locations in this request",{exact:true}).isVisible();
        assert.equal(new URL(await evidence.getAttribute("href"),origin).searchParams.get("workflow"),workflow);
        await evidence.click();
        await page.getByRole("heading",{name:"Recommendation",exact:true}).waitFor({timeout:60000});
        assert.ok(new URL(page.url()).searchParams.get("call"));
        evidenceMs = Math.round(performance.now() - open);
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      assert.deepEqual(errors,[]);
      console.log(JSON.stringify({workflow,width,readyMs,evidenceMs,matchingLocations,openingApi}));
      await page.close();
    }
  }
} finally {
  await browser.close();
}
