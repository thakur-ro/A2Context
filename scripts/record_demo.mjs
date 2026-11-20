// Record the exchange-approval workflow loaded by make demo.
import {spawn} from "node:child_process";
import {once} from "node:events";
import {readFile} from "node:fs/promises";
const [base, output] = process.argv.slice(2);
if (!base || !output) throw new Error("Usage: node scripts/record_demo.mjs SIDECAR_URL OUTPUT.mp4");
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
const fps = 15;
try {
  const page = await browser.newPage({viewport:{width:1120,height:720},deviceScaleFactor:1,reducedMotion:"reduce"});
  const dashboard = new URL("/?workflow=exchange-agent",base).href;
  await page.goto(dashboard);
  await page.getByRole("heading",{name:"Conversations with findings"}).waitFor();
  const encoder = spawn(process.env.FFMPEG || "ffmpeg",["-hide_banner","-loglevel","error","-n","-f","image2pipe",
    "-framerate",String(fps),"-i","pipe:0","-an","-c:v","libx264","-preset","fast","-crf","18","-pix_fmt","yuv420p",
    "-movflags","+faststart",output],{stdio:["pipe","inherit","inherit"]});
  const done = once(encoder,"close");
  async function hold(seconds, scene = page) {
    const frame = await scene.screenshot();
    for (let i=0;i<Math.round(seconds*fps);i++) {
      if (!encoder.stdin.write(frame)) await once(encoder.stdin,"drain");
    }
  }
  async function capture(locator, text, seconds) {
    await locator.evaluate(node => {
      node.style.outline = "3px solid #456db7";
      node.style.outlineOffset = "5px";
      window.scrollBy(0,node.getBoundingClientRect().top-58);
    });
    await page.evaluate(text => {
      let caption = document.getElementById("film-caption");
      if (!caption) {
        caption = document.createElement("div");
        caption.id = "film-caption";
        Object.assign(caption.style,{position:"fixed",bottom:"22px",left:"50%",transform:"translateX(-50%)",
          width:"max-content",maxWidth:"90%",padding:"13px 22px",borderRadius:"9px",background:"#182a47",
          color:"white",font:"600 21px/1.3 system-ui",zIndex:"1000",pointerEvents:"none",textAlign:"center"});
        document.body.append(caption);
      }
      caption.textContent = text;
    },text);
    await hold(seconds);
    await locator.evaluate(node => {
      node.style.removeProperty("outline");
      node.style.removeProperty("outline-offset");
    });
  }
  const slides = await browser.newPage({viewport:{width:1120,height:720},deviceScaleFactor:1});
  async function slide(kicker, title, cards, footer, seconds) {
    await slides.setContent(`<!doctype html><html lang="en"><meta charset="utf-8">
      <style>
        * { box-sizing:border-box } body { margin:0; padding:70px 78px; color:#edf3ff; background:#152640;
          font:22px/1.45 system-ui,sans-serif } small { font-weight:650; color:#a7bfe6 }
        h1 { font-size:64px; line-height:1.07; letter-spacing:-.06em; margin:32px 0 }
        section { display:grid; grid-template-columns:1fr 1fr; gap:18px; margin-top:32px }
        article { padding:19px 24px; background:#213957; border:1px solid #435b7c; border-radius:12px }
        strong { display:block; margin-top:8px; font-size:26px }
        footer { margin-top:28px; color:#afc1dc; font-size:18px; max-width:900px }
      </style>
      <small>NexusWeave &nbsp; / &nbsp; ${kicker}</small>
      <h1>${title}</h1>
      <section>${cards.map(([name,value]) => `<article><small>${name}</small><strong>${value}</strong></article>`).join("")}</section>
      <footer>${footer}</footer>
    </html>`);
    await hold(seconds, slides);
  }
  try {
    const results = JSON.parse(await readFile(new URL("../examples/exchange-approval/results.json",import.meta.url),"utf8"));
    const held = Object.values(results).filter(row => row.sandbox.refreshed);
    const writes = guard => held.filter(row => row.sandbox.refreshed[guard].dispatch === "written").length;

    await slide("Retail agent replay","The boots exchange started.<br>Then the customer said stop.",
      [["First order","Exchange requested"],["Second order","Customer put it on hold"]],
      "Derived from a public τ²-Bench trace. The hold scenarios and replays run in a sandbox.",4.3);
    const impact = page.getByRole("region",{name:"Conversation impact"});
    await capture(impact,"Two hold scenarios left the agent proposing the same write.",3.4);
    await page.getByRole("link",{name:/Equal match rates/}).click();
    await page.getByRole("link",{name:"Review recommendation",exact:true}).click();
    await page.getByText("Underlying evidence",{exact:true}).click();
    await page.getByRole("link",{name:"Open original span",exact:true}).click();
    const tree = page.locator(".panel").filter({has:page.getByRole("heading",{name:"Span tree",exact:true})});
    await tree.getByText("Propose the second exchange",{exact:false}).first().waitFor();
    await capture(tree,"Identity, order and inventory checks came before the first exchange.",4.7);
    await page.goto(dashboard);
    await page.locator(".featured").filter({hasText:"Conflicting rules"}).click();
    await page.getByRole("link",{name:"Review recommendation",exact:true}).click();
    const declarations = page.locator(".panel .visual").filter({hasText:"Captured declarations"}).first();
    await declarations.getByText("confirmation-cache",{exact:true}).waitFor();
    await capture(declarations,"The cached approval still said yes. The customer’s latest message said hold.",5);
    await page.goto(dashboard);
    await page.getByRole("link",{name:/Equal match rates/}).click();
    await page.getByRole("link",{name:"Review recommendation",exact:true}).click();
    await page.getByText("Show tested replacement",{exact:true}).click();
    const edit = page.getByText("Show tested replacement",{exact:true}).locator("..");
    await capture(edit,"Replay the decision with the approval cache refreshed.",3.8);
    const proposals = page.locator("details.response-pair");
    await proposals.getByText("Compare the matching proposals",{exact:true}).click();
    await capture(proposals,"The model still asks to exchange the puzzle in both replays.",5.2);
    await slide("SQLite sandbox","Check authorization<br>before the write.",
      [["Refreshed cache alone",`${writes("without_gate")}/${held.length} unwanted writes`],
       ["Dispatch checks current approval",`${writes("with_gate")}/${held.length} unwanted writes`]],
      "Same model proposals in both arms. The previously requested boots exchange stayed in place.",5.1);
  } finally {
    encoder.stdin.end();
  }
  const [status] = await done;
  if (status !== 0) throw new Error("FFmpeg exited with status " + status);
} finally {
  await browser.close();
}
