import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  const base = process.argv[2];
  const page = await browser.newPage({viewport:{width:390,height:844}});
  await page.goto(base);
  const setup = page.getByRole("region",{name:"Receiver setup"});
  await page.getByRole("heading",{name:"Connect your workflow"}).waitFor();
  const endpoint = base + "/v1/traces";
  assert.equal(await setup.locator("code").innerText(),
    "export OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf\nexport OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=" + endpoint);
  await setup.getByRole("link",{name:"Check a request before connecting"}).click();
  await page.locator('input[type="file"]').setInputFiles("examples/exchange-approval/pending.json");
  await page.getByText(/captured inputs · 1 finding$/).waitFor();
  await page.locator('input[type="file"]').setInputFiles("examples/bfcl-transfer/mlx/tool-scope/preview.json");
  const catalog = page.getByRole("region",{name:"Tool catalog scope"});
  await catalog.getByRole("heading",{name:"Review tools outside the workflow scope"}).waitFor();
  await catalog.getByText("3 tool definitions · 2 outside scope · 0 unclassified").waitFor();
  assert.deepEqual(await catalog.getByRole("listitem").allTextContents(),
    ["authenticate_travel at /tools/1","book_flight at /tools/2"]);
  await page.goto(base);
  await setup.waitFor();
  const sample = await readFile(new URL("../examples/exchange-approval/exchange.otlp.json",import.meta.url),"utf8");
  const response = await page.request.post(endpoint,{data:sample,headers:{"Content-Type":"application/json"}});
  assert.ok(response.ok());
  await page.reload();
  await page.getByRole("heading",{name:"Telemetry received"}).waitFor();
  await setup.getByText("3 traces · 3 inference spans").waitFor();
  const href = await setup.getByRole("link",{name:"Check receiver status"}).getAttribute("href");
  const health = await page.request.get(new URL(href,base).href);
  assert.equal((await health.json()).content_capture,false);
  await setup.getByRole("link",{name:"Threads"}).click();
  await page.getByRole("heading",{name:"Threads",exact:true}).waitFor();
  await page.getByRole("link",{name:/Wait on the second exchange/}).waitFor();
  await page.goto(base+"/?workflow=unrelated");
  await page.getByText("No recommendations match these filters.").waitFor();
  assert.equal(await setup.count(),0);
  console.log("Verified first-run guidance, sample OTLP delivery, metadata-only coverage and filtered empty results.");
} finally {
  await browser.close();
}
