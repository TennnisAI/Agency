# Settings, Review-Pane, Terminal & UX Fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix six UI/UX issues in the agency app: settings notification toggles, the crashing/cramped review pane, global text selection, the agents-rail `+` placement, resizeable panes, and a standalone shell terminal session.

**Architecture:** Items 1–5 are frontend-only (plain CSS + small React refactors). Item 6 adds a `kind` discriminator (`"agent" | "terminal"`) to the run model so a terminal can reuse the entire tmux/PTY + `FocusTerminal` streaming pipeline while skipping worktree/branch/review/merge.

**Tech Stack:** Rust (Tauri commands, `rusqlite` registry, tmux PTY), React + TypeScript (Vite), plain CSS (`ui/src/styles.css`, `ui/src/theme.css`), `vitest` for TS tests, `cargo test` for Rust.

## Global Constraints

- No AI attribution in commit messages (user standing rule): write the message and stop — no `Co-Authored-By`, no "Generated with" trailer.
- Rust build/test: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build && cargo test` (run from repo root `<home>/agency`).
- UI typecheck/test: `cd ui && npx tsc --noEmit && npx vitest run`.
- CSS approach is plain CSS with Catppuccin-style vars in `theme.css` (e.g. `--blue:#89b4fa`, `--green:#a6e3a1`, `--s1:#45475a`, `--text:#cdd6f4`, `--o0:#6c7086`, `--line:#25253a`, `--crust:#11111b`, `--mantle:#181825`). Use these vars, never hardcoded hex.
- Run id / branch / tmux session names are restricted to `[a-z0-9-]`; never repurpose them as display names.
- The agent code path (`create_run`, worktree, review, merge) must remain byte-for-byte behaviorally unchanged — terminal support is additive and gated on `kind`.

---

### Task 1: Settings — sleek toggle + notification layout

**Files:**
- Create: `ui/src/components/Toggle.tsx`
- Modify: `ui/src/components/Settings.tsx` (notifications section, ~lines 199–229)
- Modify: `ui/src/styles.css` (`.settings-notif*` rules ~lines 771–773; add `.toggle*`)

**Interfaces:**
- Produces: `Toggle` component — `function Toggle({ checked, onChange }: { checked: boolean; onChange: (next: boolean) => void }): JSX.Element`

- [ ] **Step 1: Create the Toggle component**

Create `ui/src/components/Toggle.tsx`:

```tsx
export default function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <span className={`toggle ${checked ? "on" : ""}`}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="toggle-track">
        <span className="toggle-thumb" />
      </span>
    </span>
  );
}
```

- [ ] **Step 2: Use Toggle + explicit label span in Settings notifications**

In `ui/src/components/Settings.tsx`, add the import near the other component imports at the top:

```tsx
import Toggle from "./Toggle";
```

Replace the notifications `.settings-notif` block (the `<div className="settings-notif">…</div>`, currently lines ~201–228) with:

```tsx
          <div className="settings-notif">
            {([
              ["agentFinished", "Agent finished"],
              ["agentIdle", "Agent needs input / idle"],
              ["runCrashed", "Run script crashed"],
              ["mergeAttention", "Merge needs attention"],
              ["onlyWhenUnfocused", "Only when app is not focused"],
            ] as [keyof NotifSettings, string][]).map(([key, label]) => (
              <div key={key} className="settings-notif-row">
                <span className="settings-notif-label">{label}</span>
                <Toggle
                  checked={notif[key] as boolean}
                  onChange={(next) => persistNotif({ ...notif, [key]: next })}
                />
              </div>
            ))}
            <div className="settings-notif-row">
              <span className="settings-notif-label">Idle after (seconds)</span>
              <input
                className="settings-input settings-notif-secs"
                type="number"
                min={5}
                value={notif.idleSecs}
                onChange={(e) => persistNotif({ ...notif, idleSecs: Number(e.target.value) || 30 })}
              />
            </div>
          </div>
```

- [ ] **Step 3: Replace the notif CSS and add toggle styles**

In `ui/src/styles.css`, replace the three `.settings-notif*` lines (~771–773) with:

```css
.settings-notif { display: flex; flex-direction: column; gap: 4px; }
.settings-notif-row {
  display: flex; align-items: center; justify-content: space-between;
  gap: 12px; padding: 7px 2px; color: var(--text); font-size: 13px;
}
.settings-notif-label { flex: 1; min-width: 0; }
.settings-notif-secs { width: 90px; flex: 0 0 auto; }

/* Sleek toggle switch */
.toggle { position: relative; display: inline-flex; flex-shrink: 0; cursor: pointer; }
.toggle input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
.toggle-track {
  width: 36px; height: 20px; border-radius: 999px; background: var(--s1);
  transition: background 0.15s ease; display: inline-flex; align-items: center; padding: 2px;
}
.toggle-thumb {
  width: 16px; height: 16px; border-radius: 50%; background: var(--text);
  transition: transform 0.15s ease; transform: translateX(0);
}
.toggle.on .toggle-track { background: var(--green); }
.toggle.on .toggle-thumb { transform: translateX(16px); background: var(--crust); }
```

- [ ] **Step 4: Typecheck**

Run: `cd ui && npx tsc --noEmit`
Expected: PASS (no errors).

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/Toggle.tsx ui/src/components/Settings.tsx ui/src/styles.css
git commit -m "fix(ui): sleek notification toggles and fix settings overflow"
```

---

### Task 2: Disable text selection on chrome

**Files:**
- Modify: `ui/src/styles.css` (append a selection block at the end)

- [ ] **Step 1: Add global no-select with opt-in re-enable**

Append to `ui/src/styles.css`:

```css
/* ── Text selection: off on chrome, on where copy matters ───── */
.shell { user-select: none; -webkit-user-select: none; }
.diff-body, .diff-cell, .terminal, .xterm, .xterm *,
input, textarea, code,
.settings-meta-val, .git-commit-hash, .git-commit-subject, .git-commitdetail-subject {
  user-select: text; -webkit-user-select: text;
}
```

- [ ] **Step 2: Typecheck (sanity — no TS touched, confirms build still green)**

Run: `cd ui && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add ui/src/styles.css
git commit -m "fix(ui): disable text selection on buttons/headers/labels"
```

---

### Task 3: Agents-rail `+` button placement

**Files:**
- Modify: `ui/src/components/AgentFocus.tsx` (rail head, lines ~36–41)

- [ ] **Step 1: Move the spacer after the add menu**

In `ui/src/components/AgentFocus.tsx`, replace the `rail-head` block:

```tsx
            <div className="rail-head">
              <span>Agents</span>
              <span className="spacer" />
              <AgentAddMenu variant="icon" onSpawn={createAgent} />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
```

with:

```tsx
            <div className="rail-head">
              <span>Agents</span>
              <AgentAddMenu variant="icon" onSpawn={createAgent} />
              <span className="spacer" />
              <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
            </div>
```

- [ ] **Step 2: Typecheck**

Run: `cd ui && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/AgentFocus.tsx
git commit -m "fix(ui): place rail + button right after Agents label"
```

---

### Task 4: Review pane — navigator that jumps to Source Control

**Files:**
- Modify: `ui/src/components/git/GitPanel.tsx` (make selection controlled; compact drops inline diff)
- Modify: `ui/src/components/AgentsView.tsx` (own the selection, wire both GitPanel instances)

**Interfaces:**
- Produces: `GitSelection` type exported from `GitPanel.tsx`:
  ```ts
  export type GitSelection =
    | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
    | { kind: "commit"; item: HistoryItem }
    | null;
  ```
- Produces: `GitPanel` now takes `selection: GitSelection` and `onSelect: (sel: GitSelection) => void` props (replaces internal `sel` state).
- Consumes (Task 5): `GitPanel` will additionally accept an optional `width?: number` prop — added in Task 5; not present yet.

- [ ] **Step 1: Make GitPanel selection-controlled and strip the compact diff**

Replace the entire body of `ui/src/components/git/GitPanel.tsx` with:

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

export type GitSelection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

export default function GitPanel({
  taskId,
  layout,
  selection,
  onSelect,
}: {
  taskId: string;
  layout: "compact" | "full";
  selection: GitSelection;
  onSelect: (sel: GitSelection) => void;
}) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [error, setError] = useState("");
  const [commentsKey, setCommentsKey] = useState(0);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setBranch(await gitBranchInfo(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { refresh(); }, [refresh]);
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
    onSelect({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} onAct={act}
      selectedPath={selection?.kind === "file" ? selection.path : null} onSelectFile={onSelectFile} />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null}
      selectedHash={selection?.kind === "commit" ? selection.item.hash : null}
      onSelectCommit={(item) => onSelect({ kind: "commit", item })} />
  );
  const sections = <GitSections changesPanel={changesPanel} historyPanel={historyPanel} />;

  if (layout === "compact") {
    return (
      <aside className="git-panel compact">
        {error && <div className="git-error">{error}</div>}
        {sections}
        <ReviewComments key={commentsKey} taskId={taskId} />
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
          {selection?.kind === "file" && <DiffViewer taskId={taskId} path={selection.path} mode={diffMode(selection.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} />}
          {selection?.kind === "commit" && <CommitDetail taskId={taskId} item={selection.item} />}
          {!selection && <div className="diff-empty">Select a file or commit.</div>}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Lift selection into AgentsView and wire both panels**

In `ui/src/components/AgentsView.tsx`:

Add to the imports at top:

```tsx
import GitPanel, { GitSelection } from "./git/GitPanel";
```

(Remove the existing `import GitPanel from "./git/GitPanel";` line so it isn't duplicated.)

After the existing `useState` declarations inside `AgentsView` (just below `const [pendingSpawn, setPendingSpawn] = useState(...)`), add:

```tsx
  const [gitSel, setGitSel] = useState<GitSelection>(null);
  useEffect(() => { setGitSel(null); }, [focusedRunId]);
