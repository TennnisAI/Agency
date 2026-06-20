# Agency — UX Fixes + Design Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Agency app's project/agent UX (header `+`, instant agent spawn, close/stop/discard, pane resizing) and apply a styling pass to match the design handoff.

**Architecture:** Tauri app — Rust core (`agency-core`) + Tauri commands (`agency-app`) + React/TS UI (`ui/`). Persistence stays in the global SQLite registry. "Close project" kills tmux sessions but keeps DB records; "remove project" and "discard agent" delete records + worktrees.

**Tech Stack:** Rust, rusqlite, tmux, Tauri v2, React 18 + TypeScript, Vite, xterm.js, vitest.

## Global Constraints

- Persistence: global SQLite registry (`~/.config/agency/agency.db`) remains source of truth. Do NOT add a per-project `.agency/agency.json`.
- `.agency/worktrees/<id>/` layout is unchanged.
- No AI attribution in commit messages.
- Catppuccin Mocha palette (exact tokens in Task 9). Fonts: Geist (UI), JetBrains Mono (code).
- Agent type → badge color: `claude`→peach `#fab387`, `pi`→teal `#94e2d5`, `hermes`→mauve `#cba6f7`.
- Spec dimensions (px): title bar 40, status bar 27, sidebar 266, focus rail 312 (38 collapsed), source-control left 336, review panel 360, grid tile min 340 / row 262.
- Run all Rust tests with `cargo test -p agency-app -p agency-core`. Run frontend tests with `cd ui && npm test`.

---

## Phase 1 — Backend

### Task 1: Seed Claude/Pi/Hermes profiles + bare-command spawn

**Files:**
- Modify: `crates/agency-app/src/state.rs:92-115` (`AppState::new` seeding)
- Modify: `crates/agency-app/src/state.rs:226-254` (`create_run`, drop empty args)
- Test: `crates/agency-app/tests/profiles.rs` (create)

**Interfaces:**
- Produces: profiles named `claude`, `pi`, `hermes` (each `args: []`), plus existing `shell`. `create_run` launches the bare command when `prompt` is empty.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-app/tests/profiles.rs`:

```rust
use agency_app::AppState;

#[test]
fn seeds_three_agent_profiles_with_bare_commands() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let profiles = state.list_profiles().unwrap();
    let by_name = |n: &str| profiles.iter().find(|p| p.name == n).cloned();

    let claude = by_name("claude").expect("claude profile");
    assert_eq!(claude.command, "claude");
    assert!(claude.args.is_empty(), "claude must run interactively (no prompt arg)");

    let pi = by_name("pi").expect("pi profile");
    assert_eq!(pi.command, "pi");
    assert!(pi.args.is_empty());

    let hermes = by_name("hermes").expect("hermes profile");
    assert_eq!(hermes.command, "hermes");
    assert!(hermes.args.is_empty());
}
```

Ensure `tempfile` is a dev-dependency of `agency-app` (check `crates/agency-app/Cargo.toml`; add `tempfile = "3"` under `[dev-dependencies]` if missing).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-app --test profiles -- --nocapture`
Expected: FAIL — `claude` has args `["{{prompt}}"]`, and `pi`/`hermes` are absent.

- [ ] **Step 3: Implement seeding + bare spawn**

In `state.rs`, replace the seeding block inside `AppState::new` (the `if registry.list_profiles()?.is_empty() { ... }` block) with an idempotent reconcile that runs every startup. Add a helper above `impl AppState`:

```rust
fn agent_profile(name: &str, command: &str) -> AgentProfile {
    AgentProfile { name: name.into(), command: command.into(), args: vec![], env: vec![] }
}
```

Then in `AppState::new`, after `let registry = Registry::open(db_path)?;`, replace the old seeding with:

```rust
    // Seed the built-in shell profile once.
    if registry.get_profile("shell")?.is_none() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        registry.upsert_profile(&AgentProfile {
            name: "shell".to_string(),
            command: shell,
            args: vec!["-l".to_string()],
            env: vec![],
        })?;
    }
    // Ensure built-in agent profiles exist (added for existing DBs too).
    for (name, command) in [("claude", "claude"), ("pi", "pi"), ("hermes", "hermes")] {
        if registry.get_profile(name)?.is_none() {
            registry.upsert_profile(&agent_profile(name, command))?;
        }
    }
```

In `create_run`, change the args line (currently `let args = profile.render_args(prompt);`) to drop empties so an empty prompt launches the bare command:

```rust
        let args: Vec<String> = profile
            .render_args(prompt)
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect();
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agency-app --test profiles -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app
git commit -m "Seed claude/pi/hermes profiles and spawn bare command on empty prompt"
```

---

### Task 2: `close_project` and `delete_project`

**Files:**
- Modify: `crates/agency-app/src/state.rs` (replace `remove_project` with `close_project` + `delete_project`)
- Modify: `crates/agency-app/src/commands.rs:53-56` (replace `remove_project` command)
- Modify: `crates/agency-app/src/lib.rs:20` (handler registration)
- Test: `crates/agency-app/tests/project_lifecycle.rs` (create)

