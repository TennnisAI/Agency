# Agency Phase 7c Implementation Plan — Command Palette, Shortcuts & Polish

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add a ⌘K command palette, global keyboard shortcuts, polished New-task & Merge modals, a Settings reskin, and collapse polish — realizing the full design handoff.

**Architecture:** Frontend-only. Lift the Agents view's `tab` + new-task/approve intents into the `RunStore` so shortcuts and the palette dispatch into one source of truth. A `useShortcuts` hook binds ⌘N/⌘G/⌘↵/⌘K; `CommandPalette` fuzzy-filters projects + runs. Modals get presentation upgrades (no behavior change).

**Tech Stack:** React + TypeScript, CSS tokens, Vitest.

## Global Constraints

- **Frontend-only:** NO Rust/backend changes. `cargo test --workspace` must stay green (untouched). No new npm dependencies.
- **Design authority:** `docs/design-handoff/Agency-Design-Handoff.html` + `docs/design-handoff/Agency v2.dc.html`. Use Catppuccin tokens in `ui/src/theme.css`; no literal hexes in components.
- **Shortcuts:** ⌘/Ctrl + `N` (new task), `G` (source control tab), `Enter` (approve→ on focused run), `K` (palette). Match `event.metaKey || event.ctrlKey`.
- **⌘K scope:** projects + the loaded runs (no repo file-content search). Simple case-insensitive substring + light scoring; no new dependency.
- **Behavior preservation:** the Merge modal flow and Settings fields are unchanged — 7c is presentation + the new palette/shortcuts.
- TS-strict clean; the one piece of real logic (palette filter) gets a vitest test. Frontend tasks build-verified; visual fidelity is a human check.

---

## File Structure

```
ui/src/
├── store/runs.tsx               # MODIFY: add tab/setTab, openNewTask/setOpenNewTask, approveRunId/setApproveRun
├── lib/paletteFilter.ts         # NEW: filterEntries(query, entries) pure helper
├── lib/paletteFilter.test.ts    # NEW: vitest
├── hooks/useShortcuts.ts        # NEW: global keydown → callbacks
├── components/CommandPalette.tsx # NEW: ⌘K overlay
├── App.tsx                      # MODIFY: mount shortcuts + palette; pass onOpenPalette to TitleBar
├── components/TitleBar.tsx      # MODIFY: search box → opens palette
├── components/AgentsView.tsx    # MODIFY: read tab/newOpen/approve from store
├── components/NewTaskForm.tsx   # MODIFY: base input + live worktree/branch preview
├── components/MergeModal.tsx    # MODIFY: step timeline header
├── components/Settings.tsx      # MODIFY: reskin (sectioned, tokens)
└── styles.css                   # MODIFY: palette/modal/settings/timeline styles
```

---

### Task 1: RunStore intents + palette filter helper

**Files:**
- Modify: `ui/src/store/runs.tsx`
- Create: `ui/src/lib/paletteFilter.ts`, `ui/src/lib/paletteFilter.test.ts`

**Interfaces:**
- `RunStore` (extend) gains: `tab: "agents" | "source"`, `setTab: (t) => void`; `openNewTask: boolean`, `setOpenNewTask: (b) => void`; `approveRunId: string | null`, `setApproveRun: (id) => void`. The existing `setSelectedProject` should also reset `tab` to `"agents"`.
- `lib/paletteFilter.ts`: `interface PaletteEntry { kind: "project" | "run"; id: string; projectId: string; label: string; sublabel: string }`; `filterEntries(query: string, entries: PaletteEntry[]): PaletteEntry[]` — case-insensitive: if query empty, return entries unchanged; else keep entries whose `label` or `sublabel` contains the query (lowercased), ordered so `label`-prefix matches come first, then `label`-substring, then `sublabel`-substring (stable within each group).

- [ ] **Step 1: Write the failing filter test**