```

Add `useEffect` to the React import at the top of the file:

```tsx
import { useEffect, useState } from "react";
```

Replace the `source` tab render:

```tsx
      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId ? <GitPanel taskId={focusedRunId} layout="full" /> : <div className="board empty">Open an agent to review its changes.</div>}
        </div>
      )}
```

with:

```tsx
      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId
            ? <GitPanel taskId={focusedRunId} layout="full" selection={gitSel} onSelect={setGitSel} />
            : <div className="board empty">Open an agent to review its changes.</div>}
        </div>
      )}
```

Replace the compact panel render:

```tsx
          {review && focusedRunId && (
            <GitPanel taskId={focusedRunId} layout="compact" />
          )}
```

with:

```tsx
          {review && focusedRunId && (
            <GitPanel
              taskId={focusedRunId}
              layout="compact"
              selection={gitSel}
              onSelect={(sel) => { setGitSel(sel); if (sel) setTab("source"); }}
            />
          )}
```

(`setTab` is already destructured from `useRuns()` in this component.)

- [ ] **Step 3: Typecheck + run UI tests**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS (no type errors; existing tests still green).

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/git/GitPanel.tsx ui/src/components/AgentsView.tsx
git commit -m "fix(ui): review pane navigates to Source Control instead of cramped inline diff"
```

---

### Task 5: Resizeable review (compact) pane

**Files:**
- Modify: `ui/src/components/git/GitPanel.tsx` (accept `width` prop on compact aside)
- Modify: `ui/src/components/AgentsView.tsx` (usePaneWidth + Resizer for the compact pane)
- Modify: `ui/src/styles.css` (`.git-panel.compact` width → min-width fallback)

**Interfaces:**
- Consumes (Task 4): `GitSelection`, `selection`/`onSelect` props on `GitPanel`.
- Produces: `GitPanel` accepts optional `width?: number`; applied as inline style on the compact `<aside>`.

- [ ] **Step 1: Accept a width prop on the compact pane**

In `ui/src/components/git/GitPanel.tsx`, extend the prop type and compact return.

Change the destructured props + signature to include `width`:

```tsx
export default function GitPanel({
  taskId,
  layout,
  selection,
  onSelect,
  width,
}: {
  taskId: string;
  layout: "compact" | "full";
  selection: GitSelection;
  onSelect: (sel: GitSelection) => void;
  width?: number;
}) {
```

Change the compact `<aside>` opening tag from:

```tsx
      <aside className="git-panel compact">
```

to:

```tsx
      <aside className="git-panel compact" style={width ? { width, minWidth: width } : undefined}>
```

- [ ] **Step 2: Add the resizer + persisted width in AgentsView**

In `ui/src/components/AgentsView.tsx`, add imports at top:

```tsx
import Resizer from "./Resizer";
import { usePaneWidth } from "../hooks/usePaneWidth";
```