**Interfaces:**
- Consumes: `Registry::list_runs(project_id)`, `Registry::get_run`, `Registry::delete_run`, `Registry::remove_project`, `Tmux::kill_session`, `WorktreeManager::remove`.
- Produces: `AppState::close_project(id)` (kills sessions, keeps records), `AppState::delete_project(id)` (kills sessions + removes worktrees + deletes runs + deletes project). Tauri commands `close_project`, `delete_project`.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-app/tests/project_lifecycle.rs`. (Sessions can't be spawned without tmux in CI, so this test asserts DB record semantics, which is the behavior that matters for reopen.)

```rust
use agency_app::AppState;

#[test]
fn close_keeps_records_delete_removes_them() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // init a git repo so project ops are valid
    std::process::Command::new("git").arg("init").current_dir(&repo).output().unwrap();

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let project = state.add_project("repo", &repo).unwrap();

    // close_project must succeed and keep the project record
    state.close_project(&project.id).unwrap();
    assert!(state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "close_project must keep the project");

    // delete_project must remove the project record
    state.delete_project(&project.id).unwrap();
    assert!(!state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "delete_project must remove the project");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-app --test project_lifecycle`
Expected: FAIL — `close_project` / `delete_project` do not exist.

- [ ] **Step 3: Implement in `state.rs`**

Replace `pub fn remove_project` (lines ~177-179) with:

```rust
    pub fn close_project(&self, id: &str) -> Result<()> {
        // Kill live terminals; keep project + run records so reopen can re-run.
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        for run in &runs {
            self.attaches.lock().unwrap().remove(&run.id);
            self.tmux.kill_session(&session_name(&run.id)).ok();
        }
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let runs = self.registry.lock().unwrap().list_runs(id)?;
        let repo = self.project_repo(id).ok();
        for run in &runs {
            self.attaches.lock().unwrap().remove(&run.id);
            self.tmux.kill_session(&session_name(&run.id)).ok();
            if let Some(repo) = &repo {
                let _ = WorktreeManager::new(repo.clone()).remove(&run.id);
            }
            self.registry.lock().unwrap().delete_run(&run.id)?;
        }
        self.registry.lock().unwrap().remove_project(id)?;
        Ok(())
    }
```

(Confirm `WorktreeManager::new` takes an owned `PathBuf`; it does in `discard_run` via `self.project_repo(...)?`. If it borrows, pass `repo` by reference to match `discard_run`'s call site.)

- [ ] **Step 4: Update commands + registration**

In `commands.rs`, replace the `remove_project` command (lines 53-56) with:

```rust
#[tauri::command]
pub fn close_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.close_project(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_project(&id).map_err(|e| e.to_string())
}
```

In `lib.rs`, replace `commands::remove_project,` with:

```rust
            commands::close_project,
            commands::delete_project,
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p agency-app --test project_lifecycle`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app
git commit -m "Add close_project (keep records) and delete_project (purge) commands"
```

---

### Task 3: `stop_run` (kill session, keep record)

**Files:**
- Modify: `crates/agency-app/src/state.rs` (add `stop_run`)
- Modify: `crates/agency-app/src/commands.rs` (add `stop_run` command)
- Modify: `crates/agency-app/src/lib.rs` (register)

**Interfaces:**
- Produces: `AppState::stop_run(id)` (remove attach, kill session, keep DB record); Tauri command `stop_run`.

- [ ] **Step 1: Implement `stop_run` in `state.rs`** (add next to `discard_run`):

```rust
    pub fn stop_run(&self, id: &str) -> Result<()> {
        self.attaches.lock().unwrap().remove(id);
        self.tmux.kill_session(&session_name(id)).ok();
        Ok(())
    }
```

- [ ] **Step 2: Add command in `commands.rs`** (next to `discard_run`):

```rust
#[tauri::command]
pub fn stop_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_run(&id).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register in `lib.rs`** — add `commands::stop_run,` after `commands::discard_run,`.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p agency-app`
Expected: builds clean.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app
git commit -m "Add stop_run command (kill session, keep run record)"
```

---

## Phase 2 — Frontend behavior

### Task 4: API bindings + agent metadata

**Files:**
- Modify: `ui/src/api.ts:35-36` (replace `removeProject`; add `closeProject`, `deleteProject`, `stopRun`)
- Create: `ui/src/agents.ts` (agent type list + badge colors)
- Test: `ui/src/agents.test.ts` (create)

**Interfaces:**
- Produces: `closeProject(id)`, `deleteProject(id)`, `stopRun(id)` in `api.ts`; `AGENT_TYPES` (`{id,label}[]`) and `agentColor(name)` in `agents.ts`.

- [ ] **Step 1: Write the failing test**

Create `ui/src/agents.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { AGENT_TYPES, agentColor } from "./agents";

