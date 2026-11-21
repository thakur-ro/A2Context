// Measure the opening page and evidence drilldown against an imported corpus.
import assert from "node:assert/strict";
import { performance } from "node:perf_hooks";

const [origin, ...workflows] = process.argv.slice(2);
if (!origin || !workflows.length) throw new Error("Usage: node scripts/check_scale_browser.mjs URL WORKFLOW [WORKFLOW...]");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
try {
  const landing = await browser.newPage({viewport:{width:1152,height:850}});
  await landing.goto(origin);
  await landing.getByRole("heading",{name:"Welcome to A2Context"}).waitFor({timeout:60000});
  const sidebar = landing.getByRole("navigation",{name:"Workflows"});
  await sidebar.getByRole("link",{name:"All workflows",exact:true}).waitFor();
  const entries = await sidebar.getByRole("link").evaluateAll(links => links.map(link => new URL(link.href).searchParams.get("workflow")));
  assert.ok(workflows.every(workflow => entries.includes(workflow)));
  assert.equal(await landing.locator("#filters").isVisible(),false);
  console.log(JSON.stringify({workspace:true,workflows:entries.filter(Boolean)}));
  await landing.close();
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
      await page.locator(".workflow-home .canvas-lead a2ui-basic-row").waitFor({timeout:60000});
      const readyMs = Math.round(performance.now() - start);
      const openingApi = {...api};
      const ask = page.getByRole("textbox",{name:"Question about workflow evidence"});
      let evidenceMs = null;
      let matchingLocations = false;
      let neighboringMessages = null;
      if (await ask.isVisible()) {
        const open = performance.now();
        await ask.fill("Which inputs were repeated?");
        await ask.press("Enter");
        const evidence = page.locator(".canvas-answer .canvas-result[data-call]");
        await evidence.waitFor();
        matchingLocations = await page.getByText("Two matching locations in this request",{exact:true}).isVisible();
        assert.equal(new URL(page.url()).searchParams.get("workflow"),workflow);
        const ref = await evidence.evaluate(node => ({trace:node.dataset.trace,call:node.dataset.call,group:node.dataset.group}));
        const verificationStart = performance.now();
        if (matchingLocations) {
          const detail = await page.request.get(new URL("/api/sidecar/trace",origin).href,
            {params:{id:ref.trace}}).then(response => response.json());
          const repeat = detail.findings.find(item => item.group === ref.group && item.call_id === ref.call);
          assert.equal(repeat.kind,"duplicate");
          const context = detail.trace.calls.find(call => call.id === repeat.call_id).context;
          const copies = repeat.detail.sources.map(source => context.find(item => item.id === source.id));
          const [first,last] = copies.map(item => item.evidence.message_index).sort((a,b) => a-b);
          if (copies.every(item => item.evidence.field === "gen_ai.input.messages") &&
              Number.isInteger(first) && Number.isInteger(last) && first < last) {
            const interval = page.locator(".canvas-intervening");
            await interval.getByText("What came between these copies?").click();
            const positions = new Set(context.filter(item => item.evidence.field === "gen_ai.input.messages" &&
              item.evidence.message_index > first && item.evidence.message_index < last).map(item => item.evidence.message_index));
            neighboringMessages = positions.size;
            assert.equal(await interval.locator(".dialogue-output").count(),Math.min(4,positions.size));
            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
          }
        }
        const verificationMs = performance.now() - verificationStart;
        await ask.fill("Show the exact inputs");
        await ask.press("Enter");
        await evidence.getByRole("heading",{name:"Inputs in the recorded request",exact:true}).waitFor({timeout:60000});
        assert.equal(await evidence.getAttribute("data-call"),ref.call);
        evidenceMs = Math.round(performance.now() - open - verificationMs);
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      assert.deepEqual(errors,[]);
      console.log(JSON.stringify({workflow,width,readyMs,evidenceMs,matchingLocations,neighboringMessages,openingApi}));
      await page.close();
    }
  }
} finally {
  await browser.close();
}
