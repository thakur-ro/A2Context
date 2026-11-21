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
const replays = [
  {base:publicSample,question:"Let me see the order database under both versions of the approval check.",workflow:"exchange-agent",view:"receipt"},
  {base:publicSample,question:"Did the cache edit change the model's answer about the exchange?",workflow:"exchange-agent",view:"proposal"},
  {base:publicSample,question:"Compare the data refueling balance with and without the check.",workflow:"telecom-refuel",view:"receipt"},
  {base:publicSample,question:"Did the model propose something different for refuel_data after the source edit?",workflow:"telecom-refuel",view:"proposal"},
  {base:publicSample,question:"What did the customer ask for, and did the approval check change the outcome?",workflow:"exchange-agent",view:"combined",parts:["message","receipt"]},
  {base:publicSample,question:"Compare the model proposal and the data refueling balance after the edit.",workflow:"telecom-refuel",view:"combined",parts:["proposal","receipt"]},
  {base:publicSample,question:"Show what the caller requested alongside the refuel_data proposal and the refueling balance after the edit.",workflow:"telecom-refuel",view:"combined",parts:["message","proposal","receipt"]},
];
const cases = {development,transfer,replays}[set];
if (!cases) throw new Error("Choose development, transfer, or replays.");

async function matchesReplay(page, answer, base, workflow, view, parts) {
  const surface = parts ? answer.locator("[data-view]").first() : view === "receipt" ? answer.getByRole("region",{name:"Sandbox dispatch comparison"}) : answer.locator('[data-view="proposal"]');
  if (!await surface.count()) return false;
  const trace = await surface.getAttribute("data-trace"), call = await surface.getAttribute("data-call");
  const report = await surface.getAttribute("data-report");
  const response = await page.request.get(new URL("/api/sidecar/findings",base).href,{params:{workflow}});
  assert.equal(response.status(),200);
  const {rows} = await response.json();
  assert.ok(rows.some(r => r.evidence.detail.sandbox_dispatch),"Expected a measured dispatch fixture for " + workflow);
  const finding = rows.find(r => r.evidence.trace_id === trace && r.evidence.call_id === call && r.evidence.detail.report_id === report);
  if (!finding) return false;
  const {sandbox_dispatch:receipt,response_pair:pair} = finding.evidence.detail;
  if (parts) {
    const refs = await answer.locator("[data-view]").evaluateAll(nodes => nodes.map(n => [n.dataset.view,n.dataset.trace,n.dataset.call,n.dataset.report]));
    assert.deepEqual(refs,parts.map(part => [part,trace,call,report]));
    if (parts.includes("message")) {
      const captured = await page.request.get(new URL("/api/sidecar/thread-calls",base).href,{params:{trace,workflow,focus_trace:trace,focus_call:call,limit:1}});
      const [row] = (await captured.json()).rows;
      assert.equal(row.call,call);
      const text = row.dialogue.user?.text || "The recording has no readable user message at this call.";
      assert.equal(await answer.locator('[data-view="message"]').getByText(text,{exact:true}).count(),1);
    }
  }
  if (view === "proposal" || parts?.includes("proposal")) {
    const arms = await answer.locator('[data-view="proposal"] a2ui-card').allInnerTexts();
    assert.ok(arms.length === 2 && [pair.original,pair.edited].every((p,i) => arms[i].includes(p.label) && arms[i].includes(p.outcome_preview)));
    if (view === "proposal") return true;
  }
  const target = receipt.states.find(row => row.key === receipt.target);
  if (parts) {
    const arms = answer.locator('[data-view="receipt"] a2ui-card');
    for (const [i,value] of [target.without_guard,target.with_guard].entries()) {
      assert.equal(await arms.nth(i).getByText(String(value).replaceAll("_"," "),{exact:true}).count(),1);
    }
    return true;
  }
  const printed = await surface.getByRole("row").filter({has:page.getByRole("rowheader",{name:receipt.target,exact:true})}).getByRole("cell").allInnerTexts();
  return JSON.stringify(printed) === JSON.stringify([target.before,target.without_guard,target.with_guard].map(x => String(x).replaceAll("_"," ")));
}
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const {base,question,accept,diagram,receipt,workflow,view,parts} of cases) {
    for (const partner of model === "-" ? [""] : ["",model]) {
      const page = await browser.newPage({reducedMotion:"reduce"});
      try {
        await page.goto(base);
        const input = page.getByLabel("Question about workflow evidence");
        await input.waitFor();
        const picker = page.getByLabel("Analysis partner");
        if (partner || await picker.isVisible()) await picker.selectOption(partner);
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
        if (view) {
          matched = await matchesReplay(page,answer,base,workflow,view,parts);
        } else if (diagram) {
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
        if (view) assert.ok(matched,"Expected the measured " + view + " for " + workflow);
      } finally { await page.close(); }
    }
  }
} finally { await browser.close(); }