describe("agents", () => {
  it("lists the three agent types", () => {
    expect(AGENT_TYPES.map((a) => a.id)).toEqual(["claude", "pi", "hermes"]);
  });
  it("maps each type to its badge color", () => {
    expect(agentColor("claude")).toBe("#fab387");
    expect(agentColor("pi")).toBe("#94e2d5");
    expect(agentColor("hermes")).toBe("#cba6f7");
  });
  it("falls back to a neutral color for unknown agents", () => {
    expect(agentColor("shell")).toBe("#a6adc8");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- agents`
Expected: FAIL — `./agents` not found.

- [ ] **Step 3: Implement `ui/src/agents.ts`**

```ts
export interface AgentType {
  id: string;
  label: string;
}

export const AGENT_TYPES: AgentType[] = [
  { id: "claude", label: "Claude Code" },
  { id: "pi", label: "Pi" },
  { id: "hermes", label: "Hermes" },
];

const COLORS: Record<string, string> = {
  claude: "#fab387",
  pi: "#94e2d5",
  hermes: "#cba6f7",
};

export function agentColor(name: string): string {
  return COLORS[name] ?? "#a6adc8";
}
```

- [ ] **Step 4: Update `api.ts`** — replace `removeProject` (lines 35-36) with:

```ts
export const closeProject = (id: string) => invoke<void>("close_project", { id });
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });
```

And add next to `discardRun` (line 46):

```ts
export const stopRun = (id: string) => invoke<void>("stop_run", { id });
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd ui && npm test -- agents`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add ui/src/agents.ts ui/src/agents.test.ts ui/src/api.ts
git commit -m "Add agent metadata + close/delete/stop API bindings"
```

---

### Task 5: Projects pane — header `+` button, remove bottom form, close/remove menu

**Files:**
- Modify: `ui/src/components/ProjectTree.tsx` (full rewrite of header + actions)
- Create: `ui/src/components/ConfirmDialog.tsx` (reusable confirm)

**Interfaces:**
- Consumes: `addProject`, `closeProject`, `deleteProject` from `api.ts`; native `open` from `@tauri-apps/plugin-dialog`.
- Produces: `ConfirmDialog` component — props `{ title: string; body: string; confirmLabel: string; danger?: boolean; onConfirm: () => void; onCancel: () => void }`.

- [ ] **Step 1: Create `ConfirmDialog.tsx`**

```tsx
export default function ConfirmDialog({
  title,
  body,
  confirmLabel,
  danger,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal confirm" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button className="modal-x" onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">{body}</div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={onCancel}>Cancel</button>
          <button className={danger ? "btn-danger" : "btn-primary"} onClick={onConfirm}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Rewrite `ProjectTree.tsx`**

Replace the header `<h2>` with a header row containing the eyebrow label and a `+` button; remove the bottom `.add-project` form; add a per-row actions menu (Close / Remove) backed by `ConfirmDialog`. The `+` opens the folder picker, derives the name from the basename, and calls `addProject`.

```tsx
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, RunInfo, addProject, closeProject, deleteProject, listProjects, listRuns } from "../api";
import ConfirmDialog from "./ConfirmDialog";

function statusClass(s: RunInfo["status"]): string {
  return s.state === "running" ? "running" : "exited";
}

type Pending =
  | { kind: "close" | "remove"; project: Project }
  | null;

export default function ProjectTree({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (p: Project) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [expanded, setExpanded] = useState<Record<string, RunInfo[] | undefined>>({});
  const [pending, setPending] = useState<Pending>(null);

  async function refresh() {
    setProjects(await listProjects());
  }
  useEffect(() => { refresh(); }, []);

  async function toggle(p: Project) {
    setExpanded((e) => ({ ...e, [p.id]: e[p.id] ? undefined : [] }));
    if (!expanded[p.id]) {
      try {
        const runs = await listRuns(p.id);
        setExpanded((e) => ({ ...e, [p.id]: runs }));
      } catch { /* ignore */ }
    }
  }

  async function handleAdd() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const name = sel.split("/").filter(Boolean).pop() ?? sel;
    await addProject(name, sel);
    await refresh();
  }

  async function confirmPending() {
    if (!pending) return;
    const { kind, project } = pending;
    if (kind === "close") await closeProject(project.id);
    else await deleteProject(project.id);
    setPending(null);
    await refresh();
  }

  return (
    <aside className="tree">
      <div className="tree-head">
        <span className="eyebrow">PROJECTS</span>
        <button className="icon-add" title="Add project" onClick={handleAdd}>+</button>
      </div>
      <ul className="tree-list">
        {projects.map((p) => (
          <li key={p.id}>
            <div className={`tree-row ${p.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(p)}>
              <span className="chev" onClick={(e) => { e.stopPropagation(); toggle(p); }}>
                {expanded[p.id] !== undefined ? "▾" : "▸"}
              </span>
              <span className="proj-icon" aria-hidden>{p.name.slice(0, 1).toUpperCase()}</span>
              <span className="tree-name tl">{p.name}</span>
              <button className="row-act" title="Close project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "close", project: p }); }}>⏻</button>
              <button className="row-act danger" title="Remove project" onClick={(e) => { e.stopPropagation(); setPending({ kind: "remove", project: p }); }}>×</button>
            </div>
            {expanded[p.id] !== undefined && (
              <ul className="tree-children">
                {(expanded[p.id] ?? []).map((r) => (
                  <li key={r.id} className="tree-child">
                    <span className={`dot ${statusClass(r.status)}`} />
                    <span className="tree-child-name tl">{r.agent}: {r.prompt || r.branch}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      {pending && (
        <ConfirmDialog
          title={pending.kind === "close" ? "Close project?" : "Remove project?"}
          body={pending.kind === "close"
            ? `Stop all running agents in "${pending.project.name}". Their setup is kept — reopen to re-run them.`
            : `Permanently remove "${pending.project.name}" and delete all its agents and worktrees. This cannot be undone.`}
          confirmLabel={pending.kind === "close" ? "Close project" : "Remove project"}
          danger={pending.kind === "remove"}
          onConfirm={confirmPending}
          onCancel={() => setPending(null)}
        />
      )}
    </aside>
  );
}
```

- [ ] **Step 3: Verify build/types**

Run: `cd ui && npx tsc --noEmit`
Expected: no type errors. (If `removeProject` is referenced elsewhere, update those call sites.)

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/ProjectTree.tsx ui/src/components/ConfirmDialog.tsx
git commit -m "Projects pane: header + button, folder-picker add, close/remove with confirm"
```

---

### Task 6: Agents header — `+ Agent ▾` menu, instant spawn

**Files:**
- Modify: `ui/src/components/AgentsView.tsx` (replace `+ New task` with agent-type menu; remove NewTaskForm modal usage)
- Modify: `ui/src/store/runs.tsx` (add `createAgent(agentId)` helper, keep `openNewTask` removed or unused)
- Delete: `ui/src/components/NewTaskForm.tsx`

**Interfaces:**
- Consumes: `createRun(projectId, "", agentId, "HEAD")`, `AGENT_TYPES`, `refreshRuns`.
- Produces: a header dropdown; selecting an agent spawns it immediately and focuses it.

- [ ] **Step 1: Add `createAgent` to the store**

In `runs.tsx`, import `createRun`:

```ts
import { RunInfo, createRun, listRuns } from "../api";
```

Add to the `RunStore` interface: `createAgent: (agentId: string) => Promise<void>;` and remove `openNewTask` / `setOpenNewTask` (and their state + provider value entries). Implement inside `RunStoreProvider`:

```ts
  const createAgent = useCallback(async (agentId: string) => {
    const pid = projectRef.current;
    if (!pid) return;
    const run = await createRun(pid, "", agentId, "HEAD");
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
  }, [refreshRuns]);
```

Add `createAgent` to the provider `value` object; delete `openNewTask`, `setOpenNewTask` from it.

- [ ] **Step 2: Rewrite the header actions in `AgentsView.tsx`**

Remove the `NewTaskForm` import and its overlay render (line 58) and the `openNewTask`/`setOpenNewTask` destructure. Add an agent menu:

```tsx
import { useState } from "react";
import { Project } from "../api";
import { AGENT_TYPES } from "../agents";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import MergeModal from "./MergeModal";
import SourceControl from "./SourceControl";
import GitReviewPanel from "./GitReviewPanel";

export default function AgentsView({ project }: { project: Project }) {
  const { runs, view, setView, focusedRunId, tab, setTab, approveRunId, setApproveRun, createAgent } = useRuns();
  const [review, setReview] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);

  async function spawn(agentId: string) {
    setMenuOpen(false);
    await createAgent(agentId);
  }

  return (
    <main className="agents">
      <div className="content-head">
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} onClick={() => setTab("agents")}>▦ Agents</button>
          <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
        </div>
        {tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} onClick={() => setView("grid")}>▦ Grid</button>
            <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")}>▭ Focus</button>
          </div>
        )}
        <div className="spacer" />
        {tab === "agents" && (
          <button className={review ? "on" : ""} onClick={() => setReview((r) => !r)}>Review</button>
        )}
        {tab === "agents" && (
          <div className="agent-add">
            <button className="btn-primary" onClick={() => setMenuOpen((o) => !o)}>+ Agent ▾</button>
            {menuOpen && (
              <div className="agent-menu" onMouseLeave={() => setMenuOpen(false)}>
                {AGENT_TYPES.map((a) => (
                  <button key={a.id} onClick={() => spawn(a.id)}>{a.label}</button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId ? <SourceControl taskId={focusedRunId} /> : <div className="board empty">Open an agent to review its changes.</div>}
        </div>
      )}

      {tab === "agents" && (
        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, overflow: "hidden" }}>
            {view === "grid" && (
              <div className="grid">
                {runs.length === 0 && <div className="board empty">No agents yet — add one with “+ Agent”.</div>}
                {runs.map((r) => <AgentTile key={r.id} run={r} />)}
              </div>
            )}
            {view === "focus" && <AgentFocus />}
          </div>
          {review && focusedRunId && (
            <GitReviewPanel taskId={focusedRunId} onOpenSource={() => setTab("source")} />
          )}
        </div>
      )}

      {approveRunId && approveRunId === focusedRunId && (
        <MergeModal taskId={approveRunId} onClose={() => setApproveRun(null)} />
      )}
    </main>
  );
}
```

- [ ] **Step 3: Update the keyboard shortcut**

In `App.tsx`, the `onNewTask` shortcut calls `setOpenNewTask(true)` which no longer exists. Change `useShortcuts`'s `onNewTask` to spawn the default agent: destructure `createAgent` from `useRuns()` and set `onNewTask: () => createAgent("claude")`. Remove the `setOpenNewTask` destructure.

- [ ] **Step 4: Delete `NewTaskForm.tsx`**

```bash
git rm ui/src/components/NewTaskForm.tsx
```

- [ ] **Step 5: Verify build/types**

Run: `cd ui && npx tsc --noEmit`
Expected: no type errors (fix any remaining `openNewTask` references).

- [ ] **Step 6: Commit**

```bash
git add ui/src
git commit -m "Add agents via instant-spawn type menu; remove new-task prompt modal"
```

---

### Task 7: Stop / discard an agent (tile + focus header + rail)

**Files:**
- Modify: `ui/src/components/AgentTile.tsx` (Stop + Discard controls)
- Modify: `ui/src/components/AgentFocus.tsx` (Stop + Discard in header/rail)
- Reuse: `ui/src/components/ConfirmDialog.tsx`

**Interfaces:**
- Consumes: `stopRun(id)`, `discardRun(id)`, `refreshRuns`, `rerun(id)`.
- Produces: per-agent Stop (keep) and Discard (delete, danger confirm) actions; after discard, clear focus if it was focused.

- [ ] **Step 1: Read both components first**

Read `ui/src/components/AgentTile.tsx` and `ui/src/components/AgentFocus.tsx` in full before editing so the new controls fit the existing footer/header markup.

- [ ] **Step 2: Add controls to `AgentTile.tsx`**

Import `stopRun`, `discardRun` from `../api`, `useRuns` from `../store/runs`, and `ConfirmDialog`. Add local `const [confirmDiscard, setConfirmDiscard] = useState(false);` and `const { refreshRuns, focusedRunId, setFocusedRun } = useRuns();`. In the tile footer, add:

```tsx
<button className="tile-act" title="Stop agent" onClick={async (e) => { e.stopPropagation(); await stopRun(run.id); await refreshRuns(); }}>■ Stop</button>
<button className="tile-act danger" title="Discard agent" onClick={(e) => { e.stopPropagation(); setConfirmDiscard(true); }}>✕</button>
```

At the end of the tile JSX:

```tsx
{confirmDiscard && (
  <ConfirmDialog
    title="Discard agent?"
    body={`Stop "${run.agent}", remove its worktree, and delete the run. This cannot be undone.`}
    confirmLabel="Discard"
    danger
    onConfirm={async () => {
      await discardRun(run.id);
      if (focusedRunId === run.id) setFocusedRun(null);
      setConfirmDiscard(false);
      await refreshRuns();
    }}
    onCancel={() => setConfirmDiscard(false)}
  />
)}
```

- [ ] **Step 3: Add the same Stop + Discard to `AgentFocus.tsx`**

In the terminal header action row, add a `■ Stop` button (calls `stopRun(focusedRunId)` then `refreshRuns`) and a danger Discard button that opens a `ConfirmDialog` with the same body; on confirm call `discardRun`, `setFocusedRun(null)`, `refreshRuns`.

- [ ] **Step 4: Verify build/types**

Run: `cd ui && npx tsc --noEmit`
Expected: no type errors.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components
git commit -m "Surface Stop (keep) and Discard (delete, confirm) per agent"
```

---

### Task 8: Drag-to-resize panes

**Files:**
- Create: `ui/src/hooks/usePaneWidth.ts` (width state + localStorage persistence + clamp)
- Create: `ui/src/components/Resizer.tsx` (drag handle)
- Modify: `ui/src/App.tsx` (sidebar width via CSS var + resizer)
- Modify: `ui/src/components/AgentFocus.tsx`, `SourceControl.tsx`, `GitReviewPanel.tsx` (resizers)
- Modify: `ui/src/styles.css` (resizer styling; widths from CSS vars)
- Test: `ui/src/hooks/usePaneWidth.test.ts` (create)

**Interfaces:**
- Produces: `usePaneWidth(key, def, min, max) → { width: number; setWidth: (n: number) => void }` (persists to `localStorage["pane:"+key]`, clamps to `[min,max]`); `Resizer` component — props `{ width: number; min: number; max: number; onChange: (n: number) => void; side?: "left" | "right" }`.

- [ ] **Step 1: Write the failing test**

Create `ui/src/hooks/usePaneWidth.test.ts`:

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { usePaneWidth } from "./usePaneWidth";

describe("usePaneWidth", () => {
  beforeEach(() => localStorage.clear());

  it("uses the default when nothing is stored", () => {
    const { result } = renderHook(() => usePaneWidth("sidebar", 266, 200, 420));
    expect(result.current.width).toBe(266);
  });

  it("clamps and persists updates", () => {
    const { result } = renderHook(() => usePaneWidth("sidebar", 266, 200, 420));
    act(() => result.current.setWidth(999));
    expect(result.current.width).toBe(420);
    expect(localStorage.getItem("pane:sidebar")).toBe("420");
  });

  it("restores a stored width", () => {
    localStorage.setItem("pane:sidebar", "300");
    const { result } = renderHook(() => usePaneWidth("sidebar", 266, 200, 420));
    expect(result.current.width).toBe(300);
  });
});
```

If `@testing-library/react` is not installed, add it: `cd ui && npm i -D @testing-library/react`. Ensure `vitest.config.ts` uses `environment: "jsdom"` (add `jsdom` dev dep if needed).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- usePaneWidth`
Expected: FAIL — hook not found.

- [ ] **Step 3: Implement `usePaneWidth.ts`**

```ts
import { useCallback, useState } from "react";

const clamp = (n: number, min: number, max: number) => Math.min(max, Math.max(min, n));

export function usePaneWidth(key: string, def: number, min: number, max: number) {
  const storageKey = `pane:${key}`;
  const [width, setWidthState] = useState<number>(() => {
    const raw = typeof localStorage !== "undefined" ? localStorage.getItem(storageKey) : null;
    const parsed = raw ? Number(raw) : NaN;
    return Number.isFinite(parsed) ? clamp(parsed, min, max) : def;
  });
  const setWidth = useCallback((n: number) => {
    const w = clamp(n, min, max);
    setWidthState(w);
    try { localStorage.setItem(storageKey, String(w)); } catch { /* ignore */ }
  }, [storageKey, min, max]);
  return { width, setWidth };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npm test -- usePaneWidth`
Expected: PASS.

- [ ] **Step 5: Implement `Resizer.tsx`**

```tsx
import { useCallback } from "react";

export default function Resizer({
  width,
  min,
  max,
  onChange,
  side = "left",
}: {
  width: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
  side?: "left" | "right";
}) {
  const onPointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = width;
    const move = (ev: PointerEvent) => {
      const delta = ev.clientX - startX;
      onChange(side === "left" ? startW + delta : startW - delta);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.cursor = "";
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    document.body.style.cursor = "col-resize";
  }, [width, min, max, onChange, side]);

  return <div className="resizer" onPointerDown={onPointerDown} role="separator" aria-orientation="vertical" />;
}
```

- [ ] **Step 6: Wire the sidebar in `App.tsx`**

In `Shell`, add `const sidebar = usePaneWidth("sidebar", 266, 200, 460);`. Render the sidebar with an explicit width and a `Resizer` after it:

```tsx
{sidebarOpen && (
  <>
    <div style={{ width: sidebar.width, flexShrink: 0, display: "flex", minHeight: 0 }}>
      <ProjectTree selectedId={selectedProjectId} onSelect={selectProject} />
    </div>
    <Resizer width={sidebar.width} min={200} max={460} onChange={sidebar.setWidth} side="left" />
  </>
)}
```

Ensure `.tree` is `width: 100%` so it fills the wrapper (adjust in Task 10's CSS). Import `usePaneWidth` and `Resizer`.

- [ ] **Step 7: Wire the focus rail, source-control left pane, and review panel**

Apply the same pattern in `AgentFocus.tsx` (rail: `usePaneWidth("rail", 312, 220, 520)`, `side="left"`), `SourceControl.tsx` (left pane: `usePaneWidth("scLeft", 336, 260, 560)`, `side="left"`), and `GitReviewPanel.tsx` (panel: `usePaneWidth("review", 360, 280, 560)`, `side="right"`). Each renders its pane at the hook width with a `Resizer` on the inner edge.

- [ ] **Step 8: Add resizer CSS** (in `styles.css`):

```css
.resizer {
  width: 5px;
  flex-shrink: 0;
  cursor: col-resize;
  background: transparent;
  transition: background .12s;
}
.resizer:hover { background: #313244; }
```

- [ ] **Step 9: Verify build/tests**

Run: `cd ui && npx tsc --noEmit && npm test -- usePaneWidth`
Expected: types clean, tests PASS.

- [ ] **Step 10: Commit**

```bash
git add ui/src
git commit -m "Add drag-to-resize for sidebar, focus rail, source-control, and review panes"
```

---

## Phase 3 — Design hardening

> Each task below reads the current `ui/src/styles.css` first, then applies the spec values. After each, run `cd ui && npx tsc --noEmit` and visually verify with `cd ui && npm run dev` if a browser is available. Keep changes scoped to styling; do not alter component behavior.

### Task 9: Catppuccin Mocha tokens + fonts

**Files:**
- Modify: `ui/src/styles.css` (`:root` tokens, font stacks)
- Create: `ui/src/fonts.css` + `ui/src/fonts/` (if font files are available)

- [ ] **Step 1: Define the token set** at the top of `styles.css` `:root` (replace any ad-hoc colors):

```css
:root {
  --crust:#11111b; --mantle:#181825; --base:#1e1e2e; --line:#25253a;
  --s0:#313244; --s1:#45475a; --s2:#585b70;
  --o0:#6c7086; --o1:#7f849c; --o2:#9399b2;
  --sub0:#a6adc8; --sub1:#bac2de; --text:#cdd6f4;
  --blue:#89b4fa; --lav:#b4befe; --mauve:#cba6f7; --pink:#f5c2e7;
  --green:#a6e3a1; --teal:#94e2d5; --yellow:#f9e2af; --peach:#fab387; --red:#f38ba8;
  --font-ui:"Geist",ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;
  --font-mono:"JetBrains Mono",ui-monospace,"SF Mono",Menlo,monospace;
  --sidebar-w:266px; --focus-rail-w:312px; --sc-left-w:336px; --review-w:360px;
}
body { font-family: var(--font-ui); background: var(--base); color: var(--text); }
code, pre, .mono { font-family: var(--font-mono); }
```

- [ ] **Step 2: Bundle fonts if available.** If Geist/JetBrains Mono `.woff2` files can be added under `ui/src/fonts/`, create `ui/src/fonts.css` with `@font-face` rules (weights 400/500/600/700) and import it from `ui/src/main.tsx`. If font binaries are not available offline, skip the `@font-face` and rely on the family fallbacks (the families are already first in the stack).

- [ ] **Step 3: Verify + commit**

Run: `cd ui && npx tsc --noEmit`

```bash
git add ui/src
git commit -m "Add Catppuccin Mocha tokens and Geist/JetBrains Mono font stacks"
```

---

### Task 10: Shell dimensions + projects pane styling

**Files:** Modify `ui/src/styles.css`.

- [ ] **Step 1:** Set fixed chrome dimensions: title bar `height:40px`, status bar `height:27px`, `.tree { width:100%; background:var(--mantle); border-right:1px solid var(--line); }`.

- [ ] **Step 2:** Style the new projects header and rows:

```css
.tree-head { display:flex; align-items:center; justify-content:space-between; padding:12px 12px 8px; }
.tree-head .eyebrow { font-size:10.5px; font-weight:600; letter-spacing:.13em; color:var(--o0); }
.icon-add { width:22px; height:22px; border-radius:6px; background:var(--base); border:1px solid var(--s0); color:var(--text); cursor:pointer; }
.icon-add:hover { background:var(--s0); }
.tree-row { display:flex; align-items:center; gap:8px; padding:7px 8px; border-radius:9px; cursor:pointer; }
.tree-row.selected { background:var(--s0); box-shadow:inset 2px 0 0 var(--blue); }
.tree-row .chev { width:14px; text-align:center; color:var(--o0); font-size:9px; }
.proj-icon { width:24px; height:24px; border-radius:7px; background:var(--blue); color:var(--crust); font-size:12px; font-weight:700; display:flex; align-items:center; justify-content:center; flex-shrink:0; }
.tree-name { font-size:13px; font-weight:500; color:var(--text); flex:1; min-width:0; }
.tl { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.row-act { opacity:0; border:none; background:transparent; color:var(--o0); cursor:pointer; }
.tree-row:hover .row-act { opacity:1; }
.row-act:hover { color:var(--text); }
.row-act.danger:hover { color:var(--red); }
.tree-children { margin:2px 0 7px 19px; border-left:1px solid var(--line); padding-left:9px; }
.tree-child { display:flex; align-items:center; gap:6px; padding:5px 7px; border-radius:6px; }
.tree-child:hover { background:var(--base); }
.dot { width:6px; height:6px; border-radius:50%; flex-shrink:0; }
.dot.running { background:var(--green); box-shadow:0 0 8px var(--green); }
.dot.exited { background:var(--o0); }
```

- [ ] **Step 2: Verify + commit**

```bash
git add ui/src/styles.css
git commit -m "Style projects pane and fix shell dimensions to spec"
```

---

### Task 11: Agent tiles/focus — status semantics, badges, terminal theme

**Files:** Modify `ui/src/styles.css`; verify `AgentTile.tsx` / `AgentFocus.tsx` emit the class names; apply xterm theme where the terminal is constructed (`FocusTerminal.tsx`).

- [ ] **Step 1:** Grid + tile per spec: `.grid { padding:16px; display:grid; grid-template-columns:repeat(auto-fill,minmax(340px,1fr)); grid-auto-rows:262px; gap:14px; align-content:start; }`. Tile base `border:1px solid var(--s0); border-radius:12px; background:var(--mantle);`. State modifiers: `.tile.awaiting { border-color:var(--s1); box-shadow:0 0 0 1px rgba(249,226,175,.12); }`, `.tile.crashed { border-color:rgba(243,139,168,.35); }`, `.tile.exited { opacity:.82; }`.

- [ ] **Step 2:** Status dot states + `pulse`/`blink` keyframes:

```css
@keyframes pulse { 0%,100%{opacity:1;} 50%{opacity:.3;} }
@keyframes blink { 0%,49%{opacity:1;} 50%,100%{opacity:0;} }
.dot.awaiting { background:var(--yellow); }
.dot.idle { background:var(--blue); }
.dot.crashed { background:var(--red); }
.dot.running { animation:pulse 1.6s infinite; }
```

- [ ] **Step 3:** Agent badge driven by `agentColor(run.agent)` — set inline `style={{ color: agentColor(run.agent) }}` on the badge in `AgentTile.tsx`/`AgentFocus.tsx`; base style `.agent-badge { font-family:var(--font-mono); font-size:10.5px; background:var(--base); border-radius:5px; padding:2px 7px; white-space:nowrap; }`.

- [ ] **Step 4:** xterm theme in `FocusTerminal.tsx` when constructing `new Terminal({...})`:

```ts
theme: {
  background: "#11111b", foreground: "#bac2de", cursor: "#a6e3a1",
  black: "#45475a", red: "#f38ba8", green: "#a6e3a1", yellow: "#f9e2af",
  blue: "#89b4fa", magenta: "#cba6f7", cyan: "#94e2d5", white: "#bac2de",
},
fontFamily: "JetBrains Mono, ui-monospace, monospace", fontSize: 13,
```

- [ ] **Step 5: Verify + commit**

```bash
git add ui/src
git commit -m "Apply agent status semantics, type-colored badges, and xterm palette"
```

---

### Task 12: Status bar

**Files:** Modify `ui/src/components/StatusBar.tsx` + `styles.css`.

- [ ] **Step 1:** Read `StatusBar.tsx`. Render: `⎇ <branch>` · project name · task counts derived from `useRuns().runs` (running / exited counts) · spacer · `⌘N new · ⌘G source · ⌘↵ approve` · `v<version>`. Style: `height:27px; display:flex; align-items:center; gap:16px; padding:0 14px; background:var(--crust); border-top:1px solid var(--line); font-family:var(--font-mono); font-size:11px; color:var(--o0);` with running counts in `var(--green)` and shortcuts in `var(--o1)`.

- [ ] **Step 2: Verify + commit**

```bash
git add ui/src
git commit -m "Status bar: branch, project, task counts, shortcuts, version"
```

---

### Task 13: Source control / review / diff / modal styling

**Files:** Modify `ui/src/styles.css`; verify class names in `SourceControl.tsx`, `DiffView.tsx`, `GitReviewPanel.tsx`, `MergeModal.tsx`, `ConfirmDialog.tsx`.

- [ ] **Step 1:** Buttons + segmented control:

```css
.btn-primary { background:var(--blue); color:var(--crust); font-weight:600; border:none; border-radius:8px; padding:8px 13px; cursor:pointer; }
.btn-primary:hover { background:var(--lav); }
.btn-secondary { background:var(--base); border:1px solid var(--s1); color:var(--text); border-radius:8px; padding:8px 13px; cursor:pointer; }
.btn-secondary:hover { background:var(--s0); }
.btn-danger { background:transparent; border:1px solid rgba(243,139,168,.4); color:var(--red); border-radius:8px; padding:8px 13px; cursor:pointer; }
.btn-danger:hover { background:rgba(243,139,168,.12); }
.seg { display:inline-flex; background:var(--crust); border:1px solid var(--line); border-radius:8px; padding:3px; }
.seg button { background:transparent; border:none; color:var(--o1); border-radius:5px; padding:4px 9px; font-size:11px; cursor:pointer; }
.seg button.on { background:var(--s0); color:var(--text); }
```

- [ ] **Step 2:** Diff add/del/hunk styling:

```css
.diff-line { padding:0 16px; white-space:pre; color:var(--o1); font-family:var(--font-mono); font-size:12.5px; }
.diff-line.add { background:rgba(166,227,161,.10); color:var(--green); }
.diff-line.del { background:rgba(243,139,168,.10); color:var(--red); }
.hunk-head { padding:7px 16px; background:rgba(137,180,250,.07); color:var(--blue); border-top:1px solid var(--line); border-bottom:1px solid var(--line); }
```

- [ ] **Step 3:** Modal/backdrop + file-status colors:

```css
.modal-backdrop { position:absolute; inset:0; background:rgba(17,17,27,.66); backdrop-filter:blur(3px); z-index:20; display:flex; align-items:center; justify-content:center; }
.modal { width:580px; max-width:92%; background:var(--mantle); border:1px solid var(--s1); border-radius:15px; box-shadow:0 24px 64px rgba(0,0,0,.5); }
.modal.confirm { width:440px; }
.modal-head { display:flex; align-items:center; justify-content:space-between; padding:16px 18px; border-bottom:1px solid var(--line); }
.modal-body { padding:18px; color:var(--sub1); line-height:1.6; }
.modal-foot { display:flex; gap:10px; justify-content:flex-end; padding:14px 18px; border-top:1px solid var(--line); background:var(--crust); }
.modal-x { background:transparent; border:none; color:var(--o0); cursor:pointer; }
.status-A { color:var(--green); font-weight:700; }
.status-M { color:var(--yellow); font-weight:700; }
.status-D { color:var(--red); font-weight:700; }
```

- [ ] **Step 4:** Set source-control/review widths from CSS vars: `.sc-left { width:var(--sc-left-w); flex-shrink:0; }`, `.review-panel { width:var(--review-w); flex-shrink:0; border-left:1px solid var(--line); }`, `.rail { width:var(--focus-rail-w); flex-shrink:0; background:var(--mantle); border-right:1px solid var(--line); }`. (When the resizers from Task 8 are active, the inline width wins; the var is the default.)

- [ ] **Step 5: Verify + commit**

```bash
git add ui/src
git commit -m "Style source control, diffs, segmented controls, buttons, and modals to spec"
```

---

## Final verification

- [ ] `cargo test -p agency-app -p agency-core` — all pass.
- [ ] `cd ui && npx tsc --noEmit && npm test` — types clean, tests pass.
- [ ] `cd ui && npm run build` — production build succeeds.
- [ ] Manual run-through (build/run the Tauri app): add a project via header `+`; pick each agent type from `+ Agent ▾` and confirm a terminal spawns running the bare command; type into it; Stop then re-run; Discard with confirm; Close a project (agents stop, project stays) then reopen and re-run; Remove a project (gone); drag-resize sidebar, focus rail, source-control, and review panes and confirm widths persist across reload.
```
