import assert from "node:assert/strict";

const base = new URL(process.argv[2]);
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  for (const width of [1152,390]) {
    const page = await browser.newPage({viewport:{width,height:850}});
    const answer = page.getByRole("region",{name:"Answer canvas"});
    const input = page.getByRole("textbox",{name:"Question about workflow evidence"});
    const ask = async q => {await input.fill(q); await input.press("Enter");};
    const read = async (path,params) => {
      const response = await page.request.get(new URL("/api/sidecar/"+path,base).href,{params});
      assert.ok(response.ok());
      return response.json();
    };
    await page.goto(base.href);
    await ask("Which issues keep coming up for this user?");
    await answer.getByRole("heading",{name:"Which user ID?"}).waitFor();
    await ask("support-17");
    const chart = answer.getByRole("region",{name:"Recurring themes"});
    await chart.waitFor();
    const user = {user:"support-17"};
    const summary = await read("summary",user);
    const {coverage} = await read("health",user);
    assert.ok((await page.locator("#status").textContent()).includes(coverage.with_inputs + "/" + coverage.inference_spans + " inference spans with inputs"));
    assert.ok(summary.captured_conversations > 1);
    assert.ok(summary.captured_conversations < (await read("summary",{})).captured_conversations);
    assert.deepEqual(await chart.locator("progress").evaluateAll(bars => bars.map(b => [b.value,b.max])),
      summary.themes.map(t => [t.conversations,summary.captured_conversations]));
    const sample = (await read("findings",{...user,kind:summary.themes[0].kind,limit:1})).rows[0];
    await chart.getByRole("button").first().click();
    await answer.locator(`[data-group="${sample.id}"]`).waitFor();
    assert.equal(await answer.locator("[data-group]").getAttribute("data-call"),sample.evidence.call_id);
    assert.deepEqual((await read("workflows",{...user,after:"2100-01-01"})).workflows,[]);
    assert.equal(new URL(page.url()).searchParams.get("user"),user.user);
    const workflows = page.getByRole("navigation",{name:"Workflows"});
    const links = await workflows.getByRole("link").evaluateAll(links => links.map(a => a.href));
    assert.ok(links.every(href => new URL(href).searchParams.get("user") === user.user));
    assert.deepEqual(links.map(href => new URL(href).searchParams.get("workflow")).filter(Boolean),(await read("workflows",user)).workflows);
    await page.evaluate(() => render(true));
    await ask("Show recorded activity");
    const threads = await read("threads",user);
    for (const thread of threads.rows.slice(0,3)) await answer.getByRole("heading",{name:thread.thread,exact:true}).waitFor();
    await workflows.getByRole("link",{name:"Returns demo"}).click();
    await page.getByRole("heading",{name:"Returns demo",exact:true}).waitFor();
    assert.equal(new URL(page.url()).searchParams.get("user"),user.user);
    await ask('Which issues keep coming up for user "Customer 042"?');
    await chart.getByText("1 of 1",{exact:true}).waitFor();
    await ask('Which issues keep coming up for user "no-recorded-user-598"?');
    await answer.getByRole("heading",{name:"No recorded conversations for this user"}).waitFor();
    assert.equal(await chart.count(),0);
    await page.reload();
    await input.waitFor();
    await page.locator(".canvas-home").getByText("User · no-recorded-user-598",{exact:true}).waitFor();
    await ask("Show all users");
    await page.waitForURL(url => !url.searchParams.has("user"));
    assert.equal(new URL(page.url()).searchParams.get("workflow"),"returns-demo");
    await page.goto(new URL("/?org=journal-south",base).href);
    await ask("Which issues recur for user support-41?");
    await answer.getByText("1 captured conversation in this scope. No finding groups were recorded.",{exact:true}).waitFor();
    await ask("Which issues recur for user support-17?");
    await answer.getByRole("heading",{name:"No recorded conversations for this user"}).waitFor();
    await ask("Show all users");
    await page.waitForURL(url => !url.searchParams.has("user"));
    assert.equal(new URL(page.url()).searchParams.get("org"),"journal-south");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),true);
    await page.close();
  }
  console.log("Verified named-user counts, follow-ups, sidebar scope, reload, empty evidence and organization limits at two widths.");
} finally {
  await browser.close();
}