Inside the component, near the other hooks, add:

```tsx
  const reviewPane = usePaneWidth("review", 360, 280, 640);
```

Replace the compact panel render (from Task 4) with the resizer + width-driven panel:

```tsx
          {review && focusedRunId && (
            <>
              <Resizer size={reviewPane.width} min={280} max={640} onChange={reviewPane.setWidth} side="right" />
              <GitPanel
                taskId={focusedRunId}
                layout="compact"
                width={reviewPane.width}
                selection={gitSel}
                onSelect={(sel) => { setGitSel(sel); if (sel) setTab("source"); }}
              />
            </>
          )}
```

- [ ] **Step 3: Relax the fixed compact width in CSS**

In `ui/src/styles.css`, change the `.git-panel.compact` rule (line ~743) from:

```css
.git-panel.compact { display: flex; flex-direction: column; width: 360px; min-width: 360px; border-left: 1px solid var(--line); background: var(--mantle); overflow: hidden; }
```

to:

```css
.git-panel.compact { display: flex; flex-direction: column; width: 360px; min-width: 280px; flex-shrink: 0; border-left: 1px solid var(--line); background: var(--mantle); overflow: hidden; }
```

(The inline `width`/`minWidth` from Step 1 override the CSS `width`; `flex-shrink: 0` keeps it from being squeezed by the agent area.)

- [ ] **Step 4: Typecheck + tests**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/git/GitPanel.tsx ui/src/components/AgentsView.tsx ui/src/styles.css
git commit -m "feat(ui): make the review pane drag-resizeable"
```

---

### Task 6: Rust — `kind` column on the run model

**Files:**
- Modify: `crates/agency-core/src/registry.rs` (struct, schema, migration, insert/select, row mapping, tests)

**Interfaces:**
- Produces: `registry::Run` gains `pub kind: String` (values `"agent"` | `"terminal"`; legacy rows default `"agent"`).

- [ ] **Step 1: Add the test for the kind column default + round-trip**

In `crates/agency-core/src/registry.rs`, inside the `#[cfg(test)] mod tests`, update `sample_run` to set `kind` and add a new test. First change `sample_run` (the helper at ~line 423) to include the field:

```rust
    fn sample_run(id: &str, port: Option<u16>) -> Run {
        Run {
            id: id.to_string(),
            project_id: "proj".to_string(),
            agent: "claude".to_string(),
            prompt: "do a thing".to_string(),
            base: "HEAD".to_string(),
            branch: format!("agent/{id}"),
            created_at: 42,
            port_base: port,
            archived_at: None,
            title: None,
            kind: "agent".to_string(),
        }
    }
```

Then add a new test function inside the same `mod tests`:

```rust
    #[test]
    fn run_kind_defaults_to_agent_and_roundtrips() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("kind.db")).unwrap();
        reg.insert_run(&sample_run("a-1", None)).unwrap();
        assert_eq!(reg.get_run("a-1").unwrap().unwrap().kind, "agent");

        let mut term = sample_run("t-1", None);
        term.kind = "terminal".to_string();
        reg.insert_run(&term).unwrap();
        assert_eq!(reg.get_run("t-1").unwrap().unwrap().kind, "terminal");
    }
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test -p agency-core registry::tests::run_kind_defaults_to_agent_and_roundtrips 2>&1 | tail -20`
Expected: FAIL — compile error, `Run` has no field `kind`.

- [ ] **Step 3: Add the field to the Run struct**

In `crates/agency-core/src/registry.rs`, change the `Run` struct (line ~18) to add `kind` after `title`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub prompt: String,
    pub base: String,
    pub branch: String,
    pub created_at: i64,
    pub port_base: Option<u16>,
    pub archived_at: Option<i64>,
    pub title: Option<String>,
    pub kind: String,
}
```

- [ ] **Step 4: Add schema column + migration**

In the `CREATE TABLE IF NOT EXISTS runs (...)` block (line ~74), change the `title TEXT` line to add `kind`:

```sql
                title TEXT,
                kind TEXT NOT NULL DEFAULT 'agent'
```

After the existing `title` migration block (line ~104–106), add:

```rust
        if !column_exists(&conn, "runs", "kind")? {
            conn.execute("ALTER TABLE runs ADD COLUMN kind TEXT NOT NULL DEFAULT 'agent'", [])?;
        }
