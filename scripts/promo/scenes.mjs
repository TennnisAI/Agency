// Promo scenes. Usage: node scenes.mjs [name ...]   (no args = all)
import fs from "fs";
import path from "path";
import { Recorder } from "./rec.mjs";

// Setup click with no frames captured.
async function tap(r, loc, settle = 600) {
  const t = await r.target(loc);
  await r.page.mouse.click(t.x, t.y);
  await r.advance(settle);
}
const L = (r, sel, text) => r.page.locator(sel, text ? { hasText: text } : undefined).first();
const sidebarProject = (r, name) => r.page.getByText(name, { exact: true }).first();
const tab = (r, name) => r.page.getByRole("button", { name: new RegExp(`^\\W*${name}`) }).first();

const SCENES = {
  async overview(r) {
    await r.showCursor(1450, 900);
    r.cur.visible = false;
    await r.hold(1.2);
    r.cur.visible = true;
    await r.moveTo(L(r, ".tile", "Cursor-based pagination"), 1.2);
    await r.hold(0.8);
    await r.moveTo(L(r, ".tile", "Wire OpenTelemetry"), 0.9);
    await r.hold(0.8);
    await r.moveTo(L(r, ".tile", "Migrate config loading"), 0.9);
    await r.hold(1.2);
    await r.moveTo({ x: 1500, y: 960 }, 1.0);
    r.cur.visible = false;
    await r.hold(0.8);
    await r.still("overview");
  },

  async grid(r) {
    await tap(r, sidebarProject(r, "orbit-api"), 1200);
    await r.showCursor(1300, 860);
    await r.hold(2.2);
    await r.still("grid");
    await r.click(L(r, ".tile", "per-key rate"), { sec: 0.9 });
    await r.moveTo({ x: 1560, y: 700 }, 0.8);
    r.cur.visible = false;
    await r.hold(5.5);
    await r.still("focus");
  },

  async issues(r) {
    await tap(r, sidebarProject(r, "orbit-api"), 800);
    await tap(r, tab(r, "Issues"), 1200);
    await r.showCursor(1100, 820);
    await r.hold(1.0);
    await r.click(r.page.getByText("Support idempotency keys").first(), { sec: 0.8, after: 1.6 });
    await r.still("issue-detail");
    await r.click(r.page.locator('.issue-detail-head button[title="Start agent on this issue"]'), { sec: 0.8, after: 0.2 });
    await r.moveTo({ x: 1500, y: 640 }, 0.8);
    r.cur.visible = false;
    await r.hold(5.5);
    await r.still("issue-dispatched");
  },

  async race(r) {
    await tap(r, sidebarProject(r, "lumen-web"), 1200);
    await r.showCursor(1300, 700);
    await r.hold(0.8);
    await r.click(r.page.locator(".titlebar-right button, button").filter({ hasText: /^\+?\s*Agent/ }).last(), { sec: 0.8, after: 0.5 });
    await r.click(r.page.getByText("Race agents"), { sec: 0.6, after: 0.4 });
    await r.click(r.page.locator(".race-prompt"), { sec: 0.5, after: 0.1 });
    await r.type("Add keyboard shortcuts to the editor: bold, italic and link", { cps: 28 });
    await r.hold(0.3);
    await r.click(r.page.locator("label.race-agent", { hasText: "Gemini CLI" }).locator("input"), { sec: 0.6, after: 0.15 });
    await r.click(r.page.locator("label.race-agent", { hasText: "OpenCode" }).locator("input"), { sec: 0.5, after: 0.4 });
    await r.still("race-dialog");
    await r.click(r.page.getByText(/Start race/), { sec: 0.6, after: 0.2 });
    await r.moveTo({ x: 1560, y: 960 }, 0.8);
    r.cur.visible = false;
    await r.hold(6);
    await r.still("race-running");
  },

  async review(r) {
    await tap(r, sidebarProject(r, "orbit-api"), 900);
    await tap(r, L(r, ".tile", "flaky webhook"), 1000);
    await tap(r, tab(r, "Source Control"), 1500);
    await r.showCursor(1000, 760);
    await r.hold(0.8);
    await r.click(r.page.getByText("retry_test.go").first(), { sec: 0.8, after: 2.0 });
    await r.still("review-diff");
    await r.click(r.page.getByText("helpers_test.go").first(), { sec: 0.6, after: 1.6 });
    await r.click(r.page.locator("textarea").first(), { sec: 0.7, after: 0.1 });
    await r.type("drive the retry test from a fake clock", { cps: 30 });
    await r.click(r.page.locator("button", { hasText: /Commit/ }).first(), { sec: 0.6, after: 1.0 });
    await r.click(tab(r, "Agents"), { sec: 0.8, after: 0.8 });
    await r.click(r.page.locator(".btn-approve"), { sec: 0.8, after: 1.2 });
    await r.still("approve");
    await r.click(r.page.locator("button", { hasText: /Merge into main/ }), { sec: 0.7, after: 2.6 });
    await r.still("merged");
  },

  async docs(r) {
    await tap(r, sidebarProject(r, "orbit-api"), 800);
    await tap(r, tab(r, "Docs"), 1500);
    await r.showCursor(900, 700);
    await r.hold(0.6);
    await r.click(r.page.getByText("Architecture", { exact: true }).first(), { sec: 0.8, after: 1.8 });
    await r.moveTo(r.page.getByText("Job queue", { exact: true }).last(), 0.8);
    await r.hold(0.6);
    await r.moveTo(r.page.getByText("Rate limits", { exact: true }).last(), 0.6);
    await r.hold(0.6);
    await r.still("docs");
    await r.click(r.page.getByText("Rate limits", { exact: true }).last(), { sec: 0.2, after: 2.4 });
    await r.still("docs-linked");
  },

  async loop(r) {
    await tap(r, sidebarProject(r, "tidepool"), 1200);
    await r.showCursor(1300, 700);
    await r.hold(0.6);
    await r.click(r.page.locator("button").filter({ hasText: /^\+?\s*Agent/ }).last(), { sec: 0.8, after: 0.5 });
    await r.click(r.page.getByText("Loop agent"), { sec: 0.6, after: 0.4 });
    await r.click(r.page.locator("textarea.race-prompt"), { sec: 0.5, after: 0.1 });
    await r.type("Return 400, not 500, for readings with no station id. Run the tests and commit.", { cps: 34 });
    const sel = r.page.locator(".loop-fields select").first();
    await r.click(sel, { sec: 0.6, after: 0.1 });
    await sel.selectOption("codex");
    await r.hold(0.3);
    await r.click(r.page.locator("input.loop-check"), { sec: 0.6, after: 0.1 });
    await r.type("pnpm test", { cps: 18 });
    await r.hold(0.3);
    await r.still("loop-dialog");
    await r.click(r.page.getByText(/^Start loop/), { sec: 0.6, after: 0.2 });
    await r.moveTo({ x: 1560, y: 960 }, 0.8);
    r.cur.visible = false;
    // 11.5s stopped mid attempt 2, so the clip never showed the check pass.
    await r.hold(18);
    await r.still("loop-complete");
  },
};

const names = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(SCENES);
const unknown = names.filter((n) => !SCENES[n]);
if (unknown.length) {
  console.error(`unknown scene: ${unknown.join(", ")}. Scenes: ${Object.keys(SCENES).join(", ")}`);
  process.exit(2);
}
let failed = 0;
for (const name of names) {
  const r = new Recorder(name);
  await r.start();
  try {
    await SCENES[name](r);
    await r.finish();
  } catch (e) {
    // A scene that stops partway still has frames, and encoding them produced
    // a clip that looked finished and wasn't. Keep the old clip, save what the
    // page looked like, and fail the run.
    failed++;
    const shot = path.join(path.dirname(new URL(import.meta.url).pathname), "out", `fail-${name}.png`);
    fs.mkdirSync(path.dirname(shot), { recursive: true });
    await r.page.screenshot({ path: shot }).catch(() => {});
    await r.browser.close();
    console.error(`[${name}] failed: ${e.message}\n  page at failure: ${shot}`);
  }
}
process.exit(failed ? 1 : 0);
