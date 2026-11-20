// Run with PLAYWRIGHT_MODULE set to the installed playwright-core module and
// CHROME_BINARY set to a Chrome or Chromium executable.
import assert from "node:assert/strict";
import process from "node:process";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
const base = process.env.SIDECAR_URL || "http://127.0.0.1:8040";
const errors = [];
const conflictCard = (page) => page.locator(".card").filter({ hasText: "Conflicting values" }).first();

try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 950 } });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(base);
  await conflictCard(page).waitFor();
  assert.match(await conflictCard(page).innerText(), /return_window_days/);
  if (process.env.FINDINGS_SCREENSHOT) await page.screenshot({ path: process.env.FINDINGS_SCREENSHOT });

  await page.locator("#filters summary").click();
  await page.getByLabel("Organization").fill("demo-retail");
  await page.getByLabel("User", { exact: true }).fill("user-042");
  await page.getByRole("button", { name: "Apply filters" }).click();
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
  await page.getByRole("link", { name: "Threads", exact: true }).click();
  await page.getByRole("link", { name: /damaged-item/ }).click();
  await page.getByText("Traces in chronological order.").waitFor();
  assert.equal(new URL(page.url()).searchParams.get("user"), "user-042");

  await page.goto(base);
  const replay = page.locator(".card").filter({ hasText: "Verified replay" }).first();
  await replay.getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText("Matches by trial arm", { exact: true }).waitFor();
  await page.locator("summary", { hasText: "Underlying evidence" }).click();
  const replayLink = page.getByRole("link", { name: "Open trial comparison" }).first();
  const legacy = await replayLink.getAttribute("href");
  await replayLink.click();
  await page.getByText("Baseline matched the recorded decision in 1 of 1 completed trials", { exact: true }).waitFor();
  await page.goto(base + "/" + new URL(legacy, base).hash);
  await page.waitForURL("**/details?**");
  await page.getByText("Baseline matched the recorded decision in 1 of 1 completed trials", { exact: true }).waitFor();

  await page.goto(base);
  const measured = page.locator(".card").filter({ hasText: "switched the preferred alternative" });
  await measured.getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText("stock favored", { exact: true }).waitFor();
  await page.getByText("price favored", { exact: true }).waitFor();
  await page.getByText(/store-name.*-0\.089/).waitFor();
  await page.goto(base);
  await page.locator(".card").filter({ hasText: "Token measurements" }).getByRole("link", { name: "Review recommendation" }).click();
  await page.getByText(/largest tested decoder-block swap/).waitFor();
  await page.getByRole("link", { name: "Before inference" }).click();
  await page.getByRole("button", { name: "Try a measured tool choice" }).click();
  await page.getByText("price favored", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Try a policy conflict" }).click();
  await page.getByRole("heading", { name: "Resolve the competing rules" }).waitFor();
  assert.equal(new URL(page.url()).searchParams.get("view"), "preview");

  const mobile = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, deviceScaleFactor: 1 });
  mobile.on("pageerror", (error) => errors.push(error.message));
  await mobile.goto(base);
  await conflictCard(mobile).waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  if (process.env.MOBILE_SCREENSHOT) await mobile.screenshot({ path: process.env.MOBILE_SCREENSHOT });
  await conflictCard(mobile).getByRole("link", { name: "Review recommendation" }).click();
  await mobile.getByRole("heading", { name: "Recommendation", exact: true }).waitFor();
  assert.equal(await mobile.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  assert.deepEqual(errors, []);
  console.log("Verified recommendations, before-inference preview, measured choices and tokens, legacy links, and desktop and mobile layouts.");
} finally {
  await browser.close();
}
