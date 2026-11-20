// Record the labeled synthetic booking study loaded by view_calendar_prefixes.py.
import {spawn} from "node:child_process";
import {once} from "node:events";
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const [base, output] = process.argv.slice(2);
if (!base || !output) throw new Error("Usage: node scripts/record_demo.mjs SIDECAR_URL OUTPUT.mp4");
const browser = await chromium.launch({executablePath:process.env.CHROME_BINARY});
try {
  const page = await browser.newPage({viewport:{width:1120,height:820},deviceScaleFactor:1,reducedMotion:"reduce"});
  await page.goto(new URL("/?org=activity-prefix-study-reference",base).href);
  const card = page.getByRole("link",{name:/Compare two questions where the same edit helped/});
  await card.waitFor();
  const fps = 12;
  const encoder = spawn(process.env.FFMPEG || "ffmpeg",["-hide_banner","-loglevel","error","-n","-f","image2pipe",
    "-framerate",String(fps),"-i","pipe:0","-an","-c:v","libx264","-preset","fast","-crf","19","-pix_fmt","yuv420p",
    "-movflags","+faststart",output],{stdio:["pipe","inherit","inherit"]});
  const done = once(encoder,"close");
  async function frames(count) {
    const frame = await page.screenshot();
    for (let i=0;i<count;i++) if (!encoder.stdin.write(frame)) await once(encoder.stdin,"drain");
  }
  async function caption(text) {
    await page.evaluate(text => {
      let sub = document.getElementById("film-caption");
      if (!sub) {
        sub = document.createElement("div");
        sub.id = "film-caption";
        Object.assign(sub.style,{position:"fixed",bottom:"14px",left:"50%",transform:"translateX(-50%)",
          width:"max-content",maxWidth:"90%",padding:"9px 16px",borderRadius:"10px",background:"#20232bf0",
          color:"white",font:"500 17px/1.4 system-ui",zIndex:"1000",pointerEvents:"none",textAlign:"center"});
        document.body.append(sub);
      }
      sub.textContent = text;
    },text);
  }
  async function focus(locator) {
    const distance = await locator.evaluate(el => el.getBoundingClientRect().top - 45);
    const start = await page.evaluate(() => scrollY);
    for (let frame=1;frame<=10;frame++) {
      await page.evaluate(y => scrollTo(0,y),start + distance*(1-Math.pow(1-frame/10,3)));
      await frames(1);
    }
  }
  try {
    await caption("Which context edit is worth investigating?");
    await frames(3*fps);
    await focus(card);
    await caption("Start with a recommendation across the booking conversations.");
    await frames(3*fps);
    await card.click();
    const pair = page.getByRole("region",{name:"Matched source edit"});
    await pair.waitFor();
    await focus(pair);
    await caption("The same edit changed both answers from 30 to 60 minutes.");
    await frames(4*fps);
    await caption("That fixed the greenhouse answer and broke the rehearsal answer.");
    await frames(4*fps);
    await pair.locator("article").last().getByRole("link",{name:"Inspect this edit"}).click();
    const evidence = page.getByRole("region",{name:"Reference continuation check"});
    await evidence.waitFor();
    await focus(evidence);
    await caption("Inspect the source, its replacement, and both generated answers.");
    await frames(4*fps);
    await evidence.getByText("Compare the other source edits",{exact:true}).click();
    await caption("The other calendar was tested separately on the same question.");
    await frames(3*fps);
  } finally {
    encoder.stdin.end();
  }
  const [status] = await done;
  if (status !== 0) throw new Error("FFmpeg exited with status " + status);
} finally {
  await browser.close();
}
