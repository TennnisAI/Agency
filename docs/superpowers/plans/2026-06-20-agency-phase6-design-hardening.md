# Agency Phase 6 Implementation Plan — Design Reskin + Hardening

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** (a) Apply the designers' Catppuccin Mocha design system (`docs/design-handoff/`) to Agency's existing surfaces, and (b) land the security/safety hardening flagged across earlier phases.

**Architecture:** Design is CSS + light markup restructuring (a custom title bar + status bar + tokenized theme); the data flow is unchanged. Hardening is small, testable changes in `agency-core`/`agency-app`/`tauri.conf.json`.

**Tech Stack:** React + TypeScript, CSS custom properties, xterm theming; Rust (`url` crate, git), Tauri v2 config.

## Global Constraints

- **Authority:** `docs/design-handoff/Agency-Design-Handoff.html` is the design spec; `docs/design-handoff/Agency v2.dc.html` is the reference mockup. Match its tokens, type scale, spacing, radii, and component specs exactly.
- **Tokens, not literals:** all colors/fonts go through CSS custom properties (the Theme setting implies palette-swapping later). Use the exact Catppuccin Mocha hexes from the handoff.
- **Fonts:** font-family stacks are `--sans: 'Geist', ui-sans-serif, system-ui, …` and `--mono: 'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, monospace`. Bundling the actual font files is out of scope (no network to fetch them) — system fallbacks render fine; leave a note.
- **Scope:** reskin the EXISTING surfaces (sidebar, task board / terminal, git panel, settings/merge modals) + add the custom title bar and status bar. **Defer (address when they arise):** multi-agent Grid/Focus views, foldable project tree, per-hunk staging, git history graph, ⌘K search, collapsible panels.
- **No first-party telemetry messaging in the chrome** (the design calls this out).
- **Local-only / zero first-party data collection** unchanged.
- Don't break existing behavior or tests. Frontend stays TS-strict clean; Rust stays warning-free; commit after each green task.

---

## File Structure

```
crates/agency-core/
├── src/merge.rs        # MODIFY: guard against dirty main working tree
└── tests/merge.rs      # MODIFY: dirty-tree guard test

crates/agency-app/
├── Cargo.toml          # MODIFY: add `url`
├── src/state.rs        # MODIFY: validate LM Studio URL in save_settings; inject provider env into resolver
├── tauri.conf.json     # MODIFY: strict CSP
└── tests/state.rs      # MODIFY: url-validation + resolver-env tests

ui/src/
├── theme.css           # NEW: Catppuccin Mocha tokens + base + xterm theme vars
├── styles.css          # MODIFY: rewrite component styles against tokens
├── main.tsx            # MODIFY: import theme.css
├── App.tsx             # MODIFY: title bar + status bar shell
├── components/*.tsx    # MODIFY: class/markup tweaks for the new chrome (status dots, badges, segmented nav)
└── lib/xtermTheme.ts   # NEW: xterm ITheme matching the palette
```

---

### Task 1: Security & safety hardening

**Files:**
- Modify: `crates/agency-core/src/merge.rs`, `crates/agency-core/tests/merge.rs`
- Modify: `crates/agency-app/Cargo.toml`, `crates/agency-app/src/state.rs`, `crates/agency-app/tests/state.rs`, `crates/agency-app/tauri.conf.json`

**Interfaces / changes:**
1. **Merge dirty-tree guard** — in `agency_core::merge::merge`, before `git checkout base`, verify the working tree is clean (`git status --porcelain` empty); if not, `bail!("repository has uncommitted changes; commit or stash them before merging")`.
2. **LM Studio URL validation** — add a `fn validate_provider_url(url: &str) -> anyhow::Result<()>` (in `state.rs`) using the `url` crate: parse it; require scheme `http`/`https`; if scheme is `http`, host must be `localhost`/`127.0.0.1`; reject embedded credentials (`url.username()` non-empty or password present). Call it at the start of `save_settings`; empty string is allowed (means "unset", keeps default behavior).
3. **Resolver provider env** — in `resolve_merge`, build the same provider env as `start_task` (ANTHROPIC_API_KEY if non-empty, OPENAI_BASE_URL, OPENAI_API_KEY) and prepend it to the resolver profile's env. (Factor the env-build into a small private helper `fn provider_env(&self) -> anyhow::Result<Vec<(String,String)>>` and use it in both `start_task` and `resolve_merge` — DRY.)
4. **Strict CSP** — in `tauri.conf.json`, set `app.security.csp` to `"default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'"` (allow inline styles for now since the app uses them; no remote origins).

- [ ] **Step 1: Failing tests**

Append to `crates/agency-core/tests/merge.rs`:

