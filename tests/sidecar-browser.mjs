// Run with PLAYWRIGHT_MODULE set to the installed playwright-core module and
// CHROME_BINARY set to a Chrome or Chromium executable.
import assert from "node:assert/strict";
import process from "node:process";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
const base = process.env.SIDECAR_URL || "http://127.0.0.1:8040";
const errors = [];
const conflictCard = (page) => page.locator(".card").filter({ hasText: "Conflicting values" }).first();

const factor = (page, signal) => page.locator(".factor-row").filter({ hasText: signal }).first();
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 950 } });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(base);
  await page.getByRole("heading", { name: "Conversations with findings" }).waitFor();
  assert.equal(await page.locator(".impact-number").innerText(), "15");
  await page.getByText("9 with a measured change").waitFor();
  assert.equal(await page.locator("#navigation a").count(), 2);
  await factor(page, "Choice flipped").waitFor();
  if (process.env.FINDINGS_SCREENSHOT) await page.screenshot({ path: process.env.FINDINGS_SCREENSHOT });
  const coverage = page.locator(".replay-coverage");
  await coverage.getByText("Scored replays in 7 of 15 conversations").click();
  assert.deepEqual(await coverage.locator("dd").allTextContents(), ["0", "0", "7", "8"]);
  await factor(page, "Results varied with source order").click();
  await page.getByText(/in 2 of 4 matched comparisons/).waitFor();
  assert.equal(await page.getByRole("heading", { name: "Match counts held across orders" }).count(), 2);
  assert.equal(await page.locator("tr.different").count(), 4);
  const formatMiss = page.locator(".replacement").filter({ hasText: "BAY WREN" });
  assert.match(await formatMiss.innerText(), /counts held across orders/);
  assert.match(await formatMiss.innerText(), /UNKNOWN/);
  if (process.env.ORDER_SCREENSHOT) await page.locator(".panel.replacement").first().screenshot({ path: process.env.ORDER_SCREENSHOT });
  await formatMiss.getByText("Original trials").click();
  const originalTrial = await formatMiss.getByRole("link").first().getAttribute("href");
  const trialTab = await browser.newPage();
  await trialTab.goto(new URL(originalTrial, base).href);
  await trialTab.locator('[data-replay-id="order-birch-dispatch-guide-2"] [data-context-id="dispatch-guide"]:focus').waitFor();
  await trialTab.close();
  await page.goto(base);

  await factor(page, "Results varied across wordings").click();
  await coverage.getByText("Scored replays in 1 of 1 conversation").click();
  assert.deepEqual(await coverage.locator("dd").allTextContents(), ["1", "0", "0", "0"]);
  const variation = page.locator(".card").filter({ hasText: "Replacement comparison" });
  await variation.getByRole("link").click();
  const replacementPanel = page.locator(".panel").first();
  const changed = replacementPanel.locator("tr.different");
  await changed.first().waitFor();
  assert.equal(await changed.count(), 1);
  assert.match(await changed.innerText(), /routing-note.*0 \/ 1.*1 \/ 1/s);
  assert.deepEqual(await changed.locator("code").allTextContents(), ['"UNKNOWN"', '"LARK"']);
  if (process.env.COMPARISON_SCREENSHOT) await replacementPanel.screenshot({ path: process.env.COMPARISON_SCREENSHOT });
  await page.locator("summary", { hasText: "Underlying evidence" }).click();
  const evidenceLinks = page.getByRole("link", { name: /Open wording [AB] trials/ });
  assert.equal(await evidenceLinks.count(), 2);
  for (const [index, id] of ["unavailable", "neutral"].entries()) {
    const evidenceUrl = await evidenceLinks.nth(index).getAttribute("href");
    const original = await browser.newPage();
    await original.goto(new URL(evidenceUrl, base).href);
    await original.locator('[data-replay-id="two-routing-sources-' + id + '"] [data-context-id="routing-note"]:focus').waitFor();
    await original.close();
  }
  await page.getByRole("link", { name: "Back to source factor" }).click();
  await page.locator(".card").filter({ hasText: "Only the joint edit" }).getByRole("link").click();
  await page.getByText(/Joint interaction: -100 percentage points/).waitFor();
  await page.getByText("How this was measured", { exact: true }).click();
  await page.getByText(/sidecar checked the request digests/).waitFor();
  const arms = page.locator(".panel").first().locator(".comparison > div");
  assert.equal(await arms.count(), 4);
  assert.match(await arms.nth(1).innerText(), /1 \/ 1 matched/);
  assert.match(await arms.nth(3).innerText(), /0 \/ 1 matched/);
  await page.getByRole("link", { name: "Recommendations", exact: true }).click();
  await page.locator("#filters summary").click();
  await page.getByLabel("Organization").fill("demo-retail");
  await page.getByLabel("User", { exact: true }).fill("user-042");
  await page.getByRole("button", { name: "Apply filters" }).click();
  await factor(page, "Conflicting rules").click();
  await conflictCard(page).getByRole("link", { name: "Review recommendation" }).click();
  await page.getByRole("heading", { name: "Recommendation", exact: true }).waitFor();
  assert.equal(new URL(page.url()).searchParams.get("user"), "user-042");
  assert.equal(await page.getByRole("heading", { name: "Span tree" }).count(), 0);
  await page.getByRole("heading", { name: "Resolve the competing rules" }).waitFor();
  await page.locator("summary", { hasText: "Compare recurrence" }).click();
  await page.getByLabel("Compare with a user in the same filters").fill("user-077");
  await page.getByRole("button", { name: "Compare", exact: true }).click();
  await page.getByText("user-077", { exact: true }).waitFor();
  assert.match(await page.locator("details[open]").last().innerText(), /2 traces.*1 trace/s);
  await page.locator("summary", { hasText: "Underlying evidence" }).click();
  await page.getByRole("link", { name: "Open original span" }).click();
  await page.getByRole("heading", { name: "Span tree" }).waitFor();
  const selected = new URL(page.url());
  assert.equal(selected.searchParams.get("user"), "user-042");
  assert.ok(selected.searchParams.get("call"));
  assert.ok(selected.searchParams.get("finding"));
  await page.getByText("Evidence at " + selected.searchParams.get("call"), { exact: true }).first().waitFor();
  await page.locator("#summary a").click();
  await page.getByText("Traces in chronological order.").waitFor();
  assert.equal(new URL(page.url()).searchParams.get("user"), "user-042");

  await page.goto(base);
  await factor(page, "Replay outcome shifted").click();
  const replay = page.locator(".card").filter({ hasText: "Verified replay" }).first();
  await replay.getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText("Matches by trial arm", { exact: true }).first().waitFor();
  await page.locator("summary", { hasText: "Underlying evidence" }).click();
  const replayLink = page.getByRole("link", { name: "Open trial comparison" }).first();
  const legacy = await replayLink.getAttribute("href");
  const legacyId = new URLSearchParams(new URL(legacy, base).hash.slice(1)).get("replay");
  const selectedReport = page.locator("[data-replay-id=" + JSON.stringify(legacyId) + "]");
  await replayLink.click();
  await selectedReport.getByText("Baseline matched the recorded decision in 1 of 1 completed trials", { exact: true }).waitFor();
  await page.goto(base + "/" + new URL(legacy, base).hash);
  await page.waitForURL("**/details?**");
  await selectedReport.getByText("Baseline matched the recorded decision in 1 of 1 completed trials", { exact: true }).waitFor();

  await page.goto(base);
  await factor(page, "Choice flipped").click();
  const measured = page.locator(".card").filter({ hasText: "switched the preferred alternative" });
  await measured.getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText("stock favored", { exact: true }).first().waitFor();
  await page.getByText("price favored", { exact: true }).first().waitFor();
  await page.getByText(/store-name.*-0\.089/).waitFor();
  await page.goto(base);
  await factor(page, "Token score shifted").click();
  await page.locator(".card").filter({ hasText: "Token measurements" }).getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText(/largest tested decoder-block swap/).waitFor();
  await page.getByRole("link", { name: "Before inference" }).click();
  await page.getByRole("button", { name: "Check a new request against earlier edits" }).click();
  const prior = page.locator(".prior-edits");
  await prior.getByRole("heading", { name: "Test dispatch-guide on this request" }).waitFor();
  await prior.getByText(/2 of 2 distinct earlier inputs/).waitFor();
  await prior.getByText("Compare tested edits").click();
  assert.equal(await prior.locator("tbody tr").count(), 4);
  const oldReplay = await prior.getByRole("link", { name: "Inspect an earlier replay" }).getAttribute("href");
  const original = await browser.newPage();
  await original.goto(new URL(oldReplay, base).href);
  await original.locator('[data-context-id="dispatch-guide"]:focus').waitFor();
  await original.close();
  await page.getByRole("button", { name: "Compare with model scores for this request" }).click();
  const measuredEdit = page.locator(".panel").filter({ has: page.getByRole("heading", {
    name: "Editing routing-note switched the preferred alternative", exact: true }) });
  await measuredEdit.getByText("UNKNOWN favored", { exact: true }).waitFor();
  await measuredEdit.getByText("KITE favored", { exact: true }).waitFor();
  await prior.getByRole("heading", { name: "Earlier replays suggested testing dispatch-guide" }).waitFor();
  if (process.env.PENDING_SCREENSHOT) await measuredEdit.screenshot({ path: process.env.PENDING_SCREENSHOT });
  await page.getByRole("button", { name: "Try a measured tool choice" }).click();
  await page.getByText("price favored", { exact: true }).first().waitFor();
  await page.getByRole("button", { name: "Try a policy conflict" }).click();
  await page.getByRole("heading", { name: "Resolve the competing rules" }).waitFor();
  assert.equal(new URL(page.url()).searchParams.get("view"), "preview");

  const mobile = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, deviceScaleFactor: 1 });
  mobile.on("pageerror", (error) => errors.push(error.message));
  await mobile.goto(base);
  await factor(mobile, "Conflicting rules").waitFor();
  assert.ok(await mobile.locator("h1").evaluate(node => node.clientHeight < 1.5 * parseFloat(getComputedStyle(node).lineHeight)));
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  if (process.env.MOBILE_SCREENSHOT) await mobile.screenshot({ path: process.env.MOBILE_SCREENSHOT });
  await factor(mobile, "Results varied with source order").click();
  await mobile.getByText(/in 2 of 4 matched comparisons/).waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  await mobile.goto(base);
  await factor(mobile, "Results varied across wordings").click();
  await mobile.locator(".card").filter({ hasText: "Replacement comparison" }).getByRole("link").click();
  await mobile.locator(".panel").first().locator("tr.different").waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  await mobile.getByRole("link", { name: "Recommendations", exact: true }).click();
  await factor(mobile, "Conflicting rules").click();
  await conflictCard(mobile).getByRole("link", { name: "Review recommendation" }).click();
  await mobile.getByRole("heading", { name: "Recommendation", exact: true }).waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  await mobile.goto(base + "/?view=preview&scenario=warranty");
  await mobile.getByText("59.7% probability outside this pair", { exact: true }).waitFor();
  await mobile.getByRole("heading", { name: "Check which other answers are plausible" }).waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  for (const device of [page, mobile]) {
    await device.goto(base + "/?view=preview&scenario=prefill");
    await device.getByRole("heading", { name: "Test the edit to request-type" }).waitFor();
    await device.getByText("Request type: price.", { exact: true }).waitFor();
    await device.getByText("Compare source edits", { exact: true }).click();
    assert.deepEqual(await device.locator("#content tbody tr td:last-child").allTextContents(), ["0.000169 bits", "0.00000469 bits"]);
    assert.equal(await device.evaluate(() => document.body.scrollWidth <= innerWidth), true);
    if (device === page && process.env.PREFILL_SCREENSHOT) {
      await device.locator(".panel").filter({hasText:"Before the first output token"}).screenshot({path:process.env.PREFILL_SCREENSHOT});
    }
    await device.getByRole("link", {name:"Compare the measured tool proposals"}).click();
    await device.getByRole("link", {name:"Check the same edits before the first token"}).waitFor();
  }
  if (process.env.PREFILL_COHORT_URL) {
    for (const device of [page, mobile]) {
      const study = process.env.PREFILL_COHORT_URL;
      await device.goto(study + "/?org=retail-north");
      await factor(device, "First-token sensitivity").click();
      const changed = device.locator(".card").filter({hasText:"2 changed, 0 unchanged"});
      await changed.getByRole("link", {name:"Review recommendation"}).click();
      await device.getByText("request-type ranked first on 2 of 2 distinct inputs.", {exact:true}).waitFor();
      await device.getByRole("heading", {name:"Repeat the leading edit on a fresh request"}).waitFor();
      await device.getByText("Compare generated proposals", {exact:true}).first().click();
      await device.getByRole("heading", {name:"request-type · proposal changed"}).first().waitFor();
      if (device === page && process.env.COHORT_SCREENSHOT) await device.screenshot({path:process.env.COHORT_SCREENSHOT});
      await device.goto(study + "/?org=retail-south");
      await factor(device, "First-token sensitivity").click();
      await device.locator(".card").filter({hasText:"0 changed, 1 unchanged"}).getByRole("link").click();
      await device.getByRole("heading", {name:"Check a later output token"}).waitFor();
      await device.getByText("Original request · 1 available tool · stock", {exact:true}).waitFor();
      await device.getByText("Compare generated proposals", {exact:true}).first().click();
      await device.getByRole("heading", {name:"request-type · same proposal"}).waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
  }
  assert.deepEqual(errors, []);
  console.log("Verified recommendations, before-inference preview, measured choices and tokens, legacy links, and desktop and mobile layouts.");
} finally {
  await browser.close();
}
