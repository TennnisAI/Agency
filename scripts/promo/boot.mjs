// Launches headless Chrome on the Vite dev server with the mock backend
// installed before any app code runs.
import { createRequire } from "module";
import path from "path";
const require = createRequire(import.meta.url);
const { chromium } = require("playwright-core");

const HERE = path.dirname(new URL(import.meta.url).pathname);
const MOCKS = ["core.js", "data.js", "git.js", "term.js", "scripts.js", "actions.js"];

// CHROME overrides the browser; otherwise playwright-core uses the one
// `pnpm --dir scripts/promo exec playwright-core install chromium` fetched.
export async function boot({ width = 1600, height = 1000, scale = 2, clock = false, headless = true } = {}) {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME || undefined,
    headless,
    args: ["--force-color-profile=srgb", "--hide-scrollbars", "--font-render-hinting=none"],
  });
  const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: scale, colorScheme: "dark", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)" });
  if (clock) {
    // Install, then pause: time moves only when the recorder advances it.
    await ctx.clock.install({ time: new Date("2026-10-07T14:32:00") });
    await ctx.clock.pauseAt(new Date("2026-10-07T14:32:01"));
  }
  for (const f of MOCKS) await ctx.addInitScript({ path: path.join(HERE, "mock", f) });
  const page = await ctx.newPage();
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errors.push(`[${m.type()}] ${m.text()}`); });
  page.on("pageerror", (e) => errors.push(`[pageerror] ${e.message}`));
  return { browser, ctx, page, errors };
}
