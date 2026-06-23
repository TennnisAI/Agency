# Agency — Review Pane Stacking + UX Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show git Changes + History together (VSCode-style stacked, movable divider, foldable history), fix the projects-pane `+` button, add an agent `+` to the focus rail, give the folded rail a vertical "AGENTS · N" spine, and auto-name agents with an LLM-summarized title from their first prompt.

**Architecture:** Frontend is React + xterm.js (`ui/`). The git pane is one component (`GitPanel`) with two layout branches that both gain a shared stacked `GitSections`. The agent-title feature adds a mutable `title` column in the SQLite registry (`agency-core`), a Tauri command that generates the title asynchronously on a worker thread (mirroring the existing notifier thread), and a best-effort terminal-input capture in the UI.

**Tech Stack:** React 18 + TypeScript, xterm.js, Vitest; Rust (Tauri v2), rusqlite, reqwest (new, for the LLM call).

## Global Constraints

- UI typecheck/test: `cd ui && npx tsc --noEmit && npx vitest run`.
- Rust build/test: `export PATH="$HOME/.cargo/bin:$PATH"` first, then `cargo build` / `cargo test` from repo root.
- Serde DTOs crossing into TS use `#[serde(rename_all = "camelCase")]`.
- `localStorage` pane keys are prefixed `pane:` (handled inside `usePaneWidth` helpers).
- Run `prompt`/`id`/`branch` are immutable — never repurpose them; the new title is a separate nullable field.
- No AI attribution in commit messages.
- Display precedence for a run's name is always `title || prompt || branch`, via the shared `runName()` helper.
- Anthropic title model id: `claude-haiku-4-5-20251001`.

---

### Task 1: Add orientation to `Resizer`

**Files:**
- Modify: `ui/src/components/Resizer.tsx`
- Test: `ui/src/components/Resizer.test.tsx` (create)

**Interfaces:**
- Produces: `Resizer` accepts `orientation?: "vertical" | "horizontal"` (default `"vertical"`). Vertical drags width via `clientX`; horizontal drags height via `clientY`. Existing props (`width`, `min`, `max`, `onChange`, `side`) unchanged; for horizontal, `onChange` receives the new height and `side="left"` semantics mean "drag-down grows".

- [ ] **Step 1: Write the failing test**

```tsx
// ui/src/components/Resizer.test.tsx
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import Resizer from "./Resizer";

describe("Resizer", () => {
  it("defaults to a vertical separator", () => {
    const { container } = render(<Resizer width={100} min={0} max={200} onChange={() => {}} />);
    const el = container.querySelector(".resizer")!;
    expect(el.className).not.toContain("horizontal");
    expect(el.getAttribute("aria-orientation")).toBe("vertical");
  });

  it("renders a horizontal separator when orientation is horizontal", () => {
    const { container } = render(
      <Resizer width={100} min={0} max={200} onChange={() => {}} orientation="horizontal" />,
    );
    const el = container.querySelector(".resizer")!;
    expect(el.className).toContain("horizontal");
    expect(el.getAttribute("aria-orientation")).toBe("horizontal");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npx vitest run src/components/Resizer.test.tsx`
Expected: FAIL — current `Resizer` always renders `aria-orientation="vertical"` and no `horizontal` class.

- [ ] **Step 3: Implement orientation support**

Replace the contents of `ui/src/components/Resizer.tsx` with:

```tsx
import { useCallback } from "react";

export default function Resizer({
  width,
  min,
  max,
  onChange,
  side = "left",
  orientation = "vertical",
}: {
  width: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
  side?: "left" | "right";
  orientation?: "vertical" | "horizontal";
}) {
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const horizontal = orientation === "horizontal";
      const start = horizontal ? e.clientY : e.clientX;
      const startW = width;
      const move = (ev: PointerEvent) => {
        const pos = horizontal ? ev.clientY : ev.clientX;
        const delta = pos - start;
        onChange(side === "left" ? startW + delta : startW - delta);
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        document.body.style.cursor = "";
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      document.body.style.cursor = horizontal ? "row-resize" : "col-resize";
    },
    [width, min, max, onChange, side, orientation],
  );

  return (
    <div
      className={`resizer${orientation === "horizontal" ? " horizontal" : ""}`}
      onPointerDown={onPointerDown}
      role="separator"
      aria-orientation={orientation}
    />
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npx vitest run src/components/Resizer.test.tsx`
Expected: PASS (both cases).

- [ ] **Step 5: Typecheck and commit**

```bash
cd ui && npx tsc --noEmit
git add ui/src/components/Resizer.tsx ui/src/components/Resizer.test.tsx
git commit -m "feat(ui): add horizontal orientation to Resizer"
```

---

### Task 2: Fold-state persistence helpers

**Files:**
- Modify: `ui/src/hooks/usePaneWidth.ts`
- Test: `ui/src/hooks/usePaneWidth.test.ts:71` (append new describe block)

**Interfaces:**
- Produces:
  - `loadFold(storage: Pick<Storage, "getItem">, key: string, def: boolean): boolean`
  - `saveFold(storage: Pick<Storage, "setItem">, key: string, folded: boolean): void`
  - Both use the `pane:` key prefix, matching `loadWidth`/`saveWidth`. Stored as `"1"` (folded) / `"0"` (expanded).

- [ ] **Step 1: Write the failing test**

Append to `ui/src/hooks/usePaneWidth.test.ts`:

```ts
import { loadFold, saveFold } from "./usePaneWidth";

describe("fold state", () => {
  it("returns the default when nothing is stored", () => {
    const storage = makeStorage();
    expect(loadFold(storage, "git-history-folded", false)).toBe(false);
    expect(loadFold(storage, "git-history-folded", true)).toBe(true);
  });

  it("round-trips a saved folded value", () => {
    const storage = makeStorage();
    saveFold(storage, "git-history-folded", true);
    expect(storage.getItem("pane:git-history-folded")).toBe("1");
    expect(loadFold(storage, "git-history-folded", false)).toBe(true);
  });

  it("round-trips a saved expanded value", () => {
    const storage = makeStorage();
    saveFold(storage, "git-history-folded", false);
    expect(storage.getItem("pane:git-history-folded")).toBe("0");
    expect(loadFold(storage, "git-history-folded", true)).toBe(false);
  });

  it("falls back to default for an unrecognized stored value", () => {
    const storage = makeStorage();
    storage.setItem("pane:git-history-folded", "weird");
    expect(loadFold(storage, "git-history-folded", true)).toBe(true);
  });
});
```

Note: the existing `makeStorage()` only types `getItem`/`setItem`; that satisfies both helper signatures.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npx vitest run src/hooks/usePaneWidth.test.ts`
Expected: FAIL — `loadFold`/`saveFold` not exported.

- [ ] **Step 3: Implement the helpers**

Append to `ui/src/hooks/usePaneWidth.ts`:

```ts
export function loadFold(
  storage: Pick<Storage, "getItem">,
  key: string,
  def: boolean,
): boolean {
  const raw = storage.getItem("pane:" + key);
  if (raw === "1") return true;
  if (raw === "0") return false;
  return def;
}

