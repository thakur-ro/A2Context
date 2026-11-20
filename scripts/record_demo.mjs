// Record the exchange workflow loaded by make demo at 1920×1080.
import { spawn } from "node:child_process";
import { once } from "node:events";

const [base, output] = process.argv.slice(2);
if (!base || !output) throw new Error("Usage: node scripts/record_demo.mjs SIDECAR_URL OUTPUT.mp4");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright-core");
const browser = await chromium.launch({ executablePath: process.env.CHROME_BINARY });
const fps = 24;
const viewport = { width: 1152, height: 648 };
let pointer = { x: 943, y: 150 };

try {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 5 / 3, reducedMotion: "reduce" });
  page.setDefaultTimeout(8000);
  // A browser screenshot does not include the system pointer. Draw one at the real click coordinates.
  await page.addInitScript(() => {
    window.addEventListener("DOMContentLoaded", () => {
      const overlay = document.createElement("div");
      overlay.setAttribute("aria-hidden", "true");
      overlay.style.cssText = "position:fixed;inset:0;pointer-events:none;z-index:1000";
      const arrow = document.createElement("div");
      arrow.style.cssText = "position:absolute;width:26px;height:32px;filter:drop-shadow(0 1px 2px #17202c99)";
      const shape = "polygon(0 0, 95% 58%, 58% 65%, 74% 91%, 58% 100%, 39% 73%, 12% 96%)";
      for (const [inset, color] of [["0", "white"], ["2px", "#18212c"]]) {
        const layer = document.createElement("span");
        layer.style.cssText = `position:absolute;inset:${inset};background:${color};clip-path:${shape}`;
        arrow.append(layer);
      }
      const ring = document.createElement("div");
      ring.style.cssText = "position:absolute;width:26px;height:26px;border:2px solid #2353a3;border-radius:50%;display:none";
      const caption = document.createElement("div");
      caption.style.cssText = "position:absolute;bottom:16px;left:50%;transform:translateX(-50%);width:max-content;max-width:90%;padding:9px 16px;border-radius:6px;background:#1b2636;color:white;font:600 17px/1.35 system-ui;text-align:center";
      overlay.append(ring, arrow, caption);
      document.body.append(overlay);
      window.demoOverlay = ({ x, y }, text, pressed) => {
        arrow.style.transform = `translate(${x}px,${y}px)`;
        ring.style.transform = `translate(${x - 13}px,${y - 13}px)`;
        ring.style.display = pressed ? "block" : "none";
        caption.textContent = text;
        caption.hidden = !text;
      };
    });
  });
  await page.goto(new URL("/?workflow=exchange-agent", base).href);
  const lead = page.getByRole("link", { name: /See what changed at dispatch/ });
  await lead.waitFor();

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
    for (let i = 1; i <= 12; i++) {
      const t = i / 12;
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
    for (let i = 1; i <= 12; i++) {
      const t = i / 12;
      const ease = t * t * (3 - 2 * t);
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
    await read("A customer paused the second exchange. The agent still proposed it.", 3.4);
    await click(lead);
    const review = page.getByRole("link", { name: "Review recommendation", exact: true });
    await click(review);
    const proposals = page.locator("details.response-pair");
    await click(proposals.getByText("Compare the matching proposals", { exact: true }));
    await read("Refreshing the approval cache left the proposed tool call unchanged.", 4.6, proposals, 112);
    const dispatch = page.getByRole("region", { name: "Sandbox dispatch comparison" }).first();
    await read("The same proposal ran twice. Only the dispatch check kept the puzzle order on hold.", 5.8, dispatch, 74);
    await click(page.getByRole("link", { name: "Before inference", exact: true }));
    await click(page.getByRole("link", { name: "Check the next exchange", exact: true }));
    const earlier = page.getByRole("region", { name: "Earlier dispatch replays" });
    await read("The next request surfaces both earlier results before it calls the model.", 5, earlier, 93);
    await click(earlier.getByRole("button", { name:"Open earlier sandbox comparison" }));
    await read("The recommendation opens the exact sandbox comparison behind it.", 3.5, dispatch, 74);
  } finally {
    encoder.stdin.end();
    const [status] = await done;
    if (status !== 0) throw new Error("FFmpeg exited with status " + status);
  }
} finally {
  await browser.close();
}