```rust
#[test]
fn merge_refuses_dirty_working_tree() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/z"]);
    std::fs::write(dir.path().join("new.txt"), "x\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "feat"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    // Dirty the main working tree.
    std::fs::write(dir.path().join("f.txt"), "dirty\n").unwrap();

    let err = merge::merge(dir.path(), "agent/z", "main").unwrap_err();
    assert!(err.to_string().contains("uncommitted changes"), "got: {err}");
}
```

Append to `crates/agency-app/tests/state.rs`:

```rust
#[test]
fn save_settings_rejects_bad_provider_url() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    // remote http (non-localhost) is rejected
    let bad = agency_app_lib::ProviderSettings {
        anthropic_api_key: "".into(),
        lm_studio_base_url: "http://evil.example.com/v1".into(),
    };
    assert!(state.save_settings(&bad).is_err());
    // localhost http and https are allowed
    for ok in ["http://localhost:1234/v1", "https://api.example.com/v1", ""] {
        let s = agency_app_lib::ProviderSettings {
            anthropic_api_key: "".into(),
            lm_studio_base_url: ok.into(),
        };
        assert!(state.save_settings(&s).is_ok(), "should accept {ok}");
    }
}
```

- [ ] **Step 2: Run → fail**

`cargo test -p agency-core --test merge` and `cargo test -p agency-app --test state` — both new tests FAIL.

- [ ] **Step 3: Implement**

`merge.rs` — at the top of `merge()`, before checkout:

```rust
    let dirty = git_ok(repo, &["status", "--porcelain"])?;
    if !dirty.trim().is_empty() {
        bail!("repository has uncommitted changes; commit or stash them before merging");
    }
```

`crates/agency-app/Cargo.toml` — add under `[dependencies]`:

```toml
url = "2"
```

`state.rs` — add the validator and call it in `save_settings`:

```rust
fn validate_provider_url(raw: &str) -> Result<()> {
    if raw.is_empty() {
        return Ok(());
    }
    let url = url::Url::parse(raw).map_err(|e| anyhow!("invalid URL: {e}"))?;
    match url.scheme() {
        "https" => {}
        "http" => {
            let host = url.host_str().unwrap_or("");
            if host != "localhost" && host != "127.0.0.1" {
                bail!("http is only allowed for localhost; use https for remote hosts");
            }
        }
        other => bail!("unsupported URL scheme: {other}"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URL must not contain embedded credentials");
    }
    Ok(())
}
```

In `save_settings`, first line:

```rust
        validate_provider_url(&s.lm_studio_base_url)?;
```

Resolver env DRY — add a helper and use it in both spawn paths:

```rust
    fn provider_env(&self) -> Result<Vec<(String, String)>> {
        let s = self.get_settings()?;
        let mut env = Vec::new();
        if !s.anthropic_api_key.is_empty() {
            env.push(("ANTHROPIC_API_KEY".into(), s.anthropic_api_key));
        }
        env.push(("OPENAI_BASE_URL".into(), s.lm_studio_base_url));
        env.push(("OPENAI_API_KEY".into(), "lm-studio".into()));
        Ok(env)
    }
```

In `start_task`, replace the inline provider-env block with `let mut env = self.provider_env()?;` then `env.extend(profile.env.iter().cloned()); profile.env = env;` (keep behavior identical). In `resolve_merge`, after looking up the resolver `profile`, do the same merge before `spawn_agent`, and remove the `// TODO(phase6)` comment.

`tauri.conf.json` — set the CSP:

```json
    "security": { "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'" }
```

- [ ] **Step 4: Run → pass**

