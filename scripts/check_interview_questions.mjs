// Paired comparisons on fixed development questions using the actual interview.
import assert from "node:assert/strict";
import {performance} from "node:perf_hooks";

const [demo, publicSample, model, set = "development"] = process.argv.slice(2);
if (!demo || !publicSample || !model) throw new Error("Supply the demo gateway URL, public sample gateway URL, and model ID.");
const development = [
  {base:demo,question:"Was the shopper’s identity confirmed?",accept:text => /Tool result · verify_customer · \{"verified": true\}/.test(text)},
  {base:publicSample,question:"Show requests to undo a plane booking.",accept:text => /Tau2 airline/i.test(text) && /(?:want|need|please|would like|request|can you|help me).{0,40}cancel|cancel (?:my|the|this) (?:reservation|booking|flight)/is.test(text)},
  {base:publicSample,question:"What peer-to-peer cash movements did people ask for in Venmo?",accept:text => /Appworld/i.test(text) && /venmo/i.test(text) && /transaction|transfer|payment|send (?:money|\$?\d+)/i.test(text)},
  {base:publicSample,question:"Where did shoppers ask to swap a jigsaw?",accept:text => /Tau2 retail/i.test(text) && /puzzle|jigsaw/i.test(text) && /exchange|swap/i.test(text)},
  {base:demo,question:"Which words were most sensitive to changing the refund instructions?",diagram:true},
];
const transfer = [
  {base:demo,question:"Did anyone establish who the customer was before working on the exchanges?",accept:text => /Tool result · verify_customer · \{"verified": true\}/.test(text)},
  {base:demo,question:"Let me see the order database under both versions of the approval check.",receipt:true},
  {base:demo,question:"Is there an experiment on how the return-policy edit affected the numbers in the response?",diagram:true},
  {base:publicSample,question:"Show me a phone customer trying to use the internet abroad.",accept:text => /Tau2 telecom/i.test(text) && /roaming|abroad|international/i.test(text)},
  {base:publicSample,question:"Find a developer working on a broken test suite.",accept:text => /Swebench/i.test(text) && /pytest|failed test|failing test|test failure/i.test(text)},
];
const cases = {development,transfer}[set];
if (!cases) throw new Error("Choose development or transfer.");
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const {base,question,accept,diagram,receipt} of cases) {
    for (const partner of ["",model]) {
      const page = await browser.newPage({reducedMotion:"reduce"});
      try {
        await page.goto(base);
        await page.getByLabel("Analysis partner").selectOption(partner);
        const input = page.getByLabel("Question about workflow evidence");
        const answer = page.getByRole("region",{name:"Answer canvas"});
        let requests = 0;
        const queries = [];
        page.on("request",req => {
          const url = new URL(req.url());
          if (url.pathname === "/api/chat/completions") requests++;
          if (url.pathname.endsWith("/activity-search")) queries.push(url.searchParams.get("q"));
        });
        const start = performance.now();
        await input.fill(question);
        await input.press("Enter");
        await answer.getByText(question,{exact:true}).waitFor();
        await page.waitForFunction(() => !document.querySelector(".canvas-answer").hasAttribute("aria-busy"),{},{timeout:130000});
        const table = answer.getByRole("table",{name:"Measured source edits by output token"});
        const first = answer.locator("a2ui-card").first();
        const text = await first.count() ? await first.innerText() : "";
        const elapsed = Math.round(performance.now()-start);
        let matched;
        if (diagram) {
          if (await answer.getByRole("region",{name:"Source edits along the answer"}).count()) await table.waitFor();
          matched = await table.count() === 1 && await table.getByRole("button",{name:/Token 14 ·/}).count() === 1;
        } else if (receipt) {
          matched = await answer.getByRole("region",{name:"Sandbox dispatch comparison"}).count() === 1;
        } else {
          matched = accept(text);
          const number = /^Call (\d+)/.exec(text.trim())?.[1];
          if (number) {
            await input.fill("Show call " + number);
            await input.press("Enter");
            const ref = answer.locator(".canvas-reference");
            await ref.waitFor();
            const id = await ref.getAttribute("data-trace"), call = await ref.getAttribute("data-call");
            const response = await page.request.get(new URL("/api/sidecar/trace",base).href,{params:{id,call}});
            assert.ok((await response.json()).trace.calls.some(r => r.id === call));
          }
        }
        assert.ok(requests <= (partner ? 2 : 0));
        console.log(JSON.stringify({question,partner:partner || "workspace",matched,requests,ms:elapsed,queries}));
      } finally { await page.close(); }
    }
  }
} finally { await browser.close(); }
