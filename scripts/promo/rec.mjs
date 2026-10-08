// Deterministic frame-stepped recorder. Time in the page only moves when we
// advance it: JS timers via Playwright's clock, CSS animations/transitions by
// seeking every live Animation by the same dt. Each frame is a screenshot.
import { boot } from "./boot.mjs";
import fs from "fs";
import path from "path";
import { execFileSync } from "child_process";

const HERE = path.dirname(new URL(import.meta.url).pathname);
const OUT = path.join(HERE, "out");
export const FPS = 30;
const DT = 1000 / FPS;

const CURSOR_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="26" height="34" viewBox="0 0 26 34"><path d="M2 2 L2 26 L8 20.5 L12.5 31 L17 29 L12.6 18.8 L20.5 18.8 Z" fill="#111" stroke="#fff" stroke-width="2" stroke-linejoin="round"/></svg>`;

const DECOR = `(() => {
  const st = document.createElement('style');
  st.textContent = \`
  #__tl{position:fixed;left:0;top:0;height:40px;display:flex;align-items:center;gap:8px;padding-left:13px;z-index:2147483646;pointer-events:none}
  #__tl i{width:12px;height:12px;border-radius:50%;display:block;box-shadow:inset 0 0 0 .5px rgba(0,0,0,.25)}
  #__cur{position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;will-change:transform;filter:drop-shadow(0 2px 3px rgba(0,0,0,.45));transform-origin:2px 2px}
  #__ring{position:fixed;left:0;top:0;width:40px;height:40px;margin:-20px 0 0 -20px;border-radius:50%;border:2px solid rgba(137,180,250,.9);z-index:2147483646;pointer-events:none;opacity:0}
  \`;
  document.head.appendChild(st);
  const tl = document.createElement('div'); tl.id='__tl';
  tl.innerHTML = '<i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i>';
  document.body.appendChild(tl);
  const c = document.createElement('div'); c.id='__cur'; c.innerHTML = ${JSON.stringify(CURSOR_SVG)};
  c.style.display = 'none';
  document.body.appendChild(c);
  const r = document.createElement('div'); r.id='__ring'; document.body.appendChild(r);
})()`;

const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

export class Recorder {
  constructor(name) {
    this.name = name;
    this.dir = path.join(OUT, "frames", name);
    fs.rmSync(this.dir, { recursive: true, force: true });
    fs.mkdirSync(this.dir, { recursive: true });
    this.n = 0;
    this.cur = { x: 800, y: 600, visible: false, scale: 1 };
    this.ring = null;
    this.recording = false;
  }

  async start({ url = "http://localhost:1420/", setup } = {}) {
    const b = await boot({ clock: true });
    Object.assign(this, b);
    const page = this.page;
    if (setup) await page.addInitScript(setup);
    await page.goto(url);
    await page.waitForLoadState("domcontentloaded");
    this.cdp = await this.ctx.newCDPSession(page);
    await this.cdp.send("Animation.enable");
    await this.cdp.send("Animation.setPlaybackRate", { playbackRate: 0 });
    // Boot: let the app load modules (real time) and run its first timers.
    for (let i = 0; i < 40; i++) {
      await page.waitForTimeout(50);
      await page.clock.runFor(100);
    }
    await page.waitForSelector(".titlebar, .shell", { timeout: 20000 });
    await page.evaluate(DECOR);
    await page.evaluate(() => window.__go && window.__go());
    await this.advance(500);
  }

  // Move page time forward without capturing.
  async advance(ms) {
    const steps = Math.ceil(ms / DT);
    for (let i = 0; i < steps; i++) await this._tick();
  }

  async _tick() {
    await this.page.clock.runFor(DT);
    await this.page.evaluate((dt) => {
      for (const a of document.getAnimations()) {
        try {
          const t = (a.currentTime ?? 0) + dt;
          a.currentTime = t;
        } catch {}
      }
      return new Promise((r) => { const c = new MessageChannel(); c.port1.onmessage = () => r(); c.port2.postMessage(0); });
    }, DT);
  }

  async _paintCursor() {
    const { x, y, visible, scale } = this.cur;
    const ring = this.ring;
    await this.page.evaluate(({ x, y, visible, scale, ring }) => {
      const c = document.getElementById("__cur");
      if (!c) return;
      c.style.display = visible ? "block" : "none";
      c.style.transform = `translate(${x - 2}px, ${y - 2}px) scale(${scale})`;
      const r = document.getElementById("__ring");
      if (ring) { r.style.opacity = String(ring.o); r.style.transform = `translate(${ring.x}px, ${ring.y}px) scale(${ring.s})`; }
      else r.style.opacity = "0";
    }, { x, y, visible, scale, ring });
  }

  async frame() {
    await this._tick();
    await this._paintCursor();
    const file = path.join(this.dir, String(this.n++).padStart(5, "0") + ".png");
    await this.page.screenshot({ path: file, animations: "allow", caret: "initial" });
  }

  async hold(sec) {
    const n = Math.round(sec * FPS);
    for (let i = 0; i < n; i++) await this.frame();
  }

  async showCursor(x, y) {
    this.cur = { ...this.cur, x, y, visible: true };
    await this.page.mouse.move(x, y);
  }

  async target(loc) {
    if (typeof loc === "string") loc = this.page.locator(loc).first();
    if (loc.x !== undefined) return loc;
    await loc.waitFor({ state: "visible", timeout: 10000 });
    const bb = await loc.boundingBox();
    return { x: bb.x + Math.min(bb.width / 2, 60), y: bb.y + bb.height / 2 };
  }

  async moveTo(loc, sec = 0.7) {
    const to = await this.target(loc);
    const from = { x: this.cur.x, y: this.cur.y };
    if (!this.cur.visible) { this.cur.visible = true; }
    const n = Math.max(1, Math.round(sec * FPS));
    // Gentle arc so it doesn't look robotic.
    const dx = to.x - from.x, dy = to.y - from.y;
    const bow = Math.min(60, Math.hypot(dx, dy) * 0.12);
    for (let i = 1; i <= n; i++) {
      const t = ease(i / n);
      const off = Math.sin(Math.PI * t) * bow;
      const len = Math.hypot(dx, dy) || 1;
      this.cur.x = from.x + dx * t + (-dy / len) * off;
      this.cur.y = from.y + dy * t + (dx / len) * off;
      await this.page.mouse.move(this.cur.x, this.cur.y);
      await this.frame();
    }
    return to;
  }

  async click(loc, { sec = 0.7, after = 0.4, before = 0.12 } = {}) {
    const to = await this.moveTo(loc, sec);
    await this.hold(before);
    this.cur.scale = 0.85;
    this.ring = { x: to.x, y: to.y, s: 0.4, o: 0.9 };
    await this.frame();
    await this.page.mouse.down();
    await this.page.mouse.up();
    this.cur.scale = 1;
    for (let i = 0; i < 9; i++) {
      this.ring = { x: to.x, y: to.y, s: 0.4 + i * 0.09, o: 0.9 * (1 - i / 9) };
      await this.frame();
    }
    this.ring = null;
    await this.hold(after);
  }

  async type(text, { cps = 22 } = {}) {
    const per = FPS / cps;
    let acc = 0;
    for (const ch of text) {
      await this.page.keyboard.type(ch);
      acc += per;
      while (acc >= 1) { await this.frame(); acc -= 1; }
    }
    await this.frame();
  }

  async press(key) { await this.page.keyboard.press(key); await this.frame(); }

  async still(name) {
    fs.mkdirSync(path.join(OUT, "stills"), { recursive: true });
    const out = path.join(OUT, "stills", `${name}.png`);
    // Stills without the fake cursor.
    const vis = this.cur.visible;
    this.cur.visible = false; await this._paintCursor();
    await this.page.screenshot({ path: out });
    this.cur.visible = vis; await this._paintCursor();
    return out;
  }

  async finish() {
    await this.browser.close();
    fs.mkdirSync(path.join(OUT, "clips"), { recursive: true });
    const out = path.join(OUT, "clips", `${this.name}.mp4`);
    execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-framerate", String(FPS), "-i", path.join(this.dir, "%05d.png"),
      "-c:v", "libx264", "-preset", "slow", "-crf", "12", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out]);
    const errs = (this.errors || []).filter((e) => !e.includes("[mock] unhandled"));
    if (errs.length) console.log(`[${this.name}] page errors:\n` + errs.slice(0, 15).join("\n"));
    const un = this.errors.filter((e) => e.includes("[mock] unhandled"));
    if (un.length) console.log(`[${this.name}] unhandled:\n` + un.join("\n"));
    console.log(`[${this.name}] ${this.n} frames -> ${out}`);
    if (!process.env.KEEP_FRAMES) fs.rmSync(this.dir, { recursive: true, force: true });
    return out;
  }
}