`cargo test --workspace` — all pass, zero warnings. (The existing `start_task_injects_provider_env` test must still pass after the DRY refactor.)

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/merge.rs crates/agency-core/tests/merge.rs crates/agency-app/Cargo.toml crates/agency-app/src/state.rs crates/agency-app/tests/state.rs crates/agency-app/tauri.conf.json
git commit -m "Harden: dirty-tree merge guard, provider URL validation, resolver env, CSP"
```

---

### Task 2: Design tokens + app shell + xterm theme

**Files:**
- Create: `ui/src/theme.css`, `ui/src/lib/xtermTheme.ts`
- Modify: `ui/src/main.tsx` (import theme.css before styles.css), `ui/src/App.tsx` (title bar + status bar)

**Reference:** `docs/design-handoff/Agency-Design-Handoff.html` §04 Color, §05 Typography, §06 Spacing, §08 (title/status bars).

- [ ] **Step 1: Tokens**

Create `ui/src/theme.css` with the Catppuccin Mocha tokens (copy hexes verbatim from the handoff `:root`):

```css
:root {
  --base:#1e1e2e; --mantle:#181825; --crust:#11111b;
  --s0:#313244; --s1:#45475a; --s2:#585b70;
  --o0:#6c7086; --o1:#7f849c; --o2:#9399b2;
  --text:#cdd6f4; --sub1:#bac2de; --sub0:#a6adc8;
  --blue:#89b4fa; --lav:#b4befe; --mauve:#cba6f7; --pink:#f5c2e7;
  --green:#a6e3a1; --teal:#94e2d5; --yellow:#f9e2af; --peach:#fab387; --red:#f38ba8;
  --line:#25253a;
  --sans:'Geist',ui-sans-serif,system-ui,-apple-system,'Segoe UI',sans-serif;
  --mono:'JetBrains Mono',ui-monospace,'SF Mono',Menlo,monospace;
  /* semantic */
  --running:var(--green); --awaiting:var(--yellow); --review:var(--blue);
  --exited:var(--o0); --crashed:var(--red);
  --agent-claude:var(--peach); --agent-pi:var(--teal); --agent-hermes:var(--mauve);
  /* dims (handoff §06) */
  --titlebar-h:40px; --statusbar-h:27px; --sidebar-w:266px;
}
* { box-sizing: border-box; }
html, body, #root { height: 100%; }
body { margin: 0; background: var(--base); color: var(--text);
  font-family: var(--sans); -webkit-font-smoothing: antialiased; line-height: 1.5; }
code, .mono { font-family: var(--mono); }
```

- [ ] **Step 2: xterm theme**

Create `ui/src/lib/xtermTheme.ts` (handoff §07 Terminal — bg crust, prompt overlay0, success green, warn yellow, error red, paths blue, identifiers teal):

```ts
import type { ITheme } from "@xterm/xterm";