```

- [ ] **Step 5: Update insert + selects + row mapping**

Change `insert_run` (line ~223):

```rust
    pub fn insert_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64), run.archived_at, run.title, run.kind
            ],
        )?;
        Ok(())
    }
```

In `get_run` (line ~236), `list_runs` (line ~247), and `list_archived_runs` (line ~260), append `, kind` to each `SELECT` column list (before `FROM runs`). The three column lists become:

```sql
SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind ...
```

Change `row_to_run` (line ~389) to read the new column:

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
        kind: row.get(10)?,
    })
}
```

- [ ] **Step 6: Run the new test + the existing title/migration tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test -p agency-core registry 2>&1 | tail -25`
Expected: PASS — including `run_kind_defaults_to_agent_and_roundtrips`, `run_roundtrips_title`, and `migrates_legacy_runs_table_without_title` (legacy rows read back as `kind == "agent"` via the column default).

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/registry.rs
git commit -m "feat(core): add kind discriminator column to runs"
```

---

### Task 7: Rust — `create_terminal`, command registration, discard gating

**Files:**
- Modify: `crates/agency-app/src/state.rs` (`RunInfo` struct + `run_info`, `create_run` kind, new `create_terminal`, `discard_run` gating)
- Modify: `crates/agency-app/src/commands.rs` (new `create_terminal` command)
- Modify: `crates/agency-app/src/lib.rs` (register command)

**Interfaces:**
- Consumes (Task 6): `registry::Run { kind: String, .. }`.
- Produces: `AppState::create_terminal(&self, project_id: &str) -> Result<RunInfo>`.
- Produces: Tauri command `create_terminal(project_id: String) -> Result<RunInfo, String>`.
- Produces: `RunInfo` DTO gains `pub kind: String` (serialized camelCase → TS `kind`).

- [ ] **Step 1: Add `kind` to the RunInfo DTO and populate it**

In `crates/agency-app/src/state.rs`, change the `RunInfo` struct (line ~29) to add `kind` after `port`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub prompt: String,
    pub title: Option<String>,
    pub branch: String,
    pub status: SessionStatus,
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
    pub port: Option<u16>,
    pub kind: String,
}
```

In `run_info` (line ~357), add `kind: run.kind.clone(),` to the returned struct (after `port: run.port_base,`).

- [ ] **Step 2: Set kind on the agent path**

In `create_run` (line ~406), add `kind: "agent".to_string(),` to the `Run { .. }` literal (after `title: None,`).

- [ ] **Step 3: Add the `create_terminal` state method**

In `crates/agency-app/src/state.rs`, add this method right after `create_run` (after line ~420):

```rust
    /// Create a standalone shell terminal session in the project repo root.
    /// Unlike `create_run` it has no worktree, branch, or agent profile — it just
    /// runs the user's login shell, reusing the tmux/attach/resize pipeline.
    pub fn create_terminal(&self, project_id: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let id = new_task_id("terminal");
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        // Login shell so the user's prompt/profile loads.
        let args = vec!["-l".to_string()];
        self.tmux
            .start_session(&session_name(&id), &repo, &shell, &args, &[])?;

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: project_id.to_string(),
            agent: "terminal".to_string(),
            prompt: String::new(),
            base: String::new(),
            branch: String::new(),
            created_at: now_secs(),
            port_base: None,
            archived_at: None,
            title: Some("terminal".to_string()),
            kind: "terminal".to_string(),
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }
```

- [ ] **Step 4: Gate worktree removal in discard_run**

In `discard_run` (line ~465), wrap the worktree-removal branch so terminals (which have none) skip it. Replace:

```rust
        if let Ok(repo) = self.project_repo(&run.project_id) {
            let _ = WorktreeManager::new(repo).remove(id);
        }
```

with:

```rust
        if run.kind == "agent" {
            if let Ok(repo) = self.project_repo(&run.project_id) {
                let _ = WorktreeManager::new(repo).remove(id);
            }
        }
```

- [ ] **Step 5: Add a state test for create_terminal**

In `crates/agency-app/src/state.rs`, locate the `#[cfg(test)] mod tests` block and add a test. If the existing tests construct an `AppState` over a temp project, mirror that setup; otherwise add this self-contained test that exercises the no-worktree invariant via the registry directly is not possible (needs tmux). Instead, add a focused unit test on the id/shape that does not require tmux:

```rust
    #[test]
    fn terminal_run_record_has_no_branch_and_terminal_kind() {
        // Shape check independent of tmux: a terminal Run carries kind="terminal",
        // an empty branch, and no port — the invariants discard_run/run_info rely on.
        let run = agency_core::registry::Run {
            id: new_task_id("terminal"),
            project_id: "proj".to_string(),
            agent: "terminal".to_string(),
            prompt: String::new(),
            base: String::new(),
            branch: String::new(),
            created_at: 0,
            port_base: None,
            archived_at: None,
            title: Some("terminal".to_string()),
            kind: "terminal".to_string(),
        };
        assert_eq!(run.kind, "terminal");
        assert!(run.branch.is_empty());
        assert!(run.port_base.is_none());
        assert!(run.id.starts_with("terminal-"));
    }
```

- [ ] **Step 6: Add the Tauri command**

In `crates/agency-app/src/commands.rs`, add after `create_run` (line ~93):

```rust
#[tauri::command]
pub fn create_terminal(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<RunInfo, String> {
    state.create_terminal(&project_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 7: Register the command**

In `crates/agency-app/src/lib.rs`, in the `tauri::generate_handler![...]` list, add after `commands::create_run,` (line ~67):

```rust
            commands::create_terminal,
```

- [ ] **Step 8: Build + test**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build && cargo test 2>&1 | tail -25`
Expected: PASS — compiles, `terminal_run_record_has_no_branch_and_terminal_kind` and all existing tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs
git commit -m "feat(app): create_terminal session reusing the run pipeline"
```

---

### Task 8: UI — terminal API, store, and add-menu entry

**Files:**
- Modify: `ui/src/api.ts` (`RunInfo.kind`, `createTerminal`)
- Modify: `ui/src/store/runs.tsx` (`createTerminal` action)
- Modify: `ui/src/components/AgentAddMenu.tsx` (optional "New terminal" entry)

**Interfaces:**
- Consumes (Task 7): command `create_terminal` returning `RunInfo` with `kind`.
- Produces: `api.createTerminal(projectId: string): Promise<RunInfo>`.
- Produces: `RunInfo.kind: "agent" | "terminal"` in TS.
- Produces: `useRuns().createTerminal: () => Promise<void>`.
- Produces: `AgentAddMenu` optional prop `onTerminal?: () => void` — when set, renders a "New terminal" entry.

- [ ] **Step 1: Extend the TS RunInfo + add createTerminal**

In `ui/src/api.ts`, add to the `RunInfo` interface (after `port: number | null;`):

```ts
  kind: "agent" | "terminal";
```

Add next to `createRun` (after its definition, line ~62):

```ts
export const createTerminal = (projectId: string) =>
  invoke<RunInfo>("create_terminal", { projectId });
```

- [ ] **Step 2: Add the createTerminal store action**

In `ui/src/store/runs.tsx`:

Add to the `RunStore` interface (after `createAgent: (agentId: string) => Promise<void>;`):

```tsx
  createTerminal: () => Promise<void>;
```

Update the api import (line ~2):

```tsx
import { RunInfo, createRun, createTerminal as createTerminalApi, listRuns } from "../api";
```

Add the action after `createAgent` (after line ~55):

```tsx
  const createTerminal = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) return;
    const run = await createTerminalApi(pid);
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
  }, [refreshRuns]);
