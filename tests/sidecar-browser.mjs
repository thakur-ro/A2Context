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
  assert.equal(await page.locator(".impact-number").innerText(), "11");
  await page.getByText("5 with a measured change").waitFor();
  assert.equal(await page.locator("#navigation a").count(), 2);
  await factor(page, "Choice flipped").waitFor();
  if (process.env.FINDINGS_SCREENSHOT) await page.screenshot({ path: process.env.FINDINGS_SCREENSHOT });

  await factor(page, "Only the joint edit").click();
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
  await replayLink.click();
  await page.getByText("Baseline matched the recorded decision in 1 of 1 completed trials", { exact: true }).waitFor();
  await page.goto(base + "/" + new URL(legacy, base).hash);
  await page.waitForURL("**/details?**");
  await page.getByText("Baseline matched the recorded decision in 1 of 1 completed trials", { exact: true }).waitFor();

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
  await factor(mobile, "Conflicting rules").click();
  await conflictCard(mobile).getByRole("link", { name: "Review recommendation" }).click();
  await mobile.getByRole("heading", { name: "Recommendation", exact: true }).waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  assert.deepEqual(errors, []);
  console.log("Verified recommendations, before-inference preview, measured choices and tokens, legacy links, and desktop and mobile layouts.");
} finally {
  await browser.close();
}