export const xtermTheme: ITheme = {
  background: "#11111b",
  foreground: "#cdd6f4",
  cursor: "#cdd6f4",
  selectionBackground: "#313244",
  black: "#45475a", red: "#f38ba8", green: "#a6e3a1", yellow: "#f9e2af",
  blue: "#89b4fa", magenta: "#cba6f7", cyan: "#94e2d5", white: "#bac2de",
  brightBlack: "#6c7086", brightRed: "#f38ba8", brightGreen: "#a6e3a1",
  brightYellow: "#f9e2af", brightBlue: "#89b4fa", brightMagenta: "#cba6f7",
  brightCyan: "#94e2d5", brightWhite: "#cdd6f4",
};
```

Wire it where terminals are created (`TerminalPane.tsx`, `MergeModal.tsx`): `new Terminal({ ..., theme: xtermTheme })`.

- [ ] **Step 3: Import order + shell**

In `ui/src/main.tsx`, import `./theme.css` BEFORE `./styles.css`.

In `ui/src/App.tsx`, wrap the app in a title bar + content + status bar shell (custom chrome per §08). Keep the existing sidebar/board/settings wiring inside the content region; add:
- a 40px **title bar**: left-panel toggle (visual only for now), app mark "Agency", spacer, and a Settings cog that opens the existing Settings overlay.
- a 27px **status bar** at the bottom: left = current project name (or "no project"), right = keyboard hints `⌘N new · ⌘G source · ⌘↵ approve` (advisory text; wiring shortcuts is deferred).

(Move the Settings button from the sidebar into the title bar.)

- [ ] **Step 4: Build**

`pnpm --dir ui build` (no TS errors) and `pnpm --dir ui test` (vitest green).

- [ ] **Step 5: Commit**

```bash
git add ui/src/theme.css ui/src/lib/xtermTheme.ts ui/src/main.tsx ui/src/App.tsx ui/src/components/TerminalPane.tsx ui/src/components/MergeModal.tsx
git commit -m "Add Catppuccin Mocha tokens, xterm theme, and app shell (title/status bars)"
```

---

### Task 3: Reskin sidebar, task board, status dots, badges

**Files:**
- Modify: `ui/src/styles.css`, `ui/src/components/ProjectSidebar.tsx`, `ui/src/components/TaskBoard.tsx`

**Reference:** handoff §07 (Button, Status dot, Agent badge, Segmented control), §03 (Task board).

- [ ] **Step 1: Reskin**

Rewrite the relevant `styles.css` rules against the tokens (replace the old literal hexes): `.app` grid with sidebar width `var(--sidebar-w)`; `.sidebar` on `var(--mantle)` with hairline `var(--line)`; primary `button` = `bg var(--blue); color var(--crust); weight 600; radius 8; padding 8px 13px; hover→var(--lav)`; secondary/ghost variants per §07; inputs/select/textarea on `var(--crust)`/`var(--mantle)` with `1px var(--s1)`.

Add a **status dot** component style and use it in the task status line: `.dot{width:9px;height:9px;border-radius:50%}` with modifier classes `.dot.running{background:var(--running);box-shadow:0 0 8px var(--running);animation:pulse 1.6s infinite}`, `.dot.awaiting{background:var(--awaiting)}`, `.dot.review{background:var(--review)}`, `.dot.exited{background:var(--exited)}`, `.dot.crashed{background:var(--crashed)}`, plus a `@keyframes pulse`. In `TaskBoard`, render a dot next to the status text, mapping the polled status string (`running`/`exited`/`crashed`/`idle`) to the modifier class.

Add an **agent badge** style `.badge{font-family:var(--mono);font-size:10.5px;border-radius:5px;padding:2px 7px}` with per-agent color classes (`.badge.claude{color:var(--agent-claude)}` etc.), and show the active profile as a badge in the running-task header (color by name: claude→claude, pi→pi, hermes→hermes, else default text).

Style the profile `<select>` and new-task textarea per §07.

- [ ] **Step 2: Build + test**

`pnpm --dir ui build`, `pnpm --dir ui test` — clean/green.

- [ ] **Step 3: Commit**

```bash
git add ui/src/styles.css ui/src/components/ProjectSidebar.tsx ui/src/components/TaskBoard.tsx
git commit -m "Reskin sidebar, task board, status dots, and agent badges"
```

---

### Task 4: Reskin git panel, diffs, and modals

**Files:**
- Modify: `ui/src/styles.css`, `ui/src/components/GitPanel.tsx`, `ui/src/components/Settings.tsx`, `ui/src/components/MergeModal.tsx`

**Reference:** handoff §07 (Diff line, File row, Modal), §03 (Source Control, Merge resolver).

- [ ] **Step 1: Diff coloring**

In `GitPanel`, render the diff `<pre>` with per-line classes instead of a single block: split the diff text into lines and wrap each in a span with a class by first char — `+`→`.diff-add`, `-`→`.diff-del`, `@`→`.diff-hunk`, else `.diff-ctx`. Add styles per §07: add = `color:var(--green);background:rgba(166,227,161,.10)`, del = `color:var(--red);background:rgba(243,139,168,.10)`, hunk = `color:var(--blue);background:rgba(137,180,250,.07)`, context = `color:var(--o1)`. Keep `white-space:pre` and `--mono`.

- [ ] **Step 2: File rows + git panel chrome**

Restyle `.git-*` against tokens: file status letter in semantic color (A→green, M→yellow, D→red, ?→overlay), file rows with hover-revealed stage/unstage, the panel on `var(--mantle)` with hairline. Status letter color: map the `index`/`worktree` code to a class.

- [ ] **Step 3: Modals**

Restyle `.settings-overlay`/modal containers per §07 Modal: backdrop `rgba(17,17,27,.66)` + `backdrop-filter: blur(6px)`; modal card `1px var(--s1)`, radius 15, shadow `0 24px 64px rgba(0,0,0,.5)`. Settings width 580px, merge modal 720px (per §03). Apply tokenized inputs/buttons inside. Keep all existing functionality (Settings fields, MergeModal flow) intact — markup changes are class/structure only.

- [ ] **Step 4: Build + test + final visual self-check**

`pnpm --dir ui build`, `pnpm --dir ui test` — clean/green. (Visual fidelity vs. the mockups is a human review item — note it.)

- [ ] **Step 5: Commit**

```bash
git add ui/src/styles.css ui/src/components/GitPanel.tsx ui/src/components/Settings.tsx ui/src/components/MergeModal.tsx
git commit -m "Reskin git panel, diff coloring, and modals to the design system"
```

---

## Self-Review

**Spec coverage:**
- Hardening (dirty-tree guard, URL validation, resolver env, CSP) → Task 1 (tested where deterministic). ✓
- Design tokens + fonts + shell + xterm theme → Task 2. ✓
- Sidebar/board/status dots/badges → Task 3. ✓
- Git panel/diff/modals → Task 4. ✓
- Deferred (documented, per the handoff's "address when they arise"): Grid/Focus views, project tree, per-hunk staging, history graph, ⌘K/keyboard wiring, collapsible panels, bundled font files, API-key keychain/masking.

**Placeholder scan:** No TBD; design tasks reference the authoritative handoff sections for exact values rather than restating every rule, and give the token foundation + the patterns to apply.

**Type/behavior consistency:** Hardening keeps `save_settings`/`start_task`/`resolve_merge` signatures unchanged; the `provider_env` helper is shared by both spawn paths. Reskin is CSS/markup only — no API or data-flow changes; existing tests stay green.

**Notes for the executor:**
- Visual fidelity to the mockups can't be unit-tested — after Task 4, the human reviews against `docs/design-handoff/Agency v2.dc.html`.
- Keep every functional element working; this phase changes look, not behavior.
- If a reskin needs a tiny markup change to hang a class, that's fine; do not refactor component logic.
