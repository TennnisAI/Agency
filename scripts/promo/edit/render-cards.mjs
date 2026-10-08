// Renders the tour's title cards and scene frames from cards.html: the intro
// and end cards frame by frame (the lanes draw on), and per scene a backdrop
// with its caption, a rounded window mask and a hairline border overlay.
import { createRequire } from "module";
import fs from "fs";
import path from "path";
const require = createRequire(import.meta.url);
const { chromium } = require("playwright-core");

const HERE = path.dirname(new URL(import.meta.url).pathname);
const OUT = path.join(HERE, "..", "out", "edit");
export const SCENES = {
  overview: ["blue", "every project", "See every agent in every project at once"],
  issues: ["mauve", "dispatch", "Hand an issue to an agent. It gets its own branch."],
  race: ["green", "race", "One prompt, four agents. Keep the best branch."],
  loop: ["peach", "loop", "Loop an agent until the tests pass"],
  review: ["teal", "review and merge", "Read the diff, then merge it into main"],
};

fs.mkdirSync(OUT, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.CHROME || undefined });
const p = await browser.newPage({ viewport: { width: 2560, height: 1440 } });
await p.goto("file://" + path.join(HERE, "cards.html"));
await p.evaluate(() => document.fonts.ready);
for (const [name, dur] of [["intro", 3.6], ["end", 4.6]]) {
  const dir = path.join(OUT, "frames-" + name);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  for (let i = 0; i < Math.round(dur * 30); i++) {
    await p.evaluate(([f, t]) => window[f](t), [name, i / 30]);
    await p.screenshot({ path: path.join(dir, String(i).padStart(4, "0") + ".png") });
  }
}
for (const [k, v] of Object.entries(SCENES)) {
  await p.evaluate((v) => window.scene(...v), v);
  await p.screenshot({ path: path.join(OUT, `bg-${k}.png`) });
}
await p.evaluate(() => window.layer("mask"));
await p.screenshot({ path: path.join(OUT, "mask.png") });
// Both html and body have to go transparent: with only body cleared, the
// overlay was an opaque sheet that painted over every scene.
await p.evaluate(() => { window.layer("border"); document.body.style.background = "transparent"; document.documentElement.style.background = "transparent"; });
await p.screenshot({ path: path.join(OUT, "border.png"), omitBackground: true });
await browser.close();