Create `ui/src/lib/paletteFilter.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { filterEntries, PaletteEntry } from "./paletteFilter";

const E: PaletteEntry[] = [
  { kind: "project", id: "p1", projectId: "p1", label: "Senba", sublabel: "/repo/senba" },
  { kind: "run", id: "r1", projectId: "p1", label: "claude: fix the bug", sublabel: "agent/r1" },
  { kind: "run", id: "r2", projectId: "p1", label: "pi: add tests", sublabel: "agent/r2" },
];

describe("filterEntries", () => {
  it("returns all when query empty", () => {
    expect(filterEntries("", E)).toHaveLength(3);
  });
  it("matches case-insensitively on label", () => {
    const r = filterEntries("FIX", E);
    expect(r.map((e) => e.id)).toEqual(["r1"]);
  });
  it("matches on sublabel", () => {
    const r = filterEntries("senba", E);
    expect(r.map((e) => e.id)).toContain("p1");
  });
  it("orders label-prefix matches before substring matches", () => {
    const entries: PaletteEntry[] = [
      { kind: "run", id: "a", projectId: "p", label: "refactor parser", sublabel: "agent/a" },
      { kind: "run", id: "b", projectId: "p", label: "parser cleanup", sublabel: "agent/b" },
    ];
    const r = filterEntries("parser", entries);
    expect(r.map((e) => e.id)).toEqual(["b", "a"]); // "parser…" prefix first
  });
});
```

- [ ] **Step 2: Run → fail**

Run: `pnpm --dir ui test`
Expected: FAIL — cannot resolve `./paletteFilter`.

- [ ] **Step 3: Implement the filter**

Create `ui/src/lib/paletteFilter.ts`:

```ts
export interface PaletteEntry {
  kind: "project" | "run";
  id: string;
  projectId: string;
  label: string;
  sublabel: string;
}

export function filterEntries(query: string, entries: PaletteEntry[]): PaletteEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return entries;
  const scored: { e: PaletteEntry; score: number }[] = [];
  for (const e of entries) {
    const label = e.label.toLowerCase();
    const sub = e.sublabel.toLowerCase();
    let score = -1;
    if (label.startsWith(q)) score = 0;
    else if (label.includes(q)) score = 1;
    else if (sub.includes(q)) score = 2;
    if (score >= 0) scored.push({ e, score });
  }
  // Stable sort by score (lower = better); preserve input order within a score.
  return scored
    .map((s, i) => ({ ...s, i }))
    .sort((a, b) => (a.score - b.score) || (a.i - b.i))
    .map((s) => s.e);
}
```

- [ ] **Step 4: Run → pass**

Run: `pnpm --dir ui test`
Expected: PASS (4 new + existing b64 tests).

- [ ] **Step 5: Extend the store**

In `ui/src/store/runs.tsx`, extend the `RunStore` interface and provider:

```tsx
type Tab = "agents" | "source";
```

Add to the interface:

```tsx
  tab: Tab;
  setTab: (t: Tab) => void;
  openNewTask: boolean;
  setOpenNewTask: (b: boolean) => void;
  approveRunId: string | null;
  setApproveRun: (id: string | null) => void;
```

In the provider body, add state:

```tsx
  const [tab, setTab] = useState<Tab>("agents");
  const [openNewTask, setOpenNewTask] = useState(false);
  const [approveRunId, setApproveRun] = useState<string | null>(null);
```

In `setSelectedProject`, also `setTab("agents")`. Add the six new fields to the `Ctx.Provider value={{ … }}`.

- [ ] **Step 6: Build**

Run: `pnpm --dir ui build`
Expected: passes (new store fields unused until later tasks is fine; if tsc flags an unused, it won't — they're provided, not declared-unused).

- [ ] **Step 7: Commit**

```bash
git add ui/src/store/runs.tsx ui/src/lib/paletteFilter.ts ui/src/lib/paletteFilter.test.ts
git commit -m "Add palette filter helper and store UI intents"
```

---

### Task 2: useShortcuts hook + wire into App

**Files:**
- Create: `ui/src/hooks/useShortcuts.ts`
- Modify: `ui/src/App.tsx`

