// Renders banner.html for the GitHub org profile, once per theme, at 2x.
import { createRequire } from "module";
import fs from "fs";
import path from "path";
const require = createRequire(import.meta.url);
const { chromium } = require("playwright-core");

const HERE = path.dirname(new URL(import.meta.url).pathname);
const OUT = path.join(HERE, "..", "out", "profile");
fs.mkdirSync(OUT, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.CHROME || undefined });
const p = await browser.newPage({ viewport: { width: 1280, height: 440 }, deviceScaleFactor: 2 });
await p.goto("file://" + path.join(HERE, "banner.html"));
await p.evaluate(() => document.fonts.ready);
for (const theme of ["dark", "light"]) {
  await p.evaluate((t) => document.documentElement.setAttribute("data-theme", t), theme);
  await p.screenshot({ path: path.join(OUT, `banner-${theme}.png`) });
}
await browser.close();
