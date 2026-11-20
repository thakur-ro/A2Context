import assert from "node:assert/strict";

const base = new URL(process.argv[2]);
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
try {
  for (const viewport of [{ width: 1152, height: 740 }, { width: 390, height: 844 }]) {
    const page = await browser.newPage({ viewport });
    const errors = [];
    const externalRequests = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("request", request => {
      if (new URL(request.url()).origin !== base.origin) externalRequests.push(request.url());
    });
    const entry = new URL(base);
    entry.searchParams.set("workflow","exchange-agent");
    await page.goto(entry.href);
    await page.getByRole("heading",{name:"2 recommendations",exact:true}).waitFor();
    const cards = page.getByRole("region",{name:"Recommendation cards"});
    assert.equal(await cards.getByRole("article").count(),2);
    const firstCard = await cards.getByRole("article").first().boundingBox();
    assert.ok(firstCard.y < viewport.height);
    if (process.env.CANVAS_CAPTURE) await page.screenshot({path:process.env.CANVAS_CAPTURE+"-home-"+viewport.width+".png"});
    await page.getByRole("button",{name:"Show the context map"}).click();
    const network = page.getByRole("region",{name:"Context map"});
    const overview = network.getByRole("group",{name:"Network overview"});
    await overview.getByRole("button",{name:/^finding · Check current approval/}).press("Enter");
    const focus = network.getByLabel("Focus a node");
    assert.match(await focus.inputValue(),/^finding:/);
    const contextLink = new URL(await network.getByRole("link",{name:"Inspect this evidence →"}).getAttribute("href"),base);
    for (const key of ["trace","call","report","workflow"]) assert.ok(contextLink.searchParams.get(key));
    await network.getByRole("button",{name:/^outcome · exchange requested/}).click();
    assert.match(await focus.inputValue(),/^arm:/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await focus.selectOption({label:"source · confirmation-cache"});
    const historyLink = await network.getByRole("link",{name:"Follow this source across conversations"}).getAttribute("href");
    const historyPage = await browser.newPage({viewport});
    await historyPage.goto(new URL(historyLink,base).href);
    const history = historyPage.getByRole("region",{name:"Source evidence map"});
    await history.getByText("2 calls · 2 identified conversations",{exact:true}).waitFor();
    if (process.env.SOURCE_HISTORY_CAPTURE && viewport.width === 1152) await history.screenshot({path:process.env.SOURCE_HISTORY_CAPTURE});
    await history.getByRole("link",{name:/Wait on the second exchange/}).click();
    const turn = history.locator("li a");
    const exact = new URL(await turn.getAttribute("href"),base);
    assert.equal(exact.searchParams.get("workflow"),"exchange-agent");
    await turn.click();
    await historyPage.locator("#selected-call:focus").waitFor();
    assert.equal(new URL(historyPage.url()).searchParams.get("focus_call"),exact.searchParams.get("focus_call"));
    assert.equal(await historyPage.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await historyPage.close();
    await focus.selectOption("");
    await overview.scrollIntoViewIfNeeded();
    if (process.env.CANVAS_CAPTURE) await network.screenshot({path:process.env.CANVAS_CAPTURE+"-network-"+viewport.width+".png"});
    await page.getByRole("button",{name:"Which instructions disagreed?"}).click();
    const canvas = page.getByRole("region",{name:"Answer canvas"});
    await canvas.getByText('"hold"',{exact:true}).waitFor();
    await canvas.getByText('"approved"',{exact:true}).waitFor();
    const question = page.getByRole("textbox",{name:"Question about workflow evidence"});
    const submit = async text => {await question.fill(text); await question.press("Enter");};
    await submit("Where else did that repeat?");
    await canvas.getByRole("heading",{name:"Where this repeats"}).waitFor();
    await canvas.getByRole("meter").waitFor();
    await canvas.getByRole("link",{name:"Wait on the second exchange"}).waitFor();
    assert.equal(await canvas.locator(".canvas-examples a").count(),2);
    await submit("Which result did the dispatch check change?");
    await canvas.getByText("exchange requested",{exact:true}).waitFor();
    assert.equal(await canvas.locator("a2ui-card").count(),2);
    if (process.env.CANVAS_CAPTURE) await page.screenshot({path:process.env.CANVAS_CAPTURE+"-map-"+viewport.width+".png"});
    const receipt = new URL(await canvas.getByRole("link",{name:"Open exact evidence →"}).getAttribute("href"),base);
    for (const key of ["trace","call","report","recommendation","workflow"]) assert.ok(receipt.searchParams.get(key));
    await submit("Unicorn weather forecast?");
    await canvas.getByText(/couldn’t match/).waitFor();
    await canvas.getByText("Choose different evidence").click();
    await canvas.getByRole("button",{name:"Check current approval before dispatch"}).click();
    await submit("Unicorn weather forecast?");
    await canvas.getByText("exchange requested",{exact:true}).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await page.getByRole("link",{name:"Explore evidence by source"}).click();
    const lead = page.getByRole("link", { name: /See what changed at dispatch/ });
    await lead.getByRole("heading", { name: "See what changed at dispatch", exact: true }).waitFor();
    assert.equal(await lead.locator("a2ui-surface").evaluate(surface => surface.surface.protocolVersion), "1.0");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    const compared = page.getByRole("link", { name:"2 Conversations with a sandbox dispatch comparison" });
    await compared.click();
    await page.getByRole("heading", { name:"Sandbox dispatch comparisons", exact:true }).waitFor();
    await page.getByRole("link", { name:"Only with the check 2 conversations" }).click();
    await page.getByRole("heading", { name:"Only with the check", exact:true }).waitFor();
    await page.getByRole("link", { name:"Only with the check", exact:true }).first().click();
    await page.getByRole("region", { name:"Sandbox dispatch comparison" }).first().waitFor();
    const outcomeParams = new URL(page.url()).searchParams;
    for (const key of ["dispatch_design", "dispatch_outcome", "trace", "call", "report"]) assert.ok(outcomeParams.get(key));
    assert.equal(outcomeParams.get("workflow"), "exchange-agent");
    await page.getByRole("link", { name:"Back to dispatch outcomes", exact:true }).click();
    await page.getByRole("link", { name:"Back to dispatch comparisons", exact:true }).click();
    await page.getByRole("link", { name:"Back to recommendations", exact:true }).click();
    await page.getByRole("link",{name:"Explore evidence by source"}).click();
    await lead.click();
    await page.getByRole("link", { name: "Review recommendation", exact: true }).click();
    const comparison = page.getByRole("region", { name: "Sandbox dispatch comparison" }).first();
    await comparison.waitFor();

    await page.getByRole("link", { name: "Before inference", exact: true }).click();
    await page.getByRole("link", { name: "Check the next exchange", exact: true }).click();
    const earlier = page.getByRole("region", { name: "Earlier dispatch replays" });
    await earlier.getByRole("heading", { name: "Test the dispatch check on this request", exact: true }).waitFor();
    await earlier.getByText(/2 distinct earlier inputs/).waitFor();
    await page.getByRole("heading", { name: "Conflicting values for exchange authorization", exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.getByRole("link", { name:"Inspect saved tool-name scores and proposals" }).click();
    const scores = page.getByRole("region", { name:"Source edits at a supplied prefix" });
    await scores.getByRole("heading", { name:"No edit switched the preferred continuation" }).waitFor();
    assert.equal(await scores.getByRole("row").filter({hasText:"Both edits"}).count(),2);
    assert.equal(await scores.locator(".prefix-proposal strong").count(),4);
    assert.equal(await scores.locator(".prefix-proposal code").filter({hasText:'"new_item_id":"#W8353027"'}).count(),2);
    const blocks = page.getByRole("region",{name:"Decoder-block intervention"});
    await blocks.getByText(/Largest shift.*block 22/).waitFor();
    await blocks.getByText("Inspect both activation positions").click();
    const dot = blocks.getByRole("button",{name:"End of the answer prefix, block 22",exact:true});
    await dot.click();
    await dot.press("ArrowRight");
    assert.match(await blocks.locator(".patch-readout").innerText(),/prefix, block 23.*Self-swap/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await blocks.getByLabel("Activation position").selectOption("last_user_token");
    await blocks.getByText(/Largest shift.*block 13/).waitFor();
    await blocks.getByLabel("Activation position").selectOption("last_prefix_token");
    await blocks.getByLabel("Source intervention").selectOption("1");
    await blocks.getByText(/Largest shift.*block 21/).waitFor();
    await comparison.waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.getByRole("link", { name:"Back to pending request" }).click();
    await earlier.getByRole("button", { name:"Open earlier sandbox comparison" }).press("Enter");
    await comparison.waitFor();
    const params = new URL(page.url()).searchParams;
    assert.equal(params.get("workflow"), "exchange-agent");
    for (const key of ["recommendation", "factor", "trace", "call", "report"]) assert.ok(params.get(key));
    assert.deepEqual(errors, []);
    assert.deepEqual(externalRequests, []);
    await page.close();
  }
  if (process.env.CHAT_MODEL) {
    const page = await browser.newPage();
    await page.goto(new URL("/?workflow=exchange-agent",base).href);
    await page.getByLabel("Analysis partner").selectOption(process.env.CHAT_MODEL);
    const input = page.getByLabel("Question about workflow evidence");
    const answer = page.getByRole("region",{name:"Answer canvas"});
    for (const [question,expected] of [
      ["Which source supplied the value hold for the second order?",'"hold"'],
      ["What changed at dispatch?","exchange requested"],
      ["Where else did that repeat?","Wait on the second exchange"],
    ]) {
      await input.fill(question);
      await input.press("Enter");
      await answer.getByText(question,{exact:true}).waitFor({timeout:65000});
      await answer.getByText(expected,{exact:true}).waitFor();
      assert.equal(await answer.locator(".partner-name").innerText(), question.includes("dispatch") ? "Sandbox replay" : process.env.CHAT_MODEL);
    }
    await page.close();
    console.log("Verified the live partner journey and the measured dispatch receipt.");
  }
  console.log("Verified A2UI rendering and exact evidence navigation on desktop and mobile.");
} finally {
  await browser.close();
}