**Interfaces:**
- `useShortcuts(handlers: { onNewTask: () => void; onSource: () => void; onApprove: () => void; onPalette: () => void }): void` — adds a `window` `keydown` listener; on `(e.metaKey || e.ctrlKey)` and key `n`/`g`/`Enter`/`k`, prevents default and calls the matching handler. For `n` and `g`, ignore the event if the active element is an `<input>`/`<textarea>` (so typing in a field isn't hijacked); `k` and `Enter` fire regardless. Removes the listener on cleanup.

- [ ] **Step 1: Implement the hook**

Create `ui/src/hooks/useShortcuts.ts`:

```ts
import { useEffect } from "react";

interface Handlers {
  onNewTask: () => void;
  onSource: () => void;
  onApprove: () => void;
  onPalette: () => void;
}

function inEditable(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

export function useShortcuts(h: Handlers): void {
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        h.onPalette();
      } else if (key === "enter") {
        e.preventDefault();
        h.onApprove();
      } else if (key === "n" && !inEditable(e.target)) {
        e.preventDefault();
        h.onNewTask();
      } else if (key === "g" && !inEditable(e.target)) {
        e.preventDefault();
        h.onSource();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [h]);
}
```

- [ ] **Step 2: Wire into App**

In `ui/src/App.tsx`, inside `Shell` (which already calls `useRuns()`), pull the new store fields and mount shortcuts + palette. Add to the `useRuns()` destructure: `setOpenNewTask, setTab, focusedRunId, setApproveRun`. Add palette state `const [paletteOpen, setPaletteOpen] = useState(false);`. Then:

```tsx
  useShortcuts({
    onNewTask: () => setOpenNewTask(true),
    onSource: () => setTab("source"),
    onApprove: () => { if (focusedRunId) setApproveRun(focusedRunId); },
    onPalette: () => setPaletteOpen(true),
  });
```

Render `{paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} />}` (CommandPalette is Task 3 — this line will not resolve until Task 3; treat Tasks 2–3 as a pair and build green at Task 3). Pass `onOpenPalette={() => setPaletteOpen(true)}` to `<TitleBar>` (TitleBar updated in Task 3). Import `useShortcuts` and `CommandPalette`.

- [ ] **Step 3: Build (expected red on CommandPalette import until Task 3)**

Run: `pnpm --dir ui exec tsc --noEmit 2>&1 | head -20`
Expected: the only error is the missing `./components/CommandPalette` import — confirm `useShortcuts` itself is clean. Document in report.

- [ ] **Step 4: Commit**

```bash
git add ui/src/hooks/useShortcuts.ts ui/src/App.tsx
git commit -m "Add keyboard shortcuts hook and wire into App"
```

---

### Task 3: CommandPalette + TitleBar wiring (green)

**Files:**
- Create: `ui/src/components/CommandPalette.tsx`
- Modify: `ui/src/components/TitleBar.tsx`, `ui/src/styles.css`

**Interfaces:**
- `CommandPalette({ onClose })` — on mount, builds entries: every project (`listProjects`) as a `project` entry, plus the **selected project's** runs (`useRuns().runs`) as `run` entries. An input filters via `filterEntries`; ↑/↓ move a highlighted index (clamped), Enter activates the highlighted entry, Esc or backdrop click closes. Activating a `project` → `setSelectedProject(id)` + close; a `run` → `setSelectedProject(projectId)` + `setFocusedRun(id)` + `setView("focus")` + close. Autofocus the input on mount.
- `TitleBar` gains `onOpenPalette: () => void`; the search box becomes a `<button>` calling it.

- [ ] **Step 1: CommandPalette**

Create `ui/src/components/CommandPalette.tsx`:

```tsx
import { useEffect, useMemo, useState } from "react";
import { Project, listProjects } from "../api";
import { useRuns } from "../store/runs";
import { PaletteEntry, filterEntries } from "../lib/paletteFilter";

export default function CommandPalette({ onClose }: { onClose: () => void }) {
  const { runs, setSelectedProject, setFocusedRun, setView } = useRuns();
  const [projects, setProjects] = useState<Project[]>([]);
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);

  useEffect(() => {
    listProjects().then(setProjects).catch(() => setProjects([]));
  }, []);

  const entries: PaletteEntry[] = useMemo(() => {
    const ps: PaletteEntry[] = projects.map((p) => ({
      kind: "project",
      id: p.id,
      projectId: p.id,
      label: p.name,
      sublabel: p.repo_path,
    }));
    const rs: PaletteEntry[] = runs.map((r) => ({
      kind: "run",
      id: r.id,
      projectId: r.projectId,
      label: `${r.agent}: ${r.prompt || r.branch}`,
      sublabel: r.branch,
    }));
    return [...ps, ...rs];
  }, [projects, runs]);

  const filtered = useMemo(() => filterEntries(query, entries), [query, entries]);

  useEffect(() => {
    setHi(0);
  }, [query]);

  function activate(e: PaletteEntry) {
    if (e.kind === "project") {
      setSelectedProject(e.id);
    } else {
      setSelectedProject(e.projectId);
      setFocusedRun(e.id);
      setView("focus");
    }
    onClose();
  }

  function onKey(ev: React.KeyboardEvent) {
    if (ev.key === "Escape") {
      onClose();
    } else if (ev.key === "ArrowDown") {
      ev.preventDefault();
      setHi((h) => Math.min(h + 1, filtered.length - 1));
    } else if (ev.key === "ArrowUp") {
      ev.preventDefault();
      setHi((h) => Math.max(h - 1, 0));
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      if (filtered[hi]) activate(filtered[hi]);
    }
  }

  return (
    <div className="palette-overlay" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="palette-input"
          autoFocus
          placeholder="Search projects and agents…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
        />
        <ul className="palette-list">
          {filtered.length === 0 && <li className="palette-empty">No matches</li>}
          {filtered.map((e, i) => (
            <li
              key={`${e.kind}:${e.id}`}
              className={`palette-row ${i === hi ? "on" : ""}`}
              onMouseEnter={() => setHi(i)}
              onClick={() => activate(e)}
            >
              <span className={`palette-kind ${e.kind}`}>{e.kind === "project" ? "▢" : "▸"}</span>
              <span className="palette-label">{e.label}</span>
              <span className="palette-sub">{e.sublabel}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: TitleBar wiring**

In `ui/src/components/TitleBar.tsx`, add `onOpenPalette: () => void` to the props, and turn the search box into a button:

```tsx
      <button className="search-box no-drag" onClick={onOpenPalette}>
        <span>⌕</span>
        <span className="search-ph">Search projects, tasks, files…</span>
        <span className="kbd">⌘K</span>
      </button>
```

(Update the `TitleBar` props interface + the call in `App.tsx` to pass `onOpenPalette`.)

- [ ] **Step 3: Styles**

Append palette styles to `ui/src/styles.css` (tokens): `.palette-overlay{position:fixed;inset:0;background:rgba(17,17,27,.5);display:flex;justify-content:center;align-items:flex-start;padding-top:12vh;z-index:20}`, `.palette{width:560px;background:var(--mantle);border:1px solid var(--s1);border-radius:12px;box-shadow:0 24px 64px rgba(0,0,0,.5);overflow:hidden}`, `.palette-input{width:100%;background:var(--crust);border:none;border-bottom:1px solid var(--line);color:var(--text);padding:12px 14px;font-size:14px;outline:none}`, `.palette-list{list-style:none;margin:0;padding:6px;max-height:50vh;overflow:auto}`, `.palette-row{display:flex;gap:8px;align-items:baseline;padding:7px 9px;border-radius:6px;cursor:pointer}`, `.palette-row.on{background:var(--s0)}`, `.palette-kind{color:var(--o0)}`, `.palette-label{color:var(--text)}`, `.palette-sub{color:var(--o0);font-family:var(--mono);font-size:11px;margin-left:auto}`, `.palette-empty{color:var(--o0);padding:10px}`.

- [ ] **Step 4: Green build + test**

Run: `pnpm --dir ui build` (tsc + vite, no errors) and `pnpm --dir ui test` (vitest green incl. palette filter), `cargo test --workspace` (unchanged green).

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/CommandPalette.tsx ui/src/components/TitleBar.tsx ui/src/styles.css ui/src/App.tsx
git commit -m "Add command palette and wire the title-bar search to it"
```

---

### Task 4: AgentsView reads store intents; New-task modal preview; Merge timeline

**Files:**
- Modify: `ui/src/components/AgentsView.tsx`, `ui/src/components/NewTaskForm.tsx`, `ui/src/components/MergeModal.tsx`, `ui/src/styles.css`

**Interfaces:**
- `AgentsView` reads `tab/setTab`, `openNewTask/setOpenNewTask`, `approveRunId/setApproveRun` from `useRuns()` instead of local state, so ⌘N/⌘G/⌘↵ drive it. Keep `review` local. When `approveRunId === focusedRunId` (and set), open `MergeModal` for it and clear `approveRunId` on close.
- `NewTaskForm` gains a base-branch input (default `"HEAD"`) and a live preview line: `worktree: <repoName>/.agency/worktrees/<new>` and `branch: agent/<new>`. Use the project's `repo_path` basename for display. Present in the modal shell.
- `MergeModal` gains a step timeline header `Checkout · Rebase · Resolve · Summary` where the active step derives from its state (`!outcome` → Checkout/Rebase; `outcome.kind==="conflicts" && resolving` → Resolve; `outcome.kind==="clean" || resolverDone` → Summary).

- [ ] **Step 1: AgentsView from store**

In `ui/src/components/AgentsView.tsx`, replace local `tab`/`newOpen` with store values:

```tsx
  const { runs, view, setView, focusedRunId, tab, setTab, openNewTask, setOpenNewTask, approveRunId, setApproveRun } = useRuns();
  const [review, setReview] = useState(false);
```

Replace `setNewOpen(true)`/`newOpen` usages with `setOpenNewTask(true)`/`openNewTask`, and the NewTaskForm overlay `onDone={() => setOpenNewTask(false)}`. After the existing content, add an approve-driven modal:

```tsx
      {approveRunId && approveRunId === focusedRunId && (
        <MergeModal taskId={approveRunId} onClose={() => setApproveRun(null)} />
      )}
```

Import `MergeModal`. (AgentFocus also opens MergeModal via its own button — that's fine; this adds the ⌘↵ path. To avoid two modals, gate AgentFocus's local merge OR accept that ⌘↵ is the global path; simplest: keep both but ensure each has its own state — they won't both be open because the user triggers one path at a time.)

- [ ] **Step 2: NewTaskForm preview + base**

In `ui/src/components/NewTaskForm.tsx`, add a base state and preview. Add `const [base, setBase] = useState("HEAD");` and pass it to `createRun(project.id, prompt, agent, base)`. Add a base input and a preview block:

```tsx
      <label>Base branch</label>
      <input value={base} onChange={(e) => setBase(e.target.value)} />
      <div className="nt-preview">
        <div>worktree: <code>{project.repo_path.split("/").filter(Boolean).pop()}/.agency/worktrees/&lt;new&gt;</code></div>
        <div>branch: <code>agent/&lt;new&gt;</code></div>
      </div>
```

- [ ] **Step 3: MergeModal timeline**

In `ui/src/components/MergeModal.tsx`, add a timeline header at the top of the modal body. Compute the active step from existing state:

```tsx
  const step = !outcome ? "rebase" : outcome.kind === "conflicts" ? (resolverDone ? "summary" : resolving ? "resolve" : "rebase") : "summary";
  const steps: { key: string; label: string }[] = [
    { key: "checkout", label: "Checkout" },
    { key: "rebase", label: "Rebase" },
    { key: "resolve", label: "Resolve" },
    { key: "summary", label: "Summary" },
  ];
```

Render (just under the header):

```tsx
        <div className="merge-timeline">
          {steps.map((s) => (
            <span key={s.key} className={`mt-step ${s.key === step ? "on" : ""}`}>{s.label}</span>
          ))}
        </div>
```

(`outcome`, `resolving`, `resolverDone` are existing state in MergeModal from Phase 5/7a — reference them; if a name differs, use the actual one. Do not change the merge flow.)

- [ ] **Step 4: Styles** — append `.nt-preview{font-size:12px;color:var(--o0);…}`, `.merge-timeline{display:flex;gap:8px;…}`, `.mt-step{color:var(--o0);font-size:12px}`, `.mt-step.on{color:var(--blue);font-weight:600}` to `ui/src/styles.css` (tokens).

- [ ] **Step 5: Build + test green**

Run: `pnpm --dir ui build`, `pnpm --dir ui test`, `cargo test --workspace`. All green.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/AgentsView.tsx ui/src/components/NewTaskForm.tsx ui/src/components/MergeModal.tsx ui/src/styles.css
git commit -m "Drive Agents view from store intents; new-task preview; merge timeline"
```

---

### Task 5: Settings reskin + collapse polish; final green

**Files:**
- Modify: `ui/src/components/Settings.tsx`, `ui/src/styles.css`
- Possibly: `ui/src/App.tsx` (sidebar reflow), `ui/src/components/AgentFocus.tsx` (rail reflow)

**Interfaces:**
- `Settings` restyled to the design system — keep ALL fields/behavior (providers, profiles), apply sectioned layout + tokenized inputs + a clear header with close. No logic change.
- Collapse polish: the sidebar (App's `sidebarOpen`) and the focus rail (AgentFocus `railOpen`) already toggle; ensure neighbors reflow to full width (CSS) and the controls read clearly. CSS-only where possible.

- [ ] **Step 1: Settings reskin**

In `ui/src/components/Settings.tsx`, keep the component logic; update the JSX class structure to sectioned cards and ensure inputs use the tokenized classes. The existing modal overlay class (`.settings-overlay`) and field handlers stay. (If `Settings` already uses `.settings`/`.settings-head` from Phase 6, refine spacing/sections; the key is tokens + the sectioned look from the mockup §Settings, no field changes.)

- [ ] **Step 2: Collapse reflow CSS**

In `ui/src/styles.css`, ensure `.body` lets the main content take full width when `.tree` is absent (it already flexes), and the focus `.rail` collapse leaves a slim stub (`.rail-stub`) without leaving a gap. Add/adjust: `.focus-main{flex:1;min-width:0}` (so it grows when the rail collapses), `.rail-stub{flex-shrink:0}`. Verify the title-bar toggle hides the tree (App already conditionally renders it) and content reflows.

- [ ] **Step 3: Final green + visual self-check**

Run: `pnpm --dir ui exec tsc --noEmit` (0), `pnpm --dir ui build` (writes dist), `pnpm --dir ui test` (vitest green), `cargo test --workspace` (green). Document outputs.

- [ ] **Step 4: Manual smoke (human, record)**

`cargo tauri dev`: press ⌘K → palette filters projects/runs → Enter jumps; ⌘N opens New task (with preview); ⌘G → Source Control; focus a run, ⌘↵ → merge modal with the step timeline; open Settings (reskinned); toggle the sidebar + focus rail (content reflows). Visual fidelity vs `Agency v2.dc.html` is a human check.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/Settings.tsx ui/src/styles.css ui/src/App.tsx ui/src/components/AgentFocus.tsx
git commit -m "Reskin settings and polish collapse reflow"
```

---

## Self-Review

**Spec coverage (7c):**
- ⌘K palette (projects + runs, fuzzy) → Tasks 1 (filter) + 3 (component). ✓
- Shortcuts ⌘N/⌘G/⌘↵/⌘K → Task 2 (hook) + 4 (AgentsView reads intents). ✓
- New-task modal preview + base → Task 4. ✓
- Merge modal step timeline → Task 4. ✓
- Settings reskin → Task 5. ✓
- Collapse polish → Task 5. ✓
- Frontend-only; cargo untouched → all tasks. ✓
- Deferred (documented in spec): repo file-content search in ⌘K, chord shortcuts, animation library.

**Placeholder scan:** Every step has complete code or precise CSS selector/value lists (styling steps name the exact classes + token values to add, consistent with prior phases). Tasks 2–3 are a declared pair (App references CommandPalette before it exists in Task 2; green at Task 3) — explicitly framed, not a gap.

**Type consistency:** new store fields (`tab/setTab`, `openNewTask/setOpenNewTask`, `approveRunId/setApproveRun`) are declared in the interface (Task 1) and consumed in App (Task 2) + AgentsView (Task 4). `PaletteEntry`/`filterEntries` identical across helper, test, and CommandPalette. `useShortcuts` handler names match App's wiring. `createRun(project.id, prompt, agent, base)` matches the existing 4-arg signature (base added). MergeModal references its existing `outcome`/`resolving`/`resolverDone` state (Task 4 notes to use the actual names).

**Notes for the executor:**
- No backend changes — if you feel the urge to touch Rust, stop; 7c is frontend-only.
- MergeModal's internal state names (`outcome`/`resolving`/`resolverDone`) are from Phase 5/7a — open the file and use the real identifiers; the timeline derivation adapts to them.
- Visual fidelity vs the mockup is the human review item, as in 7a/7b.