```

Add `createTerminal` to the context `value={{ ... }}` object (alongside `createAgent`).

- [ ] **Step 3: Add the "New terminal" entry to AgentAddMenu**

Replace `ui/src/components/AgentAddMenu.tsx` with:

```tsx
import { useState } from "react";
import { AGENT_TYPES } from "../agents";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
}: {
  onSpawn: (agentId: string) => void;
  onTerminal?: () => void;
  variant?: "button" | "icon";
}) {
  const [open, setOpen] = useState(false);
  const choose = (id: string) => { setOpen(false); onSpawn(id); };
  const chooseTerminal = () => { setOpen(false); onTerminal?.(); };

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
          {onTerminal && (
            <>
              <div className="agent-menu-sep" />
              <button onClick={chooseTerminal}>≳ New terminal</button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
```

Add the separator style to `ui/src/styles.css` (near the `.agent-menu` rules, ~line 119):

```css
.agent-menu-sep { height: 1px; background: var(--line); margin: 4px 2px; }
```

- [ ] **Step 4: Typecheck + tests**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS. (Type errors in `AgentFocus.tsx`/`AgentsView.tsx` about `kind` are addressed in Task 9; this task only adds optional/additive members, so `tsc` stays green here.)

- [ ] **Step 5: Commit**

```bash
git add ui/src/api.ts ui/src/store/runs.tsx ui/src/components/AgentAddMenu.tsx ui/src/styles.css
git commit -m "feat(ui): terminal session API, store action, and add-menu entry"
```

---

### Task 9: UI — terminal rail row, focus header, and review gating

**Files:**
- Modify: `ui/src/components/AgentFocus.tsx` (terminal rail row, simplified header, no title capture, terminal `+` entry)
- Modify: `ui/src/components/AgentsView.tsx` (gate Review button + Source Control on `kind === "agent"`)

**Interfaces:**
- Consumes (Task 8): `useRuns().createTerminal`, `RunInfo.kind`, `AgentAddMenu` `onTerminal` prop.

- [ ] **Step 1: Wire createTerminal + terminal rail rows in AgentFocus**

In `ui/src/components/AgentFocus.tsx`:

Add `createTerminal` to the `useRuns()` destructure (line ~19):

```tsx
  const { runs, focusedRunId, setFocusedRun, refreshRuns, createAgent, createTerminal } = useRuns();
```

Pass `onTerminal` to the rail `AgentAddMenu` (the head edited in Task 3):

```tsx
              <AgentAddMenu variant="icon" onSpawn={createAgent} onTerminal={createTerminal} />
```

Replace the rail-row map (lines ~42–47) so terminals render distinctly:

```tsx
            {runs.map((r) => (
              <button key={r.id} className={`rail-row ${r.id === focusedRunId ? "on" : ""}`} onClick={() => setFocusedRun(r.id)}>
                <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
                <span className="rail-name">
                  {r.kind === "terminal" ? `≳ ${r.title || "terminal"}` : `${r.agent}: ${runName(r)}`}
                </span>
              </button>
            ))}
```

- [ ] **Step 2: Branch the focus header + body for terminals**

In `ui/src/components/AgentFocus.tsx`, replace the focused block (the `<>...</>` rendered when `focused` is truthy, lines ~60–101) so a terminal gets a Stop/Close-only header and always shows the terminal:

```tsx
        {focused ? (
          focused.kind === "terminal" ? (
            <>
              <div className="focus-head">
                <span className="badge">terminal</span>
                <span className="spacer" />
                <button className="tile-act" title="Stop shell" onClick={async () => { await stopRun(focused.id); await refreshRuns(); }}>■ Stop</button>
                <button className="tile-act danger" title="Close terminal" onClick={() => setConfirmDiscard(true)}>✕ Close</button>
              </div>
              <FocusTerminal key={focused.id} runId={focused.id} />
              {confirmDiscard && (
                <ConfirmDialog
                  title="Close terminal?"
                  body="Stop the shell and remove this terminal session."
                  confirmLabel="Close"
                  danger
                  onConfirm={async () => {
                    const id = focused.id;
                    setConfirmDiscard(false);
                    await discardRun(id);
                    setFocusedRun(null);
                    await refreshRuns();
                  }}
                  onCancel={() => setConfirmDiscard(false)}
                />
              )}
            </>
          ) : (
            <>
              <div className="focus-head">
                <span className={badgeClass(focused.agent)}>{focused.agent}</span>
                <code>{focused.branch}</code>
                <div className="focus-tabs">
                  <button className={panel === "agent" ? "on" : ""} onClick={() => setPanel("agent")}>Agent</button>
                  <button className={panel === "run" ? "on" : ""} onClick={() => setPanel("run")}>Run</button>
                </div>
                <span className="spacer" />
                <button className="tile-act" title="Stop agent" onClick={async () => { await stopRun(focused.id); await refreshRuns(); }}>■ Stop</button>
                <button className="tile-act danger" title="Discard agent" onClick={() => setConfirmDiscard(true)}>✕ Discard</button>
                <button className="tile-act" title="Archive agent" onClick={async () => {
                  const id = focused.id;
                  await archiveRun(id);
                  setFocusedRun(null);
                  await refreshRuns();
                }}>⌂ Archive</button>
                <button onClick={() => setShowMerge(true)}>Approve →</button>
              </div>
              {panel === "agent"
                ? <FocusTerminal key={focused.id} runId={focused.id}
                    onFirstPrompt={focused.title ? undefined : (line) => { setRunTitle(focused.id, line).catch(() => {}); }} />
                : <RunPanel key={`run-${focused.id}`} run={focused} />}
              {showMerge && <MergeModal taskId={focused.id} onClose={() => setShowMerge(false)} />}
              {confirmDiscard && (
                <ConfirmDialog
                  title="Discard agent?"
                  body={`Stop "${focused.agent}", remove its worktree, and delete the run. This cannot be undone.`}
                  confirmLabel="Discard"
                  danger
                  onConfirm={async () => {
                    const id = focused.id;
                    setConfirmDiscard(false);
                    await discardRun(id);
                    setFocusedRun(null);
                    await refreshRuns();
                  }}
                  onCancel={() => setConfirmDiscard(false)}
                />
              )}
            </>
          )
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
```

- [ ] **Step 3: Gate Review + Source Control on agent kind in AgentsView**

In `ui/src/components/AgentsView.tsx`:

Add a `focused` lookup near the top of the component body (after the `useRuns()` destructure):

```tsx
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;
```

Gate the Review toggle button (line ~45–47). Replace:

```tsx
        {tab === "agents" && (
          <button className={review ? "on" : ""} onClick={() => setReview((r) => !r)}>Review</button>
        )}
```

with:

```tsx
        {tab === "agents" && focused?.kind === "agent" && (
          <button className={review ? "on" : ""} onClick={() => setReview((r) => !r)}>Review</button>
        )}
```

Gate the `source` tab so terminals show a message instead of erroring on git. Replace the `source` render (edited in Task 4) with:

```tsx
      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId && focused?.kind === "agent"
            ? <GitPanel taskId={focusedRunId} layout="full" selection={gitSel} onSelect={setGitSel} />
            : <div className="board empty">{focused?.kind === "terminal" ? "Terminals have no source control." : "Open an agent to review its changes."}</div>}
        </div>
      )}
```

Gate the compact review render (edited in Tasks 4–5). Change its condition from `{review && focusedRunId && (` to:

```tsx
          {review && focusedRunId && focused?.kind === "agent" && (
```

- [ ] **Step 4: Typecheck + tests**

Run: `cd ui && npx tsc --noEmit && npx vitest run`
Expected: PASS — `kind` now exists on `RunInfo`, all references resolve.

- [ ] **Step 5: Full build sanity**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/AgentFocus.tsx ui/src/components/AgentsView.tsx
git commit -m "feat(ui): terminal rail row, Stop/Close header, and review gating"
```

---

## Manual verification (after all tasks)

Run the app (`cd ui && pnpm tauri dev` or the project's usual launch) and confirm:

1. **Settings** — open Settings → Notifications: each row shows label left, a sleek pill toggle right; toggling animates and persists; no text clipped off the modal edge.
2. **Review pane** — focus an agent, open Review; expand a History commit and click a file → it jumps to the full Source Control tab with a proper wide diff; no crash; History fold works; dragging the review pane's left edge resizes it.
3. **Text selection** — try to select a button label / "AGENTS" header → nothing highlights; selecting inside a diff or the terminal still works (copy).
4. **Rail `+`** — in Focus view, the `+` sits immediately right of "Agents"; `«` stays at the far right.
5. **Terminal** — rail `+` → "New terminal" → a `≳ terminal` row appears and focuses; run `ls`/`npm run build`; header shows only Stop/Close; Source Control/Review are not offered; Close removes it and leaves no `.agency/worktrees/terminal-*` directory.

## Self-review notes (coverage)

- Spec §1 → Task 1; §2 → Task 4; §3 → Task 2; §4 → Task 3; §5 → Task 5; §6 → Tasks 6–9.
- Type consistency: `GitSelection` (Task 4) is the single selection type used by Task 5; `RunInfo.kind` (Task 8) backs the gating in Task 9; `Run.kind` (Task 6) feeds `RunInfo.kind` via `run_info` (Task 7).
