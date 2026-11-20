// Record the exchange workflow loaded by make demo at 2560×1440.
import { spawn } from "node:child_process";
import { once } from "node:events";

const [base, output, model] = process.argv.slice(2);
if (!base || !output) throw new Error("Usage: node scripts/record_demo.mjs SIDECAR_URL OUTPUT.mp4 [MODEL]");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
const fps = 30;
const viewport = { width: 1280, height: 720 };
let pointer = { x: 943, y: 150 };

try {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 2, reducedMotion: "reduce" });
  page.setDefaultTimeout(model ? 60000 : 8000);
  // A browser screenshot does not include the system pointer. Draw one at the real click coordinates.
  await page.addInitScript(() => {
    window.addEventListener("DOMContentLoaded", () => {
      const overlay = document.createElement("div");
      overlay.setAttribute("aria-hidden", "true");
      overlay.style.cssText = "position:fixed;inset:0;pointer-events:none;z-index:1000";
      const arrow = document.createElement("div");
      arrow.style.cssText = "position:absolute;width:16px;height:23px;filter:drop-shadow(0 1px 1px #17202c66)";
      const shape = "polygon(0 0, 95% 58%, 58% 65%, 74% 91%, 58% 100%, 39% 73%, 12% 96%)";
      for (const [inset, color] of [["0", "white"], ["1px", "#18212c"]]) {
        const layer = document.createElement("span");
        layer.style.cssText = `position:absolute;inset:${inset};background:${color};clip-path:${shape}`;
        arrow.append(layer);
      }
      const ring = document.createElement("div");
      ring.style.cssText = "position:absolute;width:16px;height:16px;border:1px solid #778da7;border-radius:50%;display:none";
      const caption = document.createElement("div");
      caption.style.cssText = "position:absolute;bottom:16px;left:50%;transform:translateX(-50%);width:max-content;max-width:90%;padding:9px 16px;border-radius:6px;background:#1b2636;color:white;font:500 15px/1.35 system-ui;text-align:center";
      overlay.append(ring, arrow, caption);
      document.body.append(overlay);
      window.demoOverlay = ({ x, y }, text, pressed) => {
        arrow.style.transform = `translate(${x}px,${y}px)`;
        ring.style.transform = `translate(${x - 8}px,${y - 8}px)`;
        ring.style.display = pressed ? "block" : "none";
        caption.textContent = text;
        caption.hidden = !text;
      };
    });
  });
  await page.goto(new URL("/?workflow=exchange-agent", base).href);
  await page.getByRole("heading",{name:"2 recommendations",exact:true}).waitFor();

  const encoder = spawn(process.env.FFMPEG || "ffmpeg", [
    "-hide_banner", "-loglevel", "error", "-n", "-f", "image2pipe", "-framerate", String(fps),
    "-i", "pipe:0", "-an", "-c:v", "libx264", "-preset", "fast", "-crf", "16", "-pix_fmt", "yuv420p",
    "-movflags", "+faststart", output,
  ], { stdio: ["pipe", "inherit", "inherit"] });
  const done = once(encoder, "close");
  let caption = "";

  async function frame(repeat = 1, pressed = false) {
    await page.evaluate(([point, text, down]) => window.demoOverlay(point, text, down), [pointer, caption, pressed]);
    const png = await page.screenshot({ scale: "device" });
    for (let i = 0; i < repeat; i++) {
      if (!encoder.stdin.write(png)) await once(encoder.stdin, "drain");
    }
  }

  async function focus(locator, top = 85, force = false) {
    await locator.waitFor();
    const motion = await locator.evaluate((node, top) => {
      const bounds = node.getBoundingClientRect();
      return { from: scrollY, to: scrollY + bounds.top - top, bottom: bounds.bottom, top: bounds.top };
    }, top);
    if (!force && motion.top > 14 && motion.bottom < viewport.height - 64) return;
    const end = Math.max(0, await page.evaluate(y => Math.min(y, document.documentElement.scrollHeight - innerHeight), motion.to));
    if (Math.abs(end - motion.from) < 5) return;
    const steps = Math.max(8, Math.ceil(Math.abs(end - motion.from) / 540 * fps));
    for (let i = 1; i <= steps; i++) {
      const t = i / steps;
      const y = motion.from + (end - motion.from) * (t * t * (3 - 2 * t));
      await page.evaluate(y => scrollTo(0, y), y);
      await frame();
    }
  }

  async function click(locator) {
    await focus(locator);
    const box = await locator.boundingBox();
    if (!box) throw new Error("The control disappeared before its click.");
    const target = { x: box.x + Math.min(box.width / 2, 170), y: box.y + Math.min(box.height / 2, 24) };
    const start = pointer;
    const steps = Math.max(8, Math.ceil(Math.hypot(target.x-start.x,target.y-start.y) / 690 * fps));
    for (let i = 1; i <= steps; i++) {
      const t = i / steps;
      const ease = t - Math.sin(2 * Math.PI * t) * .045;
      pointer = { x: start.x + (target.x - start.x) * ease, y: start.y + (target.y - start.y) * ease + 12 * Math.sin(t * Math.PI) };
      await page.mouse.move(pointer.x, pointer.y);
      await frame();
    }
    await page.mouse.down();
    await frame(3, true);
    await page.mouse.up();
  }

  async function read(text, seconds, locator, top) {
    caption = text;
    if (locator) await focus(locator, top, true);
    await frame(Math.round(seconds * fps));
  }

  try {
    await read("One exchange was approved. Then the customer put the second order on hold.", 3.8);
    if (model) {
      const picker = page.getByLabel("Analysis partner");
      await click(picker);
      await picker.press("Escape");
      await picker.selectOption(model);
      await read("Ask " + model + " about the evidence captured for this workflow.", 2);
    }
    await click(page.getByRole("button",{name:"Show the context map"}));
    const network = page.getByRole("region",{name:"Context map"});
    await read("The same confirmation cache appears in both findings.", 2.8, network, 38);
    await click(network.getByRole("button",{name:"source · confirmation-cache",exact:true}));
    await read("Follow the cache to the conflicting instructions and the dispatch replay.", 2.8);
    await click(page.getByRole("button",{name:"Which instructions disagreed?"}));
    const canvas = page.getByRole("region",{name:"Answer canvas"});
    await canvas.getByText('"hold"',{exact:true}).waitFor();
    await read("The confirmation cache still said approved. The latest customer message said hold.", 3.8, canvas, 65);
    const question = page.getByRole("textbox",{name:"Question about workflow evidence"});
    await click(question);
    caption = "";
    for (const char of "What changed at dispatch?") {
      await question.pressSequentially(char);
      await frame(2);
    }
    await click(page.getByRole("button",{name:"Ask question",exact:true}));
    await canvas.getByText("exchange requested",{exact:true}).waitFor();
    await read("Same proposal, same starting state. Checking approval prevented the second write.", 4.8);
    await click(question);
    caption = "";
    for (const char of "Where else did that repeat?") {
      await question.pressSequentially(char);
      await frame(2);
    }
    await question.press("Enter");
    await canvas.getByRole("heading",{name:"Where this repeats"}).waitFor();
    await canvas.getByRole("link",{name:"Wait on the second exchange"}).waitFor();
    await read("Two conversations had this finding. Open the affected requests to inspect either one.", 3.8);
    await click(canvas.getByRole("link",{name:"See affected requests →"}));
    const dispatch = page.getByRole("region",{name:"Sandbox dispatch comparison"}).first();
    await read("The replay receipt shows that the first exchange stayed in place in both runs.", 4.4, dispatch, 115);
  } finally {
    encoder.stdin.end();
    const [status] = await done;
    if (status !== 0) throw new Error("FFmpeg exited with status " + status);
  }
} finally {
  await browser.close();
}
