// Run with PLAYWRIGHT_MODULE set to the installed playwright-core module and
// CHROME_BINARY set to a Chrome or Chromium executable.
import assert from "node:assert/strict";
import process from "node:process";
import {readFile} from "node:fs/promises";

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
  await page.getByText("of 26 captured conversations", {exact:true}).waitFor();
  assert.equal(await page.locator(".impact-number").innerText(), "25");
  assert.deepEqual(await page.locator(".impact-outcome strong").allTextContents(), ["17", "2", "6"]);
  assert.ok(await page.locator(".brand img").evaluate(img => img.complete && img.naturalWidth > 0));
  assert.equal(await page.locator("#navigation a").count(), 2);
  await factor(page, "Choice flipped").waitFor();
  await page.locator(".featured h3").filter({hasText:"The proposed tool changed in 2 of 4 completed runs"}).waitFor();
  assert.match(await page.locator(".featured p").innerText(), /edit to request-type/);
  if (process.env.FINDINGS_SCREENSHOT) await page.screenshot({ path: process.env.FINDINGS_SCREENSHOT });
  const proposals = page.locator(".proposal-coverage");
  await proposals.locator(":scope > summary").click();
  assert.deepEqual(await proposals.locator(":scope > dl dd").allTextContents(), ["17", "0", "2", "6"]);
  if (process.env.PROPOSAL_COVERAGE_SCREENSHOT) await page.locator(".impact").screenshot({path:process.env.PROPOSAL_COVERAGE_SCREENSHOT});
  const coverage = page.locator(".replay-coverage");
  await coverage.getByText("Scored replays in 7 of 25 conversations").click();
  assert.deepEqual(await coverage.locator("dd").allTextContents(), ["0", "0", "7", "18"]);
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
  await proposals.locator(":scope > summary").click();
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
  const arms = page.locator(".panel").first().locator(".comparison").first().locator(":scope > div");
  assert.equal(await arms.count(), 4);
  assert.match(await arms.nth(1).innerText(), /1 \/ 1 matched/);
  assert.match(await arms.nth(3).innerText(), /0 \/ 1 matched/);
  assert.deepEqual(await arms.locator("code").allTextContents(), ['"LARK"', '"LARK"', '"LARK"', '"UNKNOWN"']);
  await page.goto(base + "/?recommendation=602208c5e9c068fab608137a9c92699923591b1f349ab8504b57e65084f85808");
  const recurrence = page.locator(".panel").filter({has:page.getByRole("heading",{name:"Where did this pattern recur?"})});
  await recurrence.getByText("2 identified conversations · 2 distinct captured inputs · 2 reports").waitFor();
  if (process.env.JOINT_PROPOSALS_SCREENSHOT) {
    const first = await page.locator(".panel").first().boundingBox();
    const last = await recurrence.boundingBox();
    await page.screenshot({path:process.env.JOINT_PROPOSALS_SCREENSHOT, fullPage:true,
      clip:{x:first.x, y:first.y, width:first.width, height:last.y + last.height - first.y}});
  }
  await recurrence.getByRole("link", {name:"Only the joint edit changed the proposal"}).click();
  const fourArmReports = page.locator("#content .list a");
  await fourArmReports.first().waitFor();
  assert.equal(await fourArmReports.count(), 2);
  await fourArmReports.last().click();
  await page.getByText(/Joint interaction: -100 percentage points/).waitFor();
  assert.equal(new URL(page.url()).searchParams.get("decision"), "bay");
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
  await page.locator("#selected-call:focus").waitFor();
  assert.equal(new URL(page.url()).searchParams.get("focus_call"),selected.searchParams.get("call"));
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
  await page.locator(".factor-row").filter({hasText:"Choice flipped"}).filter({hasText:"request-type"}).click();
  const measured = page.locator(".card").filter({ hasText: "switched the preferred alternative" });
  await measured.getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText("stock favored", { exact: true }).first().waitFor();
  await page.getByText("price favored", { exact: true }).first().waitFor();
  await page.getByRole("region",{name:"Source edits at a supplied prefix"}).getByText("Scoring conditions").click();
  await page.getByText(/store-name.*-0\.089/).first().waitFor();
  await page.goto(base);
  await page.locator(".factor-row").filter({hasText:"refund-policy"}).filter({hasText:"Token score shifted"}).click();
  await page.locator(".card").filter({ hasText: "Token measurements" }).getByRole("link", { name: "Review recommendation" }).click();
  const tokenRecommendation = page.url();
  const pathRegion = page.getByRole("region", {name:"Source edits along the answer"});
  await pathRegion.getByRole("button", {name:/Token 6 ·/}).click();
  assert.equal(await pathRegion.locator(".edit-result strong").first().innerText(), "returns-policy");
  await pathRegion.getByText("Inspect edit to refund-policy", {exact:true}).click();
  await pathRegion.getByRole("button", {name:/largest shift · token 14/}).click();
  assert.equal(await pathRegion.locator(".edit-result strong").first().innerText(), "refund-policy");
  assert.equal(new URL(page.url()).hash, "#output-token-14");
  await pathRegion.getByText("2 captured inputs without a source-edit measurement").click();
  await pathRegion.getByText("format-instruction, question", {exact:true}).waitFor();
  await pathRegion.getByText("Inspect edit to store-name", {exact:true}).click();
  await pathRegion.getByText("Generated text stayed the same after this edit").waitFor();
  await pathRegion.getByText("Inspect edit to store-name", {exact:true}).click();
  await pathRegion.getByText("Inspect edit to refund-policy", {exact:true}).click();
  if (process.env.TOKEN_PATH_SCREENSHOT) await pathRegion.screenshot({path:process.env.TOKEN_PATH_SCREENSHOT});
  const blockHref = await pathRegion.getByRole("link", {name:/decoder-block measurements/}).getAttribute("href");
  const blockTab = await browser.newPage();
  await blockTab.goto(new URL(blockHref, base).href);
  await blockTab.locator(".activation-output-detail h4").filter({hasText:"Output token 14 of 16"}).waitFor();
  await blockTab.close();
  await page.getByText("Peak score change and block swap", {exact:true}).click();
  await page.getByText(/largest tested decoder-block swap/).waitFor();
  await page.getByRole("link", { name: "Before inference" }).click();
  await page.getByRole("button", { name: "Check a new request against earlier edits" }).click();
  const prior = page.locator(".prior-edits");
  await prior.getByRole("heading", { name: "Test dispatch-guide on this request" }).waitFor();
  await prior.getByText(/changed the generated proposal on 2 of 2 distinct earlier inputs/).waitFor();
  const downloadPromise = page.waitForEvent("download");
  await prior.getByRole("button", {name:"Download edit for this request"}).click();
  const download = await downloadPromise;
  const [prepared] = JSON.parse(await readFile(await download.path(), "utf8"));
  assert.equal(prepared.pointer, "/messages/2/content");
  assert.deepEqual(prepared.text_range, {start_byte:16, end_byte:46});
  await prior.getByText("Compare tested edits").click();
  assert.equal(await prior.locator("tbody tr").count(), 4);
  const oldReplay = await prior.getByRole("link", { name: "Compare an earlier answer or tool call" }).getAttribute("href");
  const original = await browser.newPage();
  await original.goto(new URL(oldReplay, base).href);
  await original.locator(".response-pair[open] summary:focus").waitFor();
  const [before, after] = await original.locator(".response-pair code").allTextContents();
  assert.ok(before && after && before !== after);
  assert.equal(new URL(original.url()).searchParams.get("context"), "dispatch-guide");
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
  const demoFixture = name => readFile(new URL("../examples/redundant-routing/" + name, import.meta.url), "utf8").then(JSON.parse);
  const {request} = await demoFixture("candidate-checks/ash-routing-note.json");
  const {shadow} = await demoFixture("ash-history-check.json");
  delete request.attributes["nexusweave.organization.id"];
  await page.locator('input[type="file"]').setInputFiles({name:"proposal.json", mimeType:"application/json", buffer:Buffer.from(JSON.stringify({request,shadow}))});
  await page.getByRole("heading", {name:"Test another source on this request"}).waitFor();
  await page.getByText("Compare the matching proposals", {exact:true}).click();
  assert.deepEqual(await page.locator(".response-pair code").allTextContents(), ["UNKNOWN","UNKNOWN"]);

  const mobile = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, deviceScaleFactor: 1 });
  mobile.on("pageerror", (error) => errors.push(error.message));
  const jacketThread = JSON.stringify(["fixture","Northland Retail","Customer 042","Return a jacket"]);
  for (const device of [page, mobile]) {
    await device.goto(base + "/?" + new URLSearchParams({view:"threads",thread:jacketThread}));
    const turns = device.locator(".dialogue-call");
    await turns.first().waitFor();
    assert.equal(await turns.count(),2);
    assert.equal(await device.getByRole("link",{name:"Open call",exact:true}).count(),3);
    assert.equal(await device.getByText(/My order is J-104/).count(),1);
    assert.deepEqual(await turns.last().locator(".dialogue-output strong").allTextContents(),["Proposed tool · look_up_order","Assistant"]);
    await turns.last().getByText(/up to 60 days/).waitFor();
    if (device === page && process.env.CONVERSATION_SCREENSHOT) await device.screenshot({path:process.env.CONVERSATION_SCREENSHOT,fullPage:true});
    await device.getByRole("link",{name:/Declared instructions disagree.*Returns policy and Support FAQ/}).click();
    await device.getByText(/original request body was not captured/).waitFor();
    await device.getByRole("link",{name:"Supply request to test Support FAQ"}).click();
    const jacketPlanner = device.getByRole("dialog",{name:"Prepare controlled replay"});
    await jacketPlanner.getByLabel("Original inference request (JSON)").waitFor();
    assert.equal(await jacketPlanner.getByRole("button",{name:"Download replay case"}).isDisabled(),true);
    await device.goBack();
    await device.getByRole("link",{name:"Back to conversation",exact:true}).click();
    await device.locator("#selected-call:focus").getByText(/up to 60 days/).waitFor();
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await device.getByRole("link",{name:"Browse traces in this conversation"}).click();
    await device.locator("a").filter({hasText:"jacket-return-trace"}).waitFor();
  }
  for (const device of [page, mobile]) {
    await device.goto(base);
    await device.getByRole("link", {name:/3 flagged conversations · 2 with a completed single-source comparison/}).click();
    const screenGroup = device.url();
    const calls = device.locator("#content .feed > .feed > section.panel");
    await calls.first().getByRole("heading").waitFor();
    assert.deepEqual(await device.locator(".screen-outcomes tbody tr").allTextContents(), ["journal-policy02","journal-faq20"]);
    assert.deepEqual(await calls.locator("h2").allTextContents(), ["journal-return-followup", "journal-policy-support", "journal-return-support"]);
    await calls.first().getByText(/No completed single-source replay/).waitFor();
    const testedPolicy = calls.nth(1);
    await testedPolicy.getByRole("link", {name:"journal-policy edit preserved the proposal"}).waitFor();
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (device === page && process.env.SCREEN_PATTERN_SCREENSHOT) await device.screenshot({path:process.env.SCREEN_PATTERN_SCREENSHOT});
    await testedPolicy.getByRole("link", {name:"journal-faq edit changed the proposal"}).click();
    await device.locator(".response-pair[open] summary:focus").waitFor();
    assert.match(await device.locator(".response-pair[open]").innerText(), /60/);
    await device.getByRole("link", {name:"Back to flagged pair", exact:true}).click();
    assert.equal(device.url(), screenGroup);
    await device.goto(screenGroup + "&org=journal-south");
    await calls.first().getByText(/No completed single-source replay/).waitFor();
    assert.equal(await calls.count(), 1);
    await device.getByRole("link", {name:"Back to flagged pairs"}).click();
    await device.getByRole("link", {name:/1 flagged conversation · 0 with a completed single-source comparison/}).waitFor();
    await device.goto(base);
    await device.getByRole("link", {name:/1 conversation without a finding.*1 with a flagged source pair/}).click();
    const screen = device.locator(".panel").filter({hasText:"Possible conflict between two sources"});
    assert.match(await screen.innerText(), /30 days/);
    assert.match(await screen.innerText(), /60 days/);
    await screen.getByText("Classifier and coverage").click();
    await screen.getByText(/first to second 0.922.*second to first 0.991/).waitFor();
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (device === page && process.env.CAPTURE_SCREENSHOT) await screen.screenshot({path:process.env.CAPTURE_SCREENSHOT});
    await screen.getByRole("link", {name:"Inspect the pair and its replay options"}).click();
    const pair = device.locator("details.finding[open][data-screen-pair]");
    await pair.getByRole("button", {name:"Prepare replay for this pair"}).click();
    await device.getByRole("dialog").getByText("Prepare controlled replay", {exact:true}).waitFor();
    await device.goto(base + "/?view=preview&scenario=screen");
    await device.getByRole("heading", {name:"Possible conflict between two sources"}).waitFor();
    await device.goto(base + "/?view=preview&scenario=checked-history");
    const result = device.locator(".prior-edits");
    await result.getByRole("heading", {name:"The proposal stayed the same after editing dispatch-guide"}).waitFor();
    assert.deepEqual(await result.locator(".response-pair code").allTextContents(), ["UNKNOWN", "UNKNOWN"]);
    assert.match(await result.innerText(), /0\.252 bits/);
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (device === page && process.env.HISTORY_CHECK_SCREENSHOT) await result.screenshot({path:process.env.HISTORY_CHECK_SCREENSHOT});
    await device.goto(base + "/?org=routing-sampling#proposal-coverage");
    await device.getByRole("link", {name:"Changed and unchanged proposals", exact:true}).click();
    await device.getByRole("heading", {name:"sampled-routing-28", exact:true}).waitFor();
    for (const [label, setup] of [["See a changed proposal", 1], ["See a matching proposal", 0]]) {
      await device.getByRole("link", {name:label}).click();
      await device.locator('[data-routing-setup="' + setup + '"]:focus').waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await device.getByRole("link", {name:"Back to compared conversations"}).click();
    }
    await device.getByRole("link", {name:"Back to coverage"}).click();
    await device.locator(".proposal-coverage[open] > summary:focus").waitFor();
    assert.equal(new URL(device.url()).searchParams.get("org"), "routing-sampling");
  }
  await mobile.goto(base + "/?recommendation=602208c5e9c068fab608137a9c92699923591b1f349ab8504b57e65084f85808");
  await mobile.locator(".joint-recurrence a").click();
  await mobile.locator("#content .list a").first().click();
  await mobile.getByRole("heading", {name:"Only the joint edit changed the proposal"}).waitFor();
  assert.equal(await mobile.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await mobile.goto(tokenRecommendation.split("#")[0] + "#output-token-6");
  await mobile.getByRole("button", {name:/Token 6 ·/, pressed:true}).waitFor();
  assert.equal(await mobile.locator(".edit-result strong").first().innerText(), "returns-policy");
  assert.equal(await mobile.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await mobile.goto(base + "/?level=1");
  await mobile.getByRole("link", {name:/local-model-product-route/}).click();
  const toolPath = mobile.getByRole("region", {name:"Source edits along the answer"});
  await toolPath.getByRole("button", {name:/Token 8 ·/, pressed:true}).waitFor();
  await toolPath.getByText("Inspect edit to product-id", {exact:true}).click();
  await toolPath.getByText("The proposed tool stayed the same and its arguments changed.").waitFor();
  await toolPath.getByText(/shift was -20.063 here and -0.059 at position 21/).waitFor();
  await toolPath.getByRole("button", {name:"Compare the same token at position 21"}).click();
  await toolPath.getByRole("button", {name:/Token 21 ·/, pressed:true}).waitFor();
  assert.equal(new URL(mobile.url()).hash, "#output-token-21");
  assert.equal(await toolPath.locator(".edit-result strong").first().innerText(), "product-id");
  await toolPath.getByRole("button", {name:"Earlier tokens"}).click();
  await toolPath.getByRole("button", {name:/Token 5 ·/}).click();
  await toolPath.getByText("All tested edits left this token’s measured probability unchanged.").waitFor();
  assert.deepEqual(await toolPath.locator(".edit-delta").allTextContents(), ["0 log p","0 log p","0 log p"]);
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
    await device.goto(base + "/?org=journal-window-study");
    await device.getByRole("link", {name:/Every trial missed the target in 3 conversations/}).click();
    await device.getByRole("heading", {name:"Did any condition reach the target?"}).waitFor();
    assert.deepEqual(await device.locator(".target-counts dd").allTextContents(), ["3","1","0","0"]);
    if (device === page && process.env.TARGET_GROUP_SCREENSHOT) await device.locator("#content").screenshot({path:process.env.TARGET_GROUP_SCREENSHOT});
    await device.getByRole("link",{name:/Compare the opening token.*2 conversations/}).click();
    await device.getByRole("heading",{name:"Compare the opening token",exact:true}).waitFor();
    assert.equal(await device.getByRole("link",{name:/journal-window-first/}).count(), 0);
    await device.getByRole("link", {name:/journal-window-unopened/}).click();
    await device.getByText(/For an unopened journal, state the latest day/).waitFor();
    assert.deepEqual(await device.locator(".target-tokens[open] tbody code").allTextContents(), Array(4).fill('"1"'));
    assert.deepEqual(await device.locator(".target-check > table tbody code").allTextContents(), ["100","120","100","120"]);
    assert.equal(new URL(device.url()).searchParams.get("org"), "journal-window-study");
    const download = device.waitForEvent("download");
    await device.getByRole("button", {name:"Download recorded trials"}).click();
    const report = JSON.parse(await readFile(await (await download).path(), "utf8"));
    assert.equal(report.pair.trials[0].choice.outcome, "120");
    await device.getByRole("link", {name:"Back to target results"}).click();
    assert.equal(new URL(device.url()).searchParams.get("miss_signal"), "opening");
    await device.getByRole("link",{name:"Show all experiments"}).click();
    await device.getByRole("link",{name:/Inspect text after the target.*1 conversation/}).click();
    await device.getByRole("link",{name:/journal-window-fresh/}).click();
    await device.getByRole("heading",{name:"Inspect the text after the target answer"}).waitFor();
    assert.equal(await device.locator(".target-check > table tbody code").last().textContent(), "45 days");
    await device.getByRole("link", {name:"Back to target results"}).click();
    await device.getByRole("link",{name:"Show all experiments"}).click();
    await device.getByRole("link", {name:/journal-window-first/}).click();
    await device.getByRole("heading", {name:"Retest the edit to journal-faq on another request"}).waitFor();
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await device.goto(base + "/?view=preview&scenario=window-before");
    await device.getByRole("heading", {name:'Opening token "4" reached the top score in 2 of 3 edited conditions'}).waitFor();
    const candidateChoice = device.getByRole("combobox", {name:"Scored continuation"});
    assert.deepEqual(await candidateChoice.locator("option").allTextContents(), ["45","30"]);
    await candidateChoice.selectOption("1");
    assert.match(await device.locator(".target-tokens select option").first().textContent(), /"3"/);
    assert.equal(await device.locator(".target-check > table").count(), 0);
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await device.getByRole("link", {name:"Compare with generated answers"}).click();
    const target = device.locator(".target-check");
    await target.getByRole("heading", {name:"Retest the edit to journal-faq on another request"}).waitFor();
    assert.deepEqual(await target.locator("tbody strong").allTextContents(), ["Missed","Missed","Met","Met"]);
    await device.getByRole("link", {name:"Unopened journal"}).click();
    await target.getByRole("heading", {name:"Try a different edit for this target"}).waitFor();
    assert.deepEqual(await target.locator(":scope > table tbody code").allTextContents(), ["100","120","100","120"]);
    assert.deepEqual(await target.locator("tbody strong").allTextContents(), Array(4).fill("Missed"));
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (device === page && process.env.TARGET_SCREENSHOT) await target.screenshot({path:process.env.TARGET_SCREENSHOT});
    await device.getByRole("link", {name:"Fresh question",exact:true}).click();
    await target.getByRole("heading", {name:"Inspect the text after the target answer"}).waitFor();
    assert.deepEqual(await target.locator(":scope > table tbody code").allTextContents(), ["60 days","60 days","30 days","45 days"]);
    const tokens = target.locator(".target-tokens");
    await tokens.locator("summary").click();
    assert.deepEqual(await tokens.locator("tbody code").allTextContents(), ['"6"','"6"','"3"','"4"']);
    await tokens.getByRole("combobox", {name:"Target token position"}).selectOption("1");
    assert.deepEqual(await tokens.locator("tbody code").allTextContents(), Array(4).fill('"5"'));
    await tokens.getByText("Scores at this position assume the preceding 1 target token.").waitFor();
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (device === page && process.env.TARGET_TOKENS_SCREENSHOT) {
      await tokens.getByRole("combobox").selectOption("0");
      await target.screenshot({path:process.env.TARGET_TOKENS_SCREENSHOT});
    }
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
    await device.getByRole("link", {name:"Test instruction text and available tools"}).click();
    await device.getByRole("region", {name:"Four routing setups"}).waitFor();
    assert.equal(await device.locator(".routing-comparison tbody tr").count(), 4);
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  for (const device of [page, mobile]) {
    await device.goto(base + "/?org=routing-sampling");
    await factor(device, "Repeats disagreed").click();
    await device.getByRole("heading", {name:"Repeat trials on one input"}).waitFor();
    const votes = device.getByRole("region", {name:"Repeat counts by setup"});
    assert.deepEqual(await votes.locator("tbody tr td:nth-child(2)").allTextContents(), ["0 / 4", "3 / 4", "0 / 4", "4 / 4"]);
    assert.deepEqual(await votes.locator("tbody tr td:nth-child(3)").allTextContents(), ["0 / 4", "2 / 4", "0 / 4", "4 / 4"]);
    const toolWitness = device.locator("details").filter({has: device.locator("summary", {hasText:"Compare tool selection"})}).first();
    await toolWitness.locator("summary").click();
    assert.match(await toolWitness.innerText(), /2 changed · 2 unchanged · 0 incomplete/);
    if (device === page && process.env.REPEAT_SCREENSHOT) await device.screenshot({path:process.env.REPEAT_SCREENSHOT});
    const witnessUrl = device.url();
    for (const [label, changed] of [["Unchanged in this run", false], ["Changed in this run", true]]) {
      await toolWitness.getByRole("link", {name:label, exact:true}).click();
      const selected = device.locator('[data-routing-setup="1"]:focus');
      await selected.waitFor();
      const outputs = await selected.locator("code").allTextContents();
      assert.equal(outputs[0].split(" · ")[0] !== outputs[1].split(" · ")[0], changed);
      assert.equal(new URL(device.url()).searchParams.get("org"), "routing-sampling");
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await device.goto(witnessUrl);
      await toolWitness.locator("summary").click();
    }
    await device.goto(base);
    await device.getByRole("link", {name:/Generated proposal changed/}).click();
    await device.getByRole("heading", {name:"Edits that changed a proposal"}).waitFor();
    const faq = device.locator(".cohort-row").filter({hasText:"journal-faq (1)"}).filter({hasNotText:"journal-policy"});
    assert.match(await faq.innerText(), /2 of 17 conversations/);
    if (device === page && process.env.OUTCOME_SOURCES_SCREENSHOT) await device.screenshot({path:process.env.OUTCOME_SOURCES_SCREENSHOT});
    await faq.click();
    await device.getByRole("link", {name:/journal-policy-support/}).click();
    await device.getByText("Compare the changed proposals", {exact:true}).click();
    const [original, edited] = await device.locator(".response-pair code").allTextContents();
    assert.ok(original && edited && original !== edited);
    assert.equal(new URL(device.url()).searchParams.get("replay"), "qwen-screened-journal-policy");
    await device.getByRole("link",{name:"Back to conversations"}).click();
    await device.getByRole("link", {name:/journal-policy-support/}).waitFor();
    assert.ok(new URL(device.url()).searchParams.get("cohort_factor"));
    assert.equal(await device.locator("#content .list li").count(), 2);
    await device.goto(base + "/?level=1&by=edit&org=retail-south");
    const instructionEdit = device.locator(".cohort-row").filter({hasText:"request-type"});
    assert.match(await instructionEdit.innerText(), /Edited individually/);
    assert.equal(await device.locator(".cohort-row").filter({hasText:"store-name"}).count(), 0);
    await instructionEdit.click();
    await device.getByRole("link", {name:/routing-holdout-73.*Proposal changed with the same tool selection/}).waitFor();
    await device.getByRole("link", {name:/routing-holdout-28/}).click();
    await device.locator('tr.changed[data-routing-setup="1"]:focus').waitFor();
    await device.goto(base + "/?level=1&by=edit");
    await device.locator(".cohort-row").filter({hasText:"returns-policy"}).click();
    await device.locator("#content .list a").first().click();
    await device.locator("details[open] summary:focus").filter({hasText:"Inspect edit to returns-policy"}).waitFor();
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await device.goto(base + "/?org=retail-south");
    await device.getByRole("link", {name:/Model score shifted/}).click();
    await device.getByRole("link", {name:"By conversation"}).click();
    assert.equal(new URL(device.url()).searchParams.get("org"), "retail-south");
    await device.getByRole("link", {name:/routing-holdout-64/}).waitFor();
    assert.equal(await device.getByRole("link", {name:/Paired proposals stayed the same/}).count(), 2);
    await device.getByRole("link", {name:/routing-holdout-64/}).click();
    await device.getByRole("region", {name:"Four routing setups"}).waitFor();
    await device.getByRole("link", {name:"Back to conversations"}).click();
    await device.getByRole("link", {name:/routing-holdout-91/}).waitFor();
    assert.equal(new URL(device.url()).searchParams.get("org"), "retail-south");
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await device.getByRole("link", {name:"All source factors"}).click();
    await factor(device, "Routing tests on 4 inputs").click();
    await device.locator(".card").filter({hasText:"4 of 4 distinct inputs"}).getByRole("link").click();
    const region = device.getByRole("region", {name:"Routing results across inputs"});
    await region.waitFor();
    assert.deepEqual(await region.locator("tbody th").allTextContents(), ["2", "1", "1"]);
    if (device === page && process.env.REPLICATION_SCREENSHOT) await device.screenshot({path:process.env.REPLICATION_SCREENSHOT});
    await region.getByRole("row").filter({hasText:"Alternate instruction with stock + price"}).getByRole("link").click();
    await device.getByText("Changed · same tool selection", {exact:true}).waitFor();
    await device.getByText("Other user messages", {exact:true}).click();
    await device.getByText("Please check the request for this item and select the appropriate tool.", {exact:true}).waitFor();
    await device.getByRole("link", {name:"All routing results in this scope"}).click();
    await region.waitFor();
    assert.equal(new URL(device.url()).searchParams.get("org"), "retail-south");
    assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
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
      await factor(device, "Instruction and tool test").click();
      await device.locator(".card").filter({hasText:"0 changed, 1 unchanged"}).getByRole("link").click();
      await device.getByRole("heading", {name:"Retest the tool inventory on another request"}).waitFor();
      const matrix = device.getByRole("region", {name:"Four routing setups"});
      assert.deepEqual(await matrix.locator("tbody tr td:last-child").allTextContents(), ["Unchanged", "Changed", "Unchanged", "Changed"]);
      await device.getByText("Compare instruction text", {exact:true}).click();
      await device.getByText(/Always call stock using/).first().waitFor();
      if (device === page && process.env.ROUTING_SCREENSHOT) await device.screenshot({path:process.env.ROUTING_SCREENSHOT});
      await device.getByText("Original request · 1 available tool · stock", {exact:true}).waitFor();
      await device.getByText("Compare generated proposals", {exact:true}).first().click();
      await device.getByRole("heading", {name:"request-type · same proposal"}).waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
  }
  if (process.env.PUBLIC_QA_URL) {
    for (const device of [page,mobile]) {
      await device.goto(process.env.PUBLIC_QA_URL + "/?org=public-qa&workflow=public-qa-holdout-order-0");
      await device.getByRole("link",{name:/Inspect 2 calls where a lower-ranked edit changed the proposal/}).click();
      const missed = device.getByRole("link",{name:/QA (1100|1200)/});
      await missed.first().waitFor();
      assert.equal(await missed.count(), 2);
      await device.getByRole("link",{name:/QA 1100/}).click();
      const witness = device.getByRole("region",{name:"Measured source contrast"});
      await witness.getByRole("heading",{name:"Two edits on the same request"}).waitFor();
      const rows = witness.locator("tbody tr");
      assert.match(await rows.first().innerText(), /North America[\s\S]*Rank 1 of 10[\s\S]*Vespucci[\s\S]*Held/);
      assert.match(await rows.last().innerText(), /Amerigo Vespucci[\s\S]*Rank 5 of 10[\s\S]*North America[\s\S]*Changed/);
      if (device === page && process.env.LOWER_EDIT_SCREENSHOT) await witness.screenshot({path:process.env.LOWER_EDIT_SCREENSHOT});
      await rows.last().getByText("Source edit").click();
      await rows.last().getByText("[passage omitted]",{exact:true}).waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await device.getByRole("link",{name:"Back to edit rankings"}).click();
      assert.equal(new URL(device.url()).searchParams.get("review"), "lower_changed");
      await device.goto(process.env.PUBLIC_QA_URL + "/?org=public-qa&workflow=public-qa-holdout-order-0&ranking=all");
      await device.getByRole("heading",{name:"17 of 20 calls had a scored pair"}).waitFor();
      await device.getByText("The ranked edit changed the proposal in 14 of 17 scored calls. The first listed edit changed it in 11.").waitFor();
      if (device === page && process.env.RANKING_SCREENSHOT) await device.locator(".panel").filter({has:device.getByRole("heading",{name:"17 of 20 calls had a scored pair"})}).screenshot({path:process.env.RANKING_SCREENSHOT});
      await device.getByRole("link",{name:/Only the first listed edit changed it.*1 call/}).click();
      await device.getByRole("link",{name:/QA 1200/}).click();
      await device.getByText("The completed proposal stayed the same after the top edit.").waitFor();
      await device.getByText("Compare source edits",{exact:true}).click();
      const table = device.locator(".panel.replacement table");
      assert.equal(await table.locator("tbody tr").first().locator("td").last().innerText(), "Held");
      assert.equal(await table.locator("tr").filter({hasText:"First listed edit"}).locator("td").last().innerText(),"Changed");
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await device.getByRole("link",{name:"Back to edit rankings"}).click();
      assert.equal(new URL(device.url()).searchParams.get("workflow"),"public-qa-holdout-order-0");
      await device.getByRole("link",{name:/Missing a completed pair.*3 calls/}).click();
      await device.getByRole("link",{name:/QA 1000/}).click();
      await device.getByText("The effect of the top edit on the completed proposal is unmeasured.").waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
  }
  if (process.env.PREFIX_STUDY_URL) {
    for (const device of [page,mobile]) {
      await device.goto(new URL("?view=preview&scenario=prefix-before",process.env.PREFIX_STUDY_URL).href);
      const card = device.getByRole("region",{name:"Source edits at a supplied prefix"});
      await card.getByRole("heading",{name:"Editing journal-policy switched the preferred continuation"}).waitFor();
      assert.equal(await card.getByRole("meter").count(),9);
      assert.equal(await card.getByRole("heading",{name:"What did the runner generate?"}).count(),0);
      assert.match(await card.innerText(),/66.0?%|66%/);
      await card.getByRole("link",{name:"Compare with freely generated answers"}).click();
      await card.getByRole("heading",{name:"What did the runner generate?"}).waitFor();
      await card.getByText("Return window: 60 days",{exact:true}).waitFor();
      assert.equal(await card.getByText("Return window: 30 days",{exact:true}).count(),2);
      if (device === page && process.env.PREFIX_SCREENSHOT) await card.screenshot({path:process.env.PREFIX_SCREENSHOT});
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      await card.getByRole("link",{name:"See the scores before generation"}).click();
      await card.getByRole("link",{name:"Compare with freely generated answers"}).waitFor();
    }
  }
  if (process.env.OPENING_STUDY_URL) {
    for (const device of [page, mobile]) {
      await device.goto(process.env.OPENING_STUDY_URL);
      await device.getByRole("link", {name:/Review 3 conversations where the top opening token held/}).click();
      await device.getByRole("heading",{name:"When the opening token held"}).waitFor();
      assert.match(await device.locator("#summary").innerText(), /5 of 8 scored conversations/);
      const policy = device.getByRole("row").filter({has:device.getByRole("link",{name:"journal-policy",exact:true})});
      assert.deepEqual(await policy.getByRole("cell").allTextContents(), ["8","5","2"]);
      if (device === page && process.env.OPENING_STUDY_SCREENSHOT) {
        await device.screenshot({path:process.env.OPENING_STUDY_SCREENSHOT});
      }
      await policy.getByRole("link").click();
      await device.getByRole("link",{name:/Using the stated policy/}).click();
      await device.getByRole("heading",{name:"The choice changed at token 8"}).waitFor();
      assert.match(await device.locator(".opening-comparison").innerText(), /60 days/);
      assert.match(await device.locator(".opening-comparison").innerText(), /30 days/);
      await device.getByRole("rowheader",{name:"Edit journal-policy"}).waitFor();
      const back = device.getByRole("link",{name:"Back to source edits"});
      assert.ok(new URL(await back.getAttribute("href"),device.url()).searchParams.get("slot"));
      const conversation = device.getByRole("link",{name:"See this call in its conversation"});
      assert.ok(new URL(await conversation.getAttribute("href"),device.url()).searchParams.get("focus_call"));
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),true);
      await back.click();
      await device.getByRole("heading",{name:"Changes after editing journal-policy"}).waitFor();
      assert.equal(new URL(device.url()).searchParams.get("org"),"journal-opening-study");
    }
  }
  if (process.env.OPENING_URL) {
    for (const device of [page, mobile]) {
      await device.goto(new URL("?view=preview&scenario=opening-before", process.env.OPENING_URL).href);
      const panel = device.locator(".opening-comparison");
      await panel.getByRole("heading",{name:"The same opening token led under every tested edit"}).waitFor();
      assert.equal(await panel.getByRole("cell",{name:"Unmeasured",exact:true}).count(),2);
      await device.getByRole("link",{name:"Check the complete proposals"}).click();
      await panel.getByRole("heading",{name:/Editing journal-policy changed the proposal/}).waitFor();
      await panel.getByText("Compare generated proposals",{exact:true}).click();
      const policy = panel.locator('[data-context="request-source:journal-policy"]');
      await policy.getByText("A journal may be returned within 60 days.",{exact:true}).waitFor();
      await policy.getByText("A journal may be returned within 30 days.",{exact:true}).waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      if (device === page && process.env.OPENING_SCREENSHOT) await panel.screenshot({path:process.env.OPENING_SCREENSHOT});
    }
  }
  if (process.env.CONFLICT_PATTERN_URL) {
    for (const device of [page, mobile]) {
      await device.goto(process.env.CONFLICT_PATTERN_URL);
      await device.getByRole("link",{name:/Compare questions where different source edits changed the proposal/}).click();
      const followup = device.locator(".panel").filter({has:device.getByRole("heading",{name:"Return follow-up"})});
      await followup.getByText("120",{exact:true}).waitFor();
      assert.equal(await device.locator(".question-edits").count(),3);
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      if (device === page && process.env.CONFLICT_PATTERN_SCREENSHOT) await device.screenshot({path:process.env.CONFLICT_PATTERN_SCREENSHOT});
      await followup.getByRole("link",{name:"Changed · inspect trials"}).click();
      await device.locator("details.response-pair[open]").getByText("120", {exact:true}).waitFor();
      assert.equal(new URL(device.url()).searchParams.get("replay"),"qwen-screened-journal-followup");
    }
  }
  if (process.env.CONFLICT_COVERAGE_URL) {
    for (const device of [page,mobile]) {
      await device.goto(process.env.CONFLICT_COVERAGE_URL);
      await device.getByRole("link",{name:/Compare the conflicting sources in 2 conversations/}).click();
      const categories = device.getByRole("navigation",{name:"Conflict test coverage"});
      await device.getByRole("link",{name:/Return follow-up.*Awaiting an individual comparison/}).waitFor();
      await categories.getByRole("link",{name:"1 · Partially tested"}).click();
      const example = device.getByRole("link",{name:/Policy question.*Awaiting an individual comparison · journal-faq/});
      await example.waitFor();
      if (device === page && process.env.CONFLICT_COVERAGE_SCREENSHOT) await device.screenshot({path:process.env.CONFLICT_COVERAGE_SCREENSHOT});
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      await example.click();
      await device.getByRole("link",{name:"Compare both edits in one replay"}).click();
      const planner = device.getByRole("dialog",{name:"Prepare controlled replay"});
      await planner.getByRole("button",{name:"Use captured request"}).click();
      const selected = planner.locator(".replay-candidate").filter({has:device.locator("input:checked")});
      await selected.first().waitFor();
      assert.equal(await selected.count(),2);
      await planner.getByLabel("Runner ID").fill("coverage-check");
      const download = device.waitForEvent("download");
      await planner.getByRole("button",{name:"Download replay case"}).click();
      const test = JSON.parse(await readFile(await (await download).path(),"utf8"));
      assert.deepEqual(test.interventions.map(edit=>edit.context_id).sort(),["input:0:0","input:1:0"]);
      await device.goBack();
      await device.getByRole("link",{name:"Back to conflict test coverage"}).click();
      await example.waitFor();
      await categories.getByRole("link",{name:"1 · Both sources compared on every call"}).click();
      await device.getByRole("link",{name:/Standard return.*Both sources compared in one report/}).waitFor();
    }
  }
  if (process.env.CONFLICT_TEST_URL) {
    for (const device of [page, mobile]) {
      await device.goto(process.env.CONFLICT_TEST_URL);
      const heading = device.getByRole("heading",{name:"What changed when each source was edited?"});
      await heading.waitFor();
      const panel = device.locator(".panel").filter({has:heading});
      await panel.getByText("Example · 60 → 45",{exact:true}).waitFor();
      await panel.getByText("Example · 60 → 60",{exact:true}).waitFor();
      if (device === page && process.env.CONFLICT_TEST_SCREENSHOT) await panel.screenshot({path:process.env.CONFLICT_TEST_SCREENSHOT});
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
      await panel.getByRole("link",{name:"2 replay reports with a changed proposal"}).click();
      await device.locator("details.response-pair[open]").getByText("45",{exact:true}).waitFor();
      await device.goBack();
      await panel.getByRole("link",{name:"Prepare edit to journal-faq"}).click();
      const planner = device.getByRole("dialog",{name:"Prepare controlled replay"});
      await planner.getByRole("button",{name:"Use captured request"}).click();
      const selected = planner.locator(".replay-candidate").filter({has:device.locator("input:checked")});
      await selected.waitFor();
      assert.equal(await selected.count(),1);
      assert.match(await selected.innerText(),/journal-faq/);
      await planner.getByLabel("Runner ID").fill("test-runner");
      const download = device.waitForEvent("download");
      await planner.getByRole("button",{name:"Download replay case"}).click();
      const result = JSON.parse(await readFile(await (await download).path(),"utf8"));
      assert.equal(result.interventions.length,1);
      assert.equal(result.interventions[0].pointer,"/messages/1/content");
      assert.equal(result.location.decision_id,"output:0:0");
    }
  }
  if (process.env.TOKEN_QA_URL) {
    for (const device of [page,mobile]) {
      await device.goto(process.env.TOKEN_QA_URL + "/?org=public-qa&workflow=public-qa-holdout-order-0-tokens");
      await device.getByRole("link",{name:/Inspect 2 calls where a lower-ranked edit changed the proposal/}).click();
      for (const [question, position] of [["QA 1100",1],["QA 1200",3]]) {
        await device.getByRole("link",{name:new RegExp(question)}).click();
        const readout = device.getByRole("region",{name:"Token probabilities"});
        await readout.getByRole("heading",{name:"The choice changed at token "+position}).waitFor();
        if (position === 3) {
          await readout.getByText(/All three answers started with "17"/).waitFor();
          assert.equal(await readout.getByRole("columnheader",{name:"End of answer"}).count(),1);
          assert.match(await readout.getByRole("row").last().innerText(),/94.6% · chosen/);
          if (device === page && process.env.TOKEN_CONTRAST_SCREENSHOT) await device.getByRole("region",{name:"Measured source contrast"}).screenshot({path:process.env.TOKEN_CONTRAST_SCREENSHOT});
          await device.getByRole("link",{name:"See this call in its conversation"}).click();
          const original = device.locator("#selected-call:focus");
          await original.getByRole("link",{name:/The choice changed at token 3.*UC Berkeley School of Public Health/}).click();
          await readout.getByRole("heading",{name:"The choice changed at token 3"}).waitFor();
        } else {
          assert.match(await readout.getByRole("row").last().innerText(),/19% · chosen/);
        }
        assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
        await device.getByRole("link",{name:"Back to edit rankings"}).click();
      }
    }
  }
  if (process.env.REFERENCE_QA_URL) {
    for (const device of [page, mobile]) {
      await device.goto(process.env.REFERENCE_QA_URL + "/?org=public-qa&workflow=public-qa-holdout-order-0-references");
      await device.getByRole("link", {name:/Inspect 3 calls where the ranked edit lost a reference match/}).click();
      const calls = device.locator("section").filter({has:device.getByRole("heading",{name:"Matches lost after the ranked edit",exact:true})});
      await calls.getByRole("link", {name:/QA 1150/}).waitFor();
      assert.deepEqual((await calls.getByRole("link",{name:/QA [0-9]+/}).allTextContents()).map(s => s.match(/QA (\d+)/)[1]).sort(),
        ["1150","1350","1650"]);
      await calls.getByRole("link", {name:/QA 1150/}).click();
      const detail = device.getByRole("region", {name:"Reference answer check"});
      await detail.getByRole("heading", {name:"Reference answer check"}).waitFor();
      const ranked = detail.getByRole("row").filter({has:device.getByRole("rowheader",{name:/Ranked/})});
      assert.match(await ranked.innerText(), /9 seasons of "Rugrats"/);
      assert.match(await ranked.locator("td").last().innerText(), /3\s*Missed/);
      const first = detail.getByRole("row").filter({has:device.getByRole("rowheader",{name:/First listed/})});
      assert.match(await first.locator("td").last().innerText(), /9 seasons\s*Matched/);
      if (device === page && process.env.REFERENCE_SCREENSHOT) await detail.screenshot({path:process.env.REFERENCE_SCREENSHOT});
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await device.getByRole("link",{name:"Back to edit rankings"}).click();
      assert.equal(new URL(device.url()).searchParams.get("quality"), "ranked_lost");
      await device.getByRole("link", {name:"Matches gained after the ranked edit: 1 of 14"}).click();
      await device.getByRole("link", {name:/QA 1750/}).click();
      const gained = device.getByRole("region",{name:"Reference answer check"});
      await gained.getByText("Northeast Monsoon", {exact:true}).waitFor();
      assert.equal(await device.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
  }
  assert.deepEqual(errors, []);
  console.log("Verified recommendations, before-inference preview, measured choices and tokens, legacy links, and desktop and mobile layouts.");
} finally {
  await browser.close();
}