export function saveFold(
  storage: Pick<Storage, "setItem">,
  key: string,
  folded: boolean,
): void {
  storage.setItem("pane:" + key, folded ? "1" : "0");
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npx vitest run src/hooks/usePaneWidth.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/usePaneWidth.ts ui/src/hooks/usePaneWidth.test.ts
git commit -m "feat(ui): add fold-state localStorage helpers"
```

---

### Task 3: `GitSections` stacked component + GitPanel rewrite

**Files:**
- Create: `ui/src/components/git/GitSections.tsx`
- Modify: `ui/src/components/git/GitPanel.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `Resizer` (`orientation="horizontal"`, Task 1); `usePaneWidth` + `loadFold`/`saveFold` (Task 2).
- Produces: `GitSections({ changesPanel, historyPanel }: { changesPanel: React.ReactNode; historyPanel: React.ReactNode })` — renders a Changes section (flex), a horizontal `Resizer`, and a foldable History section whose expanded height persists under `pane:git-history-h` and whose fold state persists under `pane:git-history-folded`.

- [ ] **Step 1: Create `GitSections.tsx`**

```tsx
import { useState } from "react";
import Resizer from "../Resizer";
import { usePaneWidth, loadFold, saveFold } from "../../hooks/usePaneWidth";

const HISTORY_MIN = 120;
const HISTORY_MAX = 800;
const FOLD_KEY = "git-history-folded";

export default function GitSections({
  changesPanel,
  historyPanel,
}: {
  changesPanel: React.ReactNode;
  historyPanel: React.ReactNode;
}) {
  const history = usePaneWidth("git-history-h", 220, HISTORY_MIN, HISTORY_MAX);
  const [folded, setFolded] = useState<boolean>(() =>
    typeof localStorage === "undefined" ? false : loadFold(localStorage, FOLD_KEY, false),
  );

  const toggleFold = () => {
    setFolded((f) => {
      const next = !f;
      try {
        if (typeof localStorage !== "undefined") saveFold(localStorage, FOLD_KEY, next);
      } catch {
        /* ignore quota / security errors */
      }
      return next;
    });
  };

  return (
    <div className="git-sections">
      <div className="git-section git-section-changes">
        <div className="git-section-head static">Changes</div>
        <div className="git-section-body">{changesPanel}</div>
      </div>

      {!folded && <Resizer width={history.width} min={HISTORY_MIN} max={HISTORY_MAX} onChange={history.setWidth} orientation="horizontal" />}

      <div
        className={`git-section git-section-history ${folded ? "folded" : ""}`}
        style={folded ? undefined : { height: history.width, flexShrink: 0 }}
      >
        <button className="git-section-head" onClick={toggleFold} aria-expanded={!folded}>
          <span className="chev">{folded ? "▸" : "▾"}</span> History
        </button>
        {!folded && <div className="git-section-body">{historyPanel}</div>}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Rewrite `GitPanel.tsx` to use `GitSections` (remove tabs)**

Replace the whole file `ui/src/components/git/GitPanel.tsx` with:

```tsx
import { useCallback, useEffect, useState } from "react";
import { FileChange, BranchInfo, HistoryItem, gitStatus, gitBranchInfo, gitPush } from "../../api";
import ChangesPanel from "./ChangesPanel";
import HistoryPanel from "./HistoryPanel";
import CommitDetail from "./CommitDetail";
import DiffViewer from "./DiffViewer";
import ReviewComments from "./ReviewComments";
import BranchBar from "./BranchBar";
import GitSections from "./GitSections";

type Selection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

export default function GitPanel({ taskId, layout }: { taskId: string; layout: "compact" | "full" }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [error, setError] = useState("");
  const [sel, setSel] = useState<Selection>(null);
  const [commentsKey, setCommentsKey] = useState(0);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setBranch(await gitBranchInfo(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => { setSel(null); }, [taskId]);
  useEffect(() => {
    const id = setInterval(() => refresh(), 2000);
    return () => clearInterval(id);
  }, [refresh]);

  const act = useCallback((fn: () => Promise<unknown>) => {
    (async () => {
      try { await fn(); setError(""); } catch (e) { setError(String(e)); }
      await refresh();
    })();
  }, [refresh]);

  const onSelectFile = (path: string, group: "index" | "workingTree" | "merge" | "untracked") =>
    setSel({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} onAct={act}
      selectedPath={sel?.kind === "file" ? sel.path : null} onSelectFile={onSelectFile} />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null}
      selectedHash={sel?.kind === "commit" ? sel.item.hash : null}
      onSelectCommit={(item) => setSel({ kind: "commit", item })} />
  );
  const sections = <GitSections changesPanel={changesPanel} historyPanel={historyPanel} />;

  if (layout === "compact") {
    return (
      <aside className="git-panel compact">
        {error && <div className="git-error">{error}</div>}
        {sections}
        <ReviewComments key={commentsKey} taskId={taskId} />
        {sel?.kind === "file" && (
          <div className="git-compact-diff">
            <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} />
          </div>
        )}
        {sel?.kind === "commit" && <div className="git-compact-diff"><CommitDetail taskId={taskId} item={sel.item} /></div>}
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      <BranchBar info={branch} onSync={() => act(() => gitPush(taskId))} onRefresh={refresh} />
      {error && <div className="git-error">{error}</div>}
      <div className="git-full-body">
        <div className="git-full-left">
          {sections}
          <ReviewComments key={commentsKey} taskId={taskId} />
        </div>
        <div className="git-full-right">
          {sel?.kind === "file" && <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} />}
          {sel?.kind === "commit" && <CommitDetail taskId={taskId} item={sel.item} />}
          {!sel && <div className="diff-empty">Select a file or commit.</div>}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Add the CSS**

In `ui/src/styles.css`, add the horizontal-resizer rule right after the existing `.resizer:hover` rule (around line 642):

```css
.resizer.horizontal { width: auto; height: 5px; cursor: row-resize; }
```

Then append a new block (e.g. after the existing `.git-history` rule near line 707):

```css
/* ── Git stacked sections (Changes over History) ─────────────── */
.git-sections { display: flex; flex-direction: column; flex: 1; min-height: 0; overflow: hidden; }
.git-section { display: flex; flex-direction: column; min-height: 0; }
.git-section-changes { flex: 1; min-height: 0; }
.git-section-body { overflow: auto; flex: 1; min-height: 0; }
.git-section-head {
  display: flex; align-items: center; gap: 6px; width: 100%;
  padding: 6px 10px; border-bottom: 1px solid var(--line);
  background: none; border-left: none; border-right: none; border-top: none;
  color: var(--o1); font-size: 11px; font-weight: 600; letter-spacing: .08em;
  text-transform: uppercase; cursor: pointer; text-align: left;
}
.git-section-head.static { cursor: default; }
.git-section-head .chev { font-size: 10px; }
.git-section-history { border-top: 1px solid var(--line); }
.git-section-history.folded { flex-shrink: 0; }
```

Note: `.git-tabs` rules (lines ~730-732) are now unused by `GitPanel`. Leave them only if another component references them; otherwise remove those three lines.

- [ ] **Step 4: Verify no other component uses the git tabs**

Run: `cd ui && grep -rn "git-tabs" src`
Expected: only matches inside `styles.css` (now removable). If `GitPanel.tsx` still shows up, the rewrite missed something — fix before continuing. Remove the `.git-tabs*` rules if `grep` shows no `.tsx` usage.

- [ ] **Step 5: Typecheck + run full UI test suite**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS, no type errors.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/git/GitSections.tsx ui/src/components/git/GitPanel.tsx ui/src/styles.css
git commit -m "feat(ui): stack git Changes over foldable History with movable divider"
```

---

### Task 4: Keep the projects-pane `+` button visible

**Files:**
- Modify: `ui/src/styles.css`

**Interfaces:** none (CSS only).

- [ ] **Step 1: Read the current rules**

Run: `cd ui && grep -n "\.tree {" -A 8 src/styles.css | head -20`
Expected: confirms `.tree` has `min-width: var(--sidebar-w)` and `overflow: hidden`, and `.tree-head .eyebrow` exists.

- [ ] **Step 2: Update `.tree` min-width**

In `ui/src/styles.css`, in the `.tree { … }` block (around line 264), change:

```css
  min-width: var(--sidebar-w);
```

to:

```css
  min-width: 0;
```

- [ ] **Step 3: Truncate the label and pin the button**

In `ui/src/styles.css`, replace the `.tree-head .eyebrow` rule (around line 275) with:

```css
.tree-head .eyebrow {
  font-size: 10.5px; font-weight: 600; letter-spacing: .13em; color: var(--o0);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
```

And add a `flex-shrink` guard to the existing `.icon-add` rule (line ~108) by appending `flex-shrink: 0;` inside its declaration block:

```css
.icon-add { width: 22px; height: 22px; border-radius: 6px; background: var(--base); border: 1px solid var(--s0); color: var(--text); cursor: pointer; font-size: 14px; line-height: 1; display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
```

- [ ] **Step 4: Verify the app builds and the rule is present**

Run: `cd ui && npx tsc --noEmit && grep -n "min-width: 0;" src/styles.css`
Expected: tsc passes; grep shows the new `.tree` min-width.

Manual check (during the later run/verify pass): drag the sidebar to its narrowest — the "PROJECTS" label ellipsizes and the `+` stays visible.

- [ ] **Step 5: Commit**

```bash
git add ui/src/styles.css
git commit -m "fix(ui): keep projects-pane + button visible at narrow widths"
```

---

### Task 5: Extract `AgentAddMenu` and use it in `AgentsView`

**Files:**
- Create: `ui/src/components/AgentAddMenu.tsx`
- Modify: `ui/src/components/AgentsView.tsx`

**Interfaces:**
- Produces: `AgentAddMenu({ onSpawn, variant }: { onSpawn: (agentId: string) => void; variant?: "button" | "icon" })` — renders the trigger plus the `AGENT_TYPES` dropdown; calls `onSpawn(agentId)` and closes on selection. `variant="button"` (default) shows `+ Agent ▾`; `variant="icon"` shows a compact `+`.

- [ ] **Step 1: Create `AgentAddMenu.tsx`**

```tsx
import { useState } from "react";
import { AGENT_TYPES } from "../agents";

export default function AgentAddMenu({
  onSpawn,
  variant = "button",
}: {
  onSpawn: (agentId: string) => void;
  variant?: "button" | "icon";
}) {
  const [open, setOpen] = useState(false);
  const choose = (id: string) => { setOpen(false); onSpawn(id); };

  return (
    <div className="agent-add">
      {variant === "icon" ? (
        <button className="icon-btn" title="Add agent" onClick={() => setOpen((o) => !o)}>+</button>
      ) : (
        <button className="btn-primary" onClick={() => setOpen((o) => !o)}>+ Agent ▾</button>
      )}
      {open && (
        <div className="agent-menu" onMouseLeave={() => setOpen(false)}>
          {AGENT_TYPES.map((a) => (
            <button key={a.id} onClick={() => choose(a.id)}>{a.label}</button>
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Use it in `AgentsView.tsx`**

In `ui/src/components/AgentsView.tsx`:

Add the import near the other component imports (after line 9):

```tsx
import AgentAddMenu from "./AgentAddMenu";
```

Remove the now-unused `menuOpen` state. Delete this line (line 14):

```tsx
  const [menuOpen, setMenuOpen] = useState(false);
```

In `spawn`, delete the now-unneeded `setMenuOpen(false);` line (line 19).

Replace the agent-add block (lines 50-61) with:

```tsx
        {tab === "agents" && (
          <AgentAddMenu onSpawn={spawn} />
        )}
```

- [ ] **Step 3: Typecheck + run UI suite**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS — no unused-variable errors (confirms `menuOpen`/`setMenuOpen` fully removed).

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/AgentAddMenu.tsx ui/src/components/AgentsView.tsx
git commit -m "refactor(ui): extract AgentAddMenu from AgentsView"
```

---

### Task 6: Focus-rail `+` button and folded "AGENTS · N" spine

**Files:**
- Modify: `ui/src/components/AgentFocus.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `AgentAddMenu` (Task 5), `createAgent` from `useRuns()`.

- [ ] **Step 1: Wire `AgentAddMenu` and the spine into `AgentFocus.tsx`**

In `ui/src/components/AgentFocus.tsx`:

Add the import after the existing imports (after line 10):

```tsx
import AgentAddMenu from "./AgentAddMenu";
```

Pull `createAgent` out of the store hook. Change line 17 from:

```tsx
  const { runs, focusedRunId, setFocusedRun, refreshRuns } = useRuns();
```

to:

```tsx
  const { runs, focusedRunId, setFocusedRun, refreshRuns, createAgent } = useRuns();
```

Replace the rail head (lines 34-37) with:

```tsx
            <div className="rail-head">
              <span>Agents</span>
              <span className="spacer" />
              <AgentAddMenu variant="icon" onSpawn={createAgent} />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
```

Replace the folded stub (lines 48-50) with:

```tsx
      ) : (
        <div className="rail-stub">
          <button className="icon-btn" onClick={() => setRailOpen(true)}>»</button>
          <span className="rail-spine">AGENTS · {runs.length}</span>
        </div>
      )}
```

- [ ] **Step 2: Add the spine CSS**

In `ui/src/styles.css`, find the existing `.rail-stub` rule (search it) and replace it with the spine layout; if no `.rail-stub` rule exists, append this block near the other `.rail*` rules:

```css
.rail-stub {
  width: 38px; flex-shrink: 0; background: var(--mantle);
  border-right: 1px solid var(--line);
  display: flex; flex-direction: column; align-items: center;
  padding-top: 12px; gap: 12px;
}
.rail-spine {
  writing-mode: vertical-rl; font-size: 10px; font-weight: 600;
  letter-spacing: .18em; color: var(--o0);
}
```

Run first to see whether a `.rail-stub` rule already exists: `cd ui && grep -n "rail-stub" src/styles.css`. If it returns a rule, replace it; if it only appears after you edit `AgentFocus`, append the block.

- [ ] **Step 3: Typecheck + run UI suite**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS.

Manual check (run/verify pass): collapse the rail — a 38px spine shows `»` then vertical "AGENTS · N"; the `+` in the open rail opens the agent-type menu and creates an agent.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/AgentFocus.tsx ui/src/styles.css
git commit -m "feat(ui): add rail + button and folded AGENTS spine"
```

---

### Task 7: Registry `title` column + `set_run_title`

**Files:**
- Modify: `crates/agency-core/src/registry.rs`

**Interfaces:**
- Produces:
  - `Run` gains `pub title: Option<String>` (last field).
  - `Registry::set_run_title(&self, id: &str, title: &str) -> Result<()>`.
  - `runs` table gains nullable `title TEXT`, added via migration for existing DBs.

- [ ] **Step 1: Write the failing tests**

In `crates/agency-core/src/registry.rs`, update the test helper `sample_run` (line ~409) to set the new field, and add two tests inside the `tests` module:

Change `sample_run`'s returned struct to include `title: None,` as its last field (after `archived_at: None,`).

Add:

```rust
    #[test]
    fn run_roundtrips_title() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        assert_eq!(reg.get_run("x-1").unwrap().unwrap().title, None);
        reg.set_run_title("x-1", "Add hunk staging").unwrap();
        assert_eq!(
            reg.get_run("x-1").unwrap().unwrap().title.as_deref(),
            Some("Add hunk staging"),
        );
    }

    #[test]
    fn migrates_legacy_runs_table_without_title() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("legacy-title.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, agent TEXT NOT NULL,
                    prompt TEXT NOT NULL, base TEXT NOT NULL, branch TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
                 VALUES ('old-1','proj','claude','p','HEAD','agent/old-1',1)",
                [],
            )
            .unwrap();
        }
        let reg = Registry::open(&db).unwrap();
        assert_eq!(reg.get_run("old-1").unwrap().unwrap().title, None);
        reg.set_run_title("old-1", "Recovered").unwrap();
        assert_eq!(reg.get_run("old-1").unwrap().unwrap().title.as_deref(), Some("Recovered"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p agency-core registry`
Expected: FAIL — `Run` has no `title` field; `set_run_title` not found.

- [ ] **Step 3: Add the column to the struct**

In `crates/agency-core/src/registry.rs`, add to the `Run` struct (after `pub archived_at: Option<i64>,`, line 28):

```rust
    pub title: Option<String>,
```

- [ ] **Step 4: Create + migrate the column**

In `Registry::open`, add `title TEXT` to the `CREATE TABLE IF NOT EXISTS runs (...)` block (after the `archived_at INTEGER` line, line 82):

```rust
                archived_at INTEGER,
                title TEXT
```

And add a migration after the `archived_at` migration (after line 101):

```rust
        if !column_exists(&conn, "runs", "title")? {
            conn.execute("ALTER TABLE runs ADD COLUMN title TEXT", [])?;
        }
```

- [ ] **Step 5: Update INSERT/SELECT mapping and add the setter**

In `insert_run`, extend the SQL and params (line 220):

```rust
    pub fn insert_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64), run.archived_at, run.title
            ],
        )?;
        Ok(())
    }
```

Update the three `SELECT` statements that feed `row_to_run` (in `get_run` line 232, `list_runs` line 243, `list_archived_runs` line 256) to append `, title` to the column list, e.g. `get_run` becomes:

```rust
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title FROM runs WHERE id = ?1",
```

Apply the same `, title` addition to the `list_runs` and `list_archived_runs` SELECTs.

Update `row_to_run` (line 376) to read it:

```rust
fn row_to_run(row: &rusqlite::Row) -> Result<Run> {
    let port_base: Option<i64> = row.get(7)?;
    Ok(Run {
        id: row.get(0)?,
        project_id: row.get(1)?,
        agent: row.get(2)?,
        prompt: row.get(3)?,
        base: row.get(4)?,
        branch: row.get(5)?,
        created_at: row.get(6)?,
        port_base: port_base.map(|p| p as u16),
        archived_at: row.get(8)?,
        title: row.get(9)?,
    })
}
```

Add the setter method (alongside `set_archived`, after line 273):

```rust
    pub fn set_run_title(&self, id: &str, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET title = ?2 WHERE id = ?1",
            rusqlite::params![id, title],
        )?;
        Ok(())
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p agency-core registry`
Expected: PASS (including the existing port_base/archive tests, now that `sample_run` carries `title`).

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/registry.rs
git commit -m "feat(core): add mutable title column to runs"
```

---

### Task 8: `fallback_title` helper in core

**Files:**
- Create: `crates/agency-core/src/title.rs`
- Modify: `crates/agency-core/src/lib.rs`

**Interfaces:**
- Produces: `agency_core::title::fallback_title(first_prompt: &str) -> String` and `agency_core::title::sanitize_title(raw: &str) -> String`. `fallback_title` = `sanitize_title` of the first ~8 words / 60 chars of the first non-empty line. `sanitize_title` trims, takes the first line, strips surrounding quotes, collapses whitespace, and caps to 60 chars.

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/src/title.rs`:

```rust
//! Pure helpers for deriving a short run title from a first prompt.

/// Normalize a model- or user-provided title: first line, trimmed, surrounding
/// quotes removed, internal whitespace collapsed, capped at 60 chars.
pub fn sanitize_title(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or("").trim();
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(60).collect()
}

/// Best-effort offline title: the first several words of the first prompt.
pub fn fallback_title(first_prompt: &str) -> String {
    let line = first_prompt.lines().next().unwrap_or("").trim();
    let short = line.split_whitespace().take(8).collect::<Vec<_>>().join(" ");
    sanitize_title(&short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_quotes_and_collapses_ws() {
        assert_eq!(sanitize_title("  \"Add  hunk   staging\"  "), "Add hunk staging");
    }

    #[test]
    fn sanitize_takes_first_line_only() {
        assert_eq!(sanitize_title("Title here\nignored second line"), "Title here");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_title(&long).chars().count(), 60);
    }

    #[test]
    fn fallback_takes_first_words() {
        assert_eq!(
            fallback_title("implement hunk level staging in the git panel for real"),
            "implement hunk level staging in the git panel",
        );
    }

    #[test]
    fn fallback_handles_empty() {
        assert_eq!(fallback_title("   "), "");
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/agency-core/src/lib.rs`, add the module declaration alongside the others (match the existing `pub mod` style):

```rust
pub mod title;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p agency-core title`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-core/src/title.rs crates/agency-core/src/lib.rs
git commit -m "feat(core): add title sanitize/fallback helpers"
```

---

### Task 9: App-side title generation + `set_run_title` command

**Files:**
- Modify: `crates/agency-app/Cargo.toml`
- Modify: `crates/agency-app/src/state.rs`
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`

**Interfaces:**
- Consumes: `Registry::set_run_title` (Task 7); `agency_core::title::{fallback_title, sanitize_title}` (Task 8).
- Produces:
  - `RunInfo` gains `pub title: Option<String>`.
  - `AppState::run_title(&self, id: &str) -> Result<Option<String>>` and `AppState::store_run_title(&self, id: &str, title: &str) -> Result<()>`.
  - Tauri command `set_run_title(app: tauri::AppHandle, state, id: String, first_prompt: String)` — no-ops if a title already exists, otherwise generates one on a worker thread and stores it.
  - TS: `setRunTitle(id, firstPrompt)` and `RunInfo.title` (Task 11 wires the UI).

- [ ] **Step 1: Add reqwest**

In `crates/agency-app/Cargo.toml`, under `[dependencies]`, add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
```

- [ ] **Step 2: Add `title` to `RunInfo` and the two state accessors**

In `crates/agency-app/src/state.rs`:

Add to the `RunInfo` struct (after `pub prompt: String,`, line 33):

```rust
    pub title: Option<String>,
```

In `run_info` (the `RunInfo { … }` literal, line 347), add after `prompt: run.prompt.clone(),`:

```rust
            title: run.title.clone(),
```

In `create_run`, the `Run { … }` literal (line 395) currently ends with `archived_at: None,`. Add after it:

```rust
            title: None,
```

Then search for any other `registry::Run {` literal (e.g. in `rerun`) and add `title: None,` to each so the struct is fully initialized:

Run: `grep -n "registry::Run {" crates/agency-app/src/state.rs` and add `title: None,` to every match's literal.

Add the two accessor methods near `get_settings` (after line 264 or alongside other registry-backed methods):

```rust
    pub fn run_title(&self, id: &str) -> Result<Option<String>> {
        let reg = self.registry.lock().unwrap();
        Ok(reg.get_run(id)?.and_then(|r| r.title))
    }

    pub fn store_run_title(&self, id: &str, title: &str) -> Result<()> {
        self.registry.lock().unwrap().set_run_title(id, title)
    }
```

- [ ] **Step 3: Implement the command**

In `crates/agency-app/src/commands.rs`, add the imports needed for the LLM call near the top (after line 13):

```rust
use agency_core::title::{fallback_title, sanitize_title};
use tauri::Manager;
```

Add the command and its helper (place near `create_run`, after line 91):

```rust
const TITLE_MODEL_ANTHROPIC: &str = "claude-haiku-4-5-20251001";

/// Generate a short title for a run from its first prompt, off the UI thread.
/// No-ops if the run already has a title. Falls back to the first words of the
/// prompt when no provider is configured or the request fails.
#[tauri::command]
pub fn set_run_title(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    first_prompt: String,
) -> Result<(), String> {
    // Cheap guard on the calling thread: skip if already titled.
    if let Ok(Some(existing)) = state.run_title(&id) {
        if !existing.is_empty() {
            return Ok(());
        }
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let st = handle.state::<AppState>();
        // Re-check under the worker to avoid a race with a concurrent call.
        if matches!(st.run_title(&id), Ok(Some(t)) if !t.is_empty()) {
            return;
        }
        let settings = st.get_settings().unwrap_or(ProviderSettings {
            anthropic_api_key: String::new(),
            lm_studio_base_url: String::new(),
        });
        let title = llm_title(&settings, &first_prompt).unwrap_or_else(|| fallback_title(&first_prompt));
        if !title.is_empty() {
            let _ = st.store_run_title(&id, &title);
        }
    });
    Ok(())
}

fn llm_title(settings: &ProviderSettings, first_prompt: &str) -> Option<String> {
    let instruction = format!(
        "Generate a concise 3-5 word title for a coding task described by this first instruction. \
         Reply with only the title, no quotes and no trailing punctuation.\n\nInstruction: {first_prompt}",
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;

    if !settings.anthropic_api_key.is_empty() {
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &settings.anthropic_api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": TITLE_MODEL_ANTHROPIC,
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": instruction }],
            }))
            .send()
            .ok()?;
        let body: serde_json::Value = resp.json().ok()?;
        let text = body["content"][0]["text"].as_str()?;
        let title = sanitize_title(text);
        return (!title.is_empty()).then_some(title);
    }

    if !settings.lm_studio_base_url.is_empty() {
        let url = format!("{}/chat/completions", settings.lm_studio_base_url.trim_end_matches('/'));
        let resp = client
            .post(url)
            .json(&serde_json::json!({
                "model": "local-model",
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": instruction }],
            }))
            .send()
            .ok()?;
        let body: serde_json::Value = resp.json().ok()?;
        let text = body["choices"][0]["message"]["content"].as_str()?;
        let title = sanitize_title(text);
        return (!title.is_empty()).then_some(title);
    }

    None
}
```

- [ ] **Step 4: Register the command**

In `crates/agency-app/src/lib.rs`, add to the `tauri::generate_handler![ … ]` list (after `commands::create_run,`):

```rust
            commands::set_run_title,
```

- [ ] **Step 5: Build + test**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build && cargo test`
Expected: PASS — compiles with reqwest, all crate tests green.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/Cargo.toml crates/agency-app/src/state.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs Cargo.lock
git commit -m "feat(app): generate LLM-summarized run titles asynchronously"
```

---

### Task 10: First-prompt capture helper (UI)

**Files:**
- Create: `ui/src/lib/firstPrompt.ts`
- Test: `ui/src/lib/firstPrompt.test.ts`

**Interfaces:**
- Produces:
  - `interface CaptureState { buf: string; done: boolean }`
  - `initialCapture(): CaptureState`
  - `feed(state: CaptureState, chunk: string): { state: CaptureState; line: string | null }` — accumulates printable chars, applies backspace, and on the first CR/LF returns the trimmed line (and marks `done`); empty/whitespace-only submissions reset the buffer without completing.

- [ ] **Step 1: Write the failing tests**

Create `ui/src/lib/firstPrompt.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { initialCapture, feed } from "./firstPrompt";

function run(chunks: string[]) {
  let state = initialCapture();
  const lines: string[] = [];
  for (const c of chunks) {
    const r = feed(state, c);
    state = r.state;
    if (r.line !== null) lines.push(r.line);
  }
  return { state, lines };
}

describe("firstPrompt capture", () => {
  it("captures a line submitted with carriage return", () => {
    const { lines, state } = run(["hello world", "\r"]);
    expect(lines).toEqual(["hello world"]);
    expect(state.done).toBe(true);
  });

  it("captures across multiple chunks", () => {
    const { lines } = run(["fix ", "the ", "bug\r"]);
    expect(lines).toEqual(["fix the bug"]);
  });

  it("applies backspace (\\x7f)", () => {
    const { lines } = run(["hellp\x7fo\r"]);
    expect(lines).toEqual(["hello"]);
  });

  it("ignores empty submissions and keeps waiting", () => {
    const { lines, state } = run(["\r", "   \r", "real\r"]);
    expect(lines).toEqual(["real"]);
    expect(state.done).toBe(true);
  });

  it("stops capturing after the first line", () => {
    const { lines } = run(["first\r", "second\r"]);
    expect(lines).toEqual(["first"]);
  });

  it("ignores non-printable control characters", () => {
    const { lines } = run(["a\x1b[Db\r"]); // arrow-key escape bytes dropped
    expect(lines).toEqual(["ab"]);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ui && npx vitest run src/lib/firstPrompt.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the helper**

Create `ui/src/lib/firstPrompt.ts`:

```ts
export interface CaptureState {
  buf: string;
  done: boolean;
}

export const initialCapture = (): CaptureState => ({ buf: "", done: false });

// Feed a raw terminal-input chunk. Returns the (possibly updated) state and,
// when the first non-empty line is submitted, that trimmed line.
export function feed(
  state: CaptureState,
  chunk: string,
): { state: CaptureState; line: string | null } {
  if (state.done) return { state, line: null };
  let buf = state.buf;

  for (const ch of chunk) {
    const code = ch.codePointAt(0)!;
    if (ch === "\r" || ch === "\n") {
      const line = buf.trim();
      if (line.length > 0) {
        return { state: { buf: "", done: true }, line };
      }
      buf = ""; // empty submission — keep waiting
    } else if (ch === "\x7f" || ch === "\b") {
      buf = buf.slice(0, -1);
    } else if (code >= 0x20 && code !== 0x7f) {
      buf += ch;
    }
    // other control bytes (e.g. ESC sequences) are dropped
  }

  return { state: { buf, done: false }, line: null };
}
```

Note: ESC (`0x1b`) is `< 0x20`, so escape-sequence bytes are skipped; the surrounding printable letters in the test (`a`, `b`, `[`, `D`) — wait, `[` and `D` are printable, so the arrow-key test expects them dropped only for the ESC byte. Re-check: the test input `"a\x1b[Db"` would append `a`, drop ESC, then append `[`, `D`, `b` → `"a[Db"`, not `"ab"`. Fix the test expectation OR strip CSI. To keep the helper simple and the test honest, change the test in Step 1 to:

```ts
  it("ignores bare control characters", () => {
    const { lines } = run(["a\x00\x01b\r"]); // NUL/SOH dropped, letters kept
    expect(lines).toEqual(["ab"]);
  });
```

Use this corrected test (NUL/SOH are `< 0x20` and dropped; `a`/`b` kept). Do not attempt full CSI parsing — first-prompt capture is best-effort.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd ui && npx vitest run src/lib/firstPrompt.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/lib/firstPrompt.ts ui/src/lib/firstPrompt.test.ts
git commit -m "feat(ui): add best-effort first-prompt capture helper"
```

---

### Task 11: Wire titles through the UI

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/agents.ts`
- Modify: `ui/src/components/FocusTerminal.tsx`
- Modify: `ui/src/components/AgentFocus.tsx`
- Modify: `ui/src/components/AgentTile.tsx`
- Modify: `ui/src/components/ProjectTree.tsx`

**Interfaces:**
- Consumes: `set_run_title` command (Task 9); `initialCapture`/`feed` (Task 10).
- Produces: `RunInfo.title: string | null`; `setRunTitle(id, firstPrompt)`; `runName(run)` returns `title || prompt || branch`; `FocusTerminal` accepts `onFirstPrompt?: (line: string) => void`.

- [ ] **Step 1: Extend `api.ts`**

In `ui/src/api.ts`, add to the `RunInfo` interface (after `prompt: string;`, line 22):

```ts
  title: string | null;
```

Add the command wrapper after `createRun` (line 54):

```ts
export const setRunTitle = (id: string, firstPrompt: string) =>
  invoke<void>("set_run_title", { id, firstPrompt });
```

- [ ] **Step 2: Add the `runName` helper**

In `ui/src/agents.ts`, append:

```ts
import type { RunInfo } from "./api";

// Display name precedence: generated title, else the original prompt, else branch.
export function runName(run: Pick<RunInfo, "title" | "prompt" | "branch">): string {
  return run.title || run.prompt || run.branch;
}
```

- [ ] **Step 3: Add `onFirstPrompt` to `FocusTerminal`**

In `ui/src/components/FocusTerminal.tsx`:

Add the import after line 7:

```tsx
import { initialCapture, feed } from "../lib/firstPrompt";
import { useRef as useReactRef } from "react";
```

(If `useRef` is already imported from `react` on line 1, reuse it instead of adding `useReactRef`; line 1 is `import { useEffect, useRef } from "react";` — so just use `useRef`, do not add a second import.)

Change the component signature (line 26-28) to accept the callback:

```tsx
export default function FocusTerminal(
  { runId, stream = agentStream, onFirstPrompt }: { runId: string; stream?: TerminalStream; onFirstPrompt?: (line: string) => void },
) {
  const ref = useRef<HTMLDivElement>(null);
  const onFirstPromptRef = useRef(onFirstPrompt);
  onFirstPromptRef.current = onFirstPrompt;
  const captureRef = useRef(initialCapture());
```

Replace the `onData` wiring (line 56) with capture-aware input:

```tsx
      onData = term.onData((d) => {
        stream.input(runId, d);
        const cb = onFirstPromptRef.current;
        if (cb && !captureRef.current.done) {
          const r = feed(captureRef.current, d);
          captureRef.current = r.state;
          if (r.line !== null) cb(r.line);
        }
      });
```

Reset the capture when the run changes — add to the cleanup return (inside the `return () => { … }`, after `disposed = true;`, line 62):

```tsx
      captureRef.current = initialCapture();
```

- [ ] **Step 4: Wire the callback + `runName` in `AgentFocus.tsx`**

In `ui/src/components/AgentFocus.tsx`:

Add imports:

```tsx
import { setRunTitle } from "../api";
import { runName } from "../agents";
```

(Combine with existing `../api` import if present; `AgentFocus` currently imports `{ stopRun, discardRun, archiveRun }` from `../api` on line 3 — add `setRunTitle` there.)

Replace the rail row name (line 41) to use `runName`:

```tsx
                <span className="rail-name">{runName(r)}</span>
```

Pass `onFirstPrompt` to the agent terminal (line 74). Change:

```tsx
              ? <FocusTerminal key={focused.id} runId={focused.id} />
```

to:

```tsx
              ? <FocusTerminal key={focused.id} runId={focused.id}
                  onFirstPrompt={focused.title ? undefined : (line) => { setRunTitle(focused.id, line).catch(() => {}); }} />
```

- [ ] **Step 5: Use `runName` in `AgentTile.tsx` and `ProjectTree.tsx`**

In `ui/src/components/AgentTile.tsx`: add `import { runName } from "../agents";` (after line 3) and change line 43:

```tsx
        <span className="tile-title">{runName(run)}</span>
```

In `ui/src/components/ProjectTree.tsx`: add `import { runName } from "../agents";` (after line 5) and change the child name (line 119):

```tsx
                    <span className="tree-child-name tl">{r.agent}: {runName(r)}</span>
```

- [ ] **Step 6: Typecheck + full UI suite**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS — `RunInfo` now requires `title`, satisfied by the backend DTO; no type errors.

- [ ] **Step 7: Commit**

```bash
git add ui/src/api.ts ui/src/agents.ts ui/src/components/FocusTerminal.tsx ui/src/components/AgentFocus.tsx ui/src/components/AgentTile.tsx ui/src/components/ProjectTree.tsx
git commit -m "feat(ui): show LLM-summarized run titles and capture first prompt"
```

---

### Task 12: Full verification pass

**Files:** none (verification only).

- [ ] **Step 1: Rust build + tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build && cargo test`
Expected: PASS.

- [ ] **Step 2: UI typecheck + tests**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS.

- [ ] **Step 3: Manual smoke (use the `run` skill / launch the app)**

Verify each:
- Git review pane (full and compact): Changes and History show together; the divider drags; the History header chevron folds/unfolds; the split + fold persist across reloads.
- Projects pane: drag to narrowest — `+` stays visible, "PROJECTS" ellipsizes.
- Focus rail: `+` opens the agent menu and creates an agent; collapse shows the vertical "AGENTS · N" spine.
- New agent: type a first instruction in its terminal, press Enter; within ~2s the tile/rail/tree name updates to a short title (LLM if a provider is configured, otherwise the first words of the prompt).

- [ ] **Step 4: No commit** (verification only). If issues found, fix in the relevant task's files and re-run Steps 1-2.

---

## Self-Review

**Spec coverage:**
- Spec §1 (stacked git pane) → Tasks 1, 2, 3. ✓
- Spec §2 (projects `+`) → Task 4. ✓
- Spec §3 (rail `+` / AgentAddMenu) → Tasks 5, 6. ✓
- Spec §4 (folded spine) → Task 6. ✓
- Spec §5 (mutable title, command, capture, display) → Tasks 7, 8, 9, 10, 11. ✓
- Build/verification → Task 12. ✓

**Placeholder scan:** No TBD/TODO; all steps include concrete code and exact commands. Task 10 Step 3 explicitly corrects its own test expectation rather than leaving an inconsistency.

**Type consistency:** `Run.title: Option<String>` (Task 7) ↔ `RunInfo.title: Option<String>` (Task 9) ↔ TS `RunInfo.title: string | null` (Task 11). Registry `set_run_title` (Task 7) ↔ state `store_run_title`/`run_title` (Task 9) ↔ command `set_run_title` (Task 9) ↔ TS `setRunTitle` (Task 11). `runName(run)` precedence matches the global constraint. `feed`/`initialCapture` signatures match between Task 10 and the Task 11 wiring. `GitSections({ changesPanel, historyPanel })` matches its use in `GitPanel` (Task 3).
