# Base Branch & Merge Target Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user pick which branch an agent is created *from* (base) and which branch its work merges *into* (merge target) from the "+ Agent" menu, with the two synced by default until the merge target is explicitly edited.

**Architecture:** The backend already plumbs a `base` string end-to-end (`create_run` → `WorktreeManager::create` → `git worktree add … <base>`). We add a sibling `merge_target` that is persisted on the `Run` record and consumed at merge time, replacing the hard-coded `detect_base` call with "use the stored target, else fall back to `detect_base`". A new read-only command lists a project's branches so the UI can populate two dropdowns. The agent still gets its own quarantined `agent/<id>` branch in the middle — we only change where that branch *starts* and where it *lands*.

**Tech Stack:** Rust (`agency-core`, `agency-app` / Tauri commands, `rusqlite`), TypeScript + React 18 (Vite), Vitest, `cargo test`.

## Global Constraints

- UI package manager is **pnpm**, not npm (lockfile churns otherwise). Run UI commands as `pnpm …` from `ui/`.
- Rust edition/style: every git call goes through a `Command::new("git")` helper that `bail!`s on non-zero exit (see `agency-core/src/git.rs:17`). Match it.
- SQLite schema changes must be **additive migrations** guarded by `column_exists(...)` (see `registry.rs:100-111`). Never rewrite the `CREATE TABLE`.
- The agent's own branch name stays `agent/<task_id>` (`worktree.rs:88`). Do **not** touch it — it is the isolation boundary.
- Backward compatibility: existing runs and the icon-variant menu must keep working with no base/target chosen (base `"HEAD"`, merge target auto-detected).
- Serde on these structs uses default field-name casing for `Run`/`Worktree` (snake_case) and `#[serde(rename_all = "camelCase")]` only where already annotated. New API-facing structs that cross to TS use `#[serde(rename_all = "camelCase")]`.

---

## File Structure

**Backend — created/modified:**
- `crates/agency-core/src/git.rs` — add `ProjectBranches` struct + `list_branches(repo)`.
- `crates/agency-core/src/merge.rs` — add `resolve_target(explicit, repo)` helper (the "stored target else detect_base" decision, made unit-testable).
- `crates/agency-core/src/registry.rs` — add `Run.merge_target: Option<String>`, migration, and update every `runs` SELECT/INSERT.
- `crates/agency-app/src/state.rs` — `create_run` accepts/stores `merge_target`; new `list_project_branches`; `merge_preview`/`merge_task` use `resolve_target`.
- `crates/agency-app/src/commands.rs` — `create_run` command gains `merge_target` param; new `list_project_branches` command.
- `crates/agency-app/src/lib.rs` — register the new command in the Tauri handler.
- `crates/agency-core/tests/merge.rs` — tests for `resolve_target`.
- `crates/agency-core/src/git.rs` (tests module) — test for `list_branches`.

**Frontend — created/modified:**
- `ui/src/api.ts` — `ProjectBranches` type, `listProjectBranches`, widened `createRun`.
- `ui/src/lib/branchTargets.ts` (new) — pure `effectiveMergeTarget` helper.
- `ui/src/lib/branchTargets.test.ts` (new) — Vitest unit tests.
- `ui/src/store/runs.tsx` — widen `createAgent` to take optional `{ base, mergeTarget }`.
- `ui/src/components/AgentAddMenu.tsx` — optional branch pickers; widened `onSpawn`.
- `ui/src/components/AgentsView.tsx` — thread base/target through `spawn` + `pendingSpawn`; pass `projectId` to the menu.
- `ui/src/styles.css` — styles for the branch-picker footer.

---

### Task 1: Backend — list a project's branches

**Files:**
- Modify: `crates/agency-core/src/git.rs` (add struct + fn near other pub fns; add test in `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `agency_core::git::ProjectBranches { current: String, branches: Vec<String> }` and `agency_core::git::list_branches(repo: &Path) -> anyhow::Result<ProjectBranches>`.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/agency-core/src/git.rs`, inside (or adding) a `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod branch_tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn run(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git").args(args).current_dir(dir).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn list_branches_returns_current_first_and_all_locals() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        run(repo, &["init", "-q", "-b", "main"]);
        run(repo, &["config", "user.email", "t@t"]);
        run(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        run(repo, &["add", "."]);
        run(repo, &["commit", "-qm", "init"]);
        run(repo, &["branch", "develop"]);

        let pb = list_branches(repo).unwrap();
        assert_eq!(pb.current, "main");
        assert!(pb.branches.contains(&"main".to_string()));
        assert!(pb.branches.contains(&"develop".to_string()));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core list_branches_returns_current_first_and_all_locals`
Expected: FAIL — `cannot find function list_branches` / `ProjectBranches`.

- [ ] **Step 3: Write minimal implementation**

Add near the top-level pub fns in `crates/agency-core/src/git.rs` (after the `use` block / `FileChange` struct):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranches {
    /// The branch currently checked out in the primary worktree (or "HEAD" if detached).
    pub current: String,
    /// All local branch names, with `current` first.
    pub branches: Vec<String>,
}

pub fn list_branches(repo: &Path) -> Result<ProjectBranches> {
    let current = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    let raw = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
    let mut branches: Vec<String> = raw.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    // Put the current branch first so the UI can preselect it.
    if let Some(pos) = branches.iter().position(|b| b == &current) {
        branches.remove(pos);
        branches.insert(0, current.clone());
    }
    Ok(ProjectBranches { current, branches })
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agency-core list_branches_returns_current_first_and_all_locals`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/git.rs
git commit -m "feat(core): list a project's local branches with current first"
```

---

### Task 2: Backend — persist `merge_target` on the Run record

**Files:**
- Modify: `crates/agency-core/src/registry.rs` (struct `Run:18-31`, migrations `:100-111`, `insert_run:228-238`, every `runs` SELECT at `:242, :253, :266`, `row_to_run:394-409`)

**Interfaces:**
- Produces: `Run.merge_target: Option<String>` round-trips through SQLite. `None` for legacy rows.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `registry.rs` (follow the existing in-memory-registry test pattern; if helpers differ, mirror the nearest existing run test):

```rust
#[test]
fn run_merge_target_round_trips() {
    let reg = Registry::open_in_memory().unwrap();
    let project = reg.add_project("p", std::path::Path::new("/tmp/p")).unwrap();
    let run = Run {
        id: "t1".into(),
        project_id: project.id.clone(),
        agent: "claude".into(),
        prompt: "hi".into(),
        base: "main".into(),
        branch: "agent/t1".into(),
        created_at: 1,
        port_base: None,
        archived_at: None,
        title: None,
        kind: "agent".into(),
        merge_target: Some("develop".into()),
    };
    reg.insert_run(&run).unwrap();
    let got = reg.get_run("t1").unwrap().unwrap();
    assert_eq!(got.merge_target.as_deref(), Some("develop"));
}
```

> If `Registry::open_in_memory` does not exist, use the same constructor the existing registry tests use (search the test module for how they build a `Registry`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core run_merge_target_round_trips`
Expected: FAIL — `Run` has no field `merge_target`.

- [ ] **Step 3: Write minimal implementation**

In `registry.rs`:

Add the field to `Run` (after `kind` at line 30):

```rust
    pub kind: String,
    /// Branch this run's work merges into. `None` = auto-detect (main/master) at merge time.
    pub merge_target: Option<String>,
```

Add a migration after the `kind` migration (after line 111):

```rust
        if !column_exists(&conn, "runs", "merge_target")? {
            conn.execute("ALTER TABLE runs ADD COLUMN merge_target TEXT", [])?;
        }
```

Update `insert_run` (lines 229-236) to write the new column:

```rust
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at, title, kind, merge_target)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64), run.archived_at, run.title, run.kind,
                run.merge_target
            ],
        )?;
```

Append `, merge_target` to the column list in all three SELECTs:
- `get_run` (line 242)
- `list_runs` (line 253)
- `list_archived_runs` (line 266)

Each becomes `… title, kind, merge_target FROM runs …`.

Update `row_to_run` (lines 394-409) to read column 11:

```rust
        kind: row.get(10)?,
        merge_target: row.get(11)?,
    })
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agency-core run_merge_target_round_trips`
Expected: PASS.

- [ ] **Step 5: Build the workspace to catch other `Run {…}` constructors**

Run: `cargo build -p agency-core -p agency-app`
Expected: FAIL with "missing field `merge_target`" in `state.rs` `create_run` — that's fixed in Task 4. If any *other* `Run { … }` literal errors, add `merge_target: None` there.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-core/src/registry.rs
git commit -m "feat(core): persist merge_target on the Run record"
```

---

### Task 3: Backend — `resolve_target` merge helper

**Files:**
- Modify: `crates/agency-core/src/merge.rs` (add fn after `detect_base:31-38`)
- Test: `crates/agency-core/tests/merge.rs`

**Interfaces:**
- Consumes: `detect_base` (merge.rs:31).
- Produces: `agency_core::merge::resolve_target(explicit: Option<&str>, repo: &Path) -> anyhow::Result<String>` — returns `explicit` if `Some`, else `detect_base(repo)`.

- [ ] **Step 1: Write the failing test**

Add to `crates/agency-core/tests/merge.rs` (reuse whatever temp-repo setup helper the file already defines; the snippet below inlines its own to be safe):

```rust
#[test]
fn resolve_target_prefers_explicit_then_falls_back() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let run = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(repo).status().unwrap().success());
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f"), "x").unwrap();
    run(&["add", "."]);
    run(&["commit", "-qm", "init"]);

    // Explicit wins.
    assert_eq!(
        agency_core::merge::resolve_target(Some("develop"), repo).unwrap(),
        "develop"
    );
    // None falls back to detect_base → "main".
    assert_eq!(
        agency_core::merge::resolve_target(None, repo).unwrap(),
        "main"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core --test merge resolve_target_prefers_explicit_then_falls_back`
Expected: FAIL — `cannot find function resolve_target`.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/agency-core/src/merge.rs` after `detect_base` (line 38):

```rust
/// The branch a run should merge into: the explicitly chosen target if set,
/// otherwise the auto-detected main/master. Centralizes the fallback so
/// `merge_preview` and `merge_task` stay in agreement.
pub fn resolve_target(explicit: Option<&str>, repo: &Path) -> Result<String> {
    match explicit {
        Some(t) if !t.trim().is_empty() => Ok(t.to_string()),
        _ => detect_base(repo),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agency-core --test merge resolve_target_prefers_explicit_then_falls_back`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/merge.rs crates/agency-core/tests/merge.rs
git commit -m "feat(core): add resolve_target merge-destination helper"
```

---

### Task 4: Backend — wire create/merge/list through state + commands

**Files:**
- Modify: `crates/agency-app/src/state.rs` (`create_run:396-437`, `merge_preview:700`, `merge_task:722-726`; add `list_project_branches`)
- Modify: `crates/agency-app/src/commands.rs` (`create_run:82-93`; add `list_project_branches`)
- Modify: `crates/agency-app/src/lib.rs` (Tauri `generate_handler!` registration)

**Interfaces:**
- Consumes: `agency_core::git::{ProjectBranches, list_branches}` (Task 1), `Run.merge_target` (Task 2), `agency_core::merge::resolve_target` (Task 3).
- Produces:
  - `state.create_run(project_id, prompt, agent, base, merge_target: Option<&str>) -> Result<RunInfo>`
  - `state.list_project_branches(project_id) -> Result<ProjectBranches>`
  - Tauri commands `create_run(... merge_target: Option<String>)` and `list_project_branches(project_id) -> ProjectBranches`.

- [ ] **Step 1: Update `state.create_run` signature + Run literal**

In `state.rs`, change the signature (line 396):

```rust
    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str, merge_target: Option<&str>) -> Result<RunInfo> {
```

In the `Run { … }` literal (lines 422-434), after `kind: "agent".to_string(),` add:

```rust
            kind: "agent".to_string(),
            merge_target: merge_target.map(|s| s.to_string()),
```

- [ ] **Step 2: Point merge_preview + merge_task at `resolve_target`**

In `merge_preview` (around line 705) replace:

```rust
    let base = agency_core::merge::detect_base(&repo)?;
```
with:
```rust
    let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
```

In `merge_task` (lines 722-726) replace the `detect_base` line the same way:

```rust
    pub fn merge_task(&self, id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::resolve_target(run.merge_target.as_deref(), &repo)?;
        agency_core::merge::merge(&repo, &run.branch, &base)
    }
```

- [ ] **Step 3: Add `list_project_branches` to state**

Add near `create_run` in `state.rs`:

```rust
    pub fn list_project_branches(&self, project_id: &str) -> Result<agency_core::git::ProjectBranches> {
        let repo = self.project_repo(project_id)?;
        agency_core::git::list_branches(&repo)
    }
```

- [ ] **Step 4: Update the Tauri commands**

In `commands.rs`, widen `create_run` (lines 82-93):

```rust
#[tauri::command]
pub fn create_run(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agent: String,
    base: String,
    merge_target: Option<String>,
) -> Result<RunInfo, String> {
    state
        .create_run(&project_id, &prompt, &agent, &base, merge_target.as_deref())
        .map_err(|e| e.to_string())
}
```

Add a new command (next to the other git commands, e.g. after `git_branch_info`):

```rust
#[tauri::command]
pub fn list_project_branches(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<agency_core::git::ProjectBranches, String> {
    state.list_project_branches(&project_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Register the command**

In `crates/agency-app/src/lib.rs`, add `list_project_branches` to the `tauri::generate_handler![…]` list (alongside `create_run` / `git_branch_info`).

- [ ] **Step 6: Fix the other internal `create_run` caller**

`create_terminal` and `rerun` don't call `create_run`, but search to be safe:

Run: `grep -rn "create_run(" crates/agency-app/src`
For each call (other than the command wrapper and the method def), pass the new arg. The terminal path uses its own logic; agent creation is only via the command. If a test or seed calls `create_run`, append `None` (or `Some("…")`).

- [ ] **Step 7: Build + run backend tests**

Run: `cargo build -p agency-app && cargo test -p agency-core`
Expected: PASS, no missing-field/arity errors.

- [ ] **Step 8: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs
git commit -m "feat(app): create_run accepts merge_target; expose list_project_branches; merge honors stored target"
```

---

### Task 5: Frontend — API layer

**Files:**
- Modify: `ui/src/api.ts` (`createRun:55-56`; add `ProjectBranches` + `listProjectBranches`)

**Interfaces:**
- Consumes: Tauri commands `create_run`, `list_project_branches` (Task 4).
- Produces:
  - `interface ProjectBranches { current: string; branches: string[] }`
  - `listProjectBranches(projectId: string): Promise<ProjectBranches>`
  - `createRun(projectId, prompt, agent, base, mergeTarget?: string | null): Promise<RunInfo>`

- [ ] **Step 1: Widen `createRun`**

Replace `ui/src/api.ts:55-56`:

```ts
export const createRun = (projectId: string, prompt: string, agent: string, base: string, mergeTarget?: string | null) =>
  invoke<RunInfo>("create_run", { projectId, prompt, agent, base, mergeTarget: mergeTarget ?? null });
```

- [ ] **Step 2: Add branch types + getter**

Add near `gitBranchInfo` (api.ts ~215-226):

```ts
export interface ProjectBranches {
  current: string;
  branches: string[];
}

export const listProjectBranches = (projectId: string) =>
  invoke<ProjectBranches>("list_project_branches", { projectId });
```

- [ ] **Step 3: Typecheck**

Run: `cd ui && pnpm exec tsc --noEmit`
Expected: PASS (no callers broke — `mergeTarget` is optional).

- [ ] **Step 4: Commit**

```bash
git add ui/src/api.ts
git commit -m "feat(ui): api for listProjectBranches and createRun mergeTarget"
```

---

### Task 6: Frontend — pure sync helper (TDD)

**Files:**
- Create: `ui/src/lib/branchTargets.ts`
- Test: `ui/src/lib/branchTargets.test.ts`

**Interfaces:**
- Produces: `effectiveMergeTarget(base: string, override: string | null): string` — returns `override ?? base`. This encodes "merge target follows base unless the user has overridden it."

- [ ] **Step 1: Write the failing test**

Create `ui/src/lib/branchTargets.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { effectiveMergeTarget } from "./branchTargets";

describe("effectiveMergeTarget", () => {
  it("follows the base when there is no override", () => {
    expect(effectiveMergeTarget("main", null)).toBe("main");
    expect(effectiveMergeTarget("develop", null)).toBe("develop");
  });

  it("uses the override when set", () => {
    expect(effectiveMergeTarget("main", "release")).toBe("release");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && pnpm test branchTargets`
Expected: FAIL — cannot resolve `./branchTargets`.

- [ ] **Step 3: Write minimal implementation**

Create `ui/src/lib/branchTargets.ts`:

```ts
/**
 * The merge target the UI should use: the user's explicit override if they have
 * diverged it from the base, otherwise the base branch (synced-by-default).
 */
export function effectiveMergeTarget(base: string, override: string | null): string {
  return override ?? base;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && pnpm test branchTargets`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/lib/branchTargets.ts ui/src/lib/branchTargets.test.ts
git commit -m "feat(ui): effectiveMergeTarget sync helper with tests"
```

---

### Task 7: Frontend — thread base/target through the store and spawn flow

**Files:**
- Modify: `ui/src/store/runs.tsx` (`createAgent:49-56`)
- Modify: `ui/src/components/AgentsView.tsx` (`spawn:24-36`, `pendingSpawn:19`, confirm handler `:115-123`, menu render `:57`)

**Interfaces:**
- Consumes: `createRun(..., mergeTarget?)` (Task 5).
- Produces:
  - `createAgent(agentId: string, opts?: { base: string; mergeTarget: string }): Promise<void>`
  - `AgentAddMenu` receives `projectId` and a widened `onSpawn(agentId, opts?)` (consumed in Task 8).

- [ ] **Step 1: Widen `createAgent` in the store**

Replace `ui/src/store/runs.tsx:49-56`:

```tsx
  const createAgent = useCallback(async (agentId: string, opts?: { base: string; mergeTarget: string }) => {
    const pid = projectRef.current;
    if (!pid) return;
    const base = opts?.base ?? "HEAD";
    const mergeTarget = opts?.mergeTarget ?? null;
    const run = await createRun(pid, "", agentId, base, mergeTarget);
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
  }, [refreshRuns]);
```

- [ ] **Step 2: Thread opts through `spawn` + `pendingSpawn` in AgentsView**

Widen the `pendingSpawn` state type (line 19):

```tsx
  const [pendingSpawn, setPendingSpawn] = useState<{ agentId: string; readiness: RepoReadiness; repoPath: string; opts?: { base: string; mergeTarget: string } } | null>(null);
```

Replace `spawn` (lines 24-36):

```tsx
  async function spawn(agentId: string, opts?: { base: string; mergeTarget: string }) {
    setError("");
    try {
      const r = await inspectRepo(project.repo_path);
      if (r.state === "ready" && !r.dirty) {
        await createAgent(agentId, opts);
      } else {
        setPendingSpawn({ agentId, readiness: r, repoPath: project.repo_path, opts });
      }
    } catch (e) {
      setError(String(e));
    }
  }
```

Update the `pendingSpawn` confirm handler (lines 115-123) to forward `opts`:

```tsx
          onResolved={async () => {
            const { agentId, opts } = pendingSpawn;
            setPendingSpawn(null);
            try {
              await createAgent(agentId, opts);
            } catch (e) {
              setError(String(e));
            }
          }}
```

Pass `projectId` to the main-button menu (line 57):

```tsx
        {tab === "agents" && (
          <AgentAddMenu projectId={project.id} onSpawn={spawn} />
        )}
```

- [ ] **Step 3: Typecheck**

Run: `cd ui && pnpm exec tsc --noEmit`
Expected: FAIL only inside `AgentAddMenu.tsx` (its `onSpawn` prop type is still narrow) — fixed in Task 8. The `AgentFocus.tsx:38` icon usage (`onSpawn={createAgent}`) stays valid because `createAgent`'s new optional 2nd arg is compatible with a `(agentId) => void` call.

- [ ] **Step 4: Commit**

```bash
git add ui/src/store/runs.tsx ui/src/components/AgentsView.tsx
git commit -m "feat(ui): thread base/mergeTarget through spawn and pendingSpawn"
```

---

### Task 8: Frontend — branch pickers in the Agent menu

**Files:**
- Modify: `ui/src/components/AgentAddMenu.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `listProjectBranches`, `ProjectBranches` (Task 5); `effectiveMergeTarget` (Task 6); widened `onSpawn` / `projectId` (Task 7).
- Produces: final UX — when `projectId` is provided the menu shows "from"/"into" branch selects; selecting an agent spawns with `{ base, mergeTarget }`. Without `projectId` (icon variant) the menu is unchanged and spawns with no opts.

- [ ] **Step 1: Rewrite `AgentAddMenu.tsx`**

Replace the file with:

```tsx
import { useEffect, useRef, useState } from "react";
import { AGENT_TYPES } from "../agents";
import { listProjectBranches } from "../api";
import { effectiveMergeTarget } from "../lib/branchTargets";

export default function AgentAddMenu({
  onSpawn,
  onTerminal,
  variant = "button",
  projectId,
}: {
  onSpawn: (agentId: string, opts?: { base: string; mergeTarget: string }) => void;
  onTerminal?: () => void;
  variant?: "button" | "icon";
  projectId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left?: number; right?: number }>();
  const btnRef = useRef<HTMLButtonElement>(null);

  // Branch-picker state (only used when projectId is supplied).
  const [branches, setBranches] = useState<string[]>([]);
  const [base, setBase] = useState<string>("");
  const [targetOverride, setTargetOverride] = useState<string | null>(null);

  const showPicker = !!projectId;
  const mergeTarget = effectiveMergeTarget(base, targetOverride);
  const diverged = targetOverride !== null && targetOverride !== base;

  // Load branches when the menu opens (cheap; reflects any branch the user just made).
  useEffect(() => {
    if (!open || !projectId) return;
    let live = true;
    listProjectBranches(projectId)
      .then((pb) => {
        if (!live) return;
        setBranches(pb.branches);
        setBase((b) => (b && pb.branches.includes(b) ? b : pb.current));
      })
      .catch(() => { /* leave selects empty; spawn falls back to HEAD */ });
    return () => { live = false; };
  }, [open, projectId]);

  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      if (next && btnRef.current) {
        const r = btnRef.current.getBoundingClientRect();
        setCoords(
          variant === "icon"
            ? { top: r.bottom + 4, left: r.left }
            : { top: r.bottom + 4, right: window.innerWidth - r.right },
        );
      }
      return next;
    });
  };

  const choose = (id: string) => {
    setOpen(false);
    if (showPicker && base) onSpawn(id, { base, mergeTarget });
    else onSpawn(id);
  };
  const chooseTerminal = () => { setOpen(false); onTerminal?.(); };

  return (
    <div className="agent-add">
      {variant === "icon" ? (
        <button ref={btnRef} className="icon-btn" title="Add agent" onClick={toggle}>+</button>
      ) : (
        <button ref={btnRef} className="btn-primary" onClick={toggle}>+ Agent ▾</button>
      )}
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="agent-menu" style={{ position: "fixed", ...coords }}>
            {AGENT_TYPES.map((a) => (
              <button key={a.id} onClick={() => choose(a.id)}>{a.label}</button>
            ))}
            {onTerminal && (
              <>
                <div className="agent-menu-sep" />
                <button onClick={chooseTerminal}>≳ New terminal</button>
              </>
            )}
            {showPicker && branches.length > 0 && (
              <>
                <div className="agent-menu-sep" />
                <div className="branch-picker">
                  <label className="branch-row">
                    <span>from</span>
                    <select
                      value={base}
                      onChange={(e) => setBase(e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                    >
                      {branches.map((b) => <option key={b} value={b}>{b}</option>)}
                    </select>
                  </label>
                  <label className="branch-row">
                    <span>into</span>
                    <select
                      value={mergeTarget}
                      onChange={(e) => setTargetOverride(e.target.value === base ? null : e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                    >
                      {branches.map((b) => <option key={b} value={b}>{b}</option>)}
                    </select>
                    {diverged && (
                      <button
                        type="button"
                        className="branch-reset"
                        title="Sync merge target to base"
                        onClick={(e) => { e.stopPropagation(); setTargetOverride(null); }}
                      >↺</button>
                    )}
                  </label>
                </div>
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Add styles**

Append to `ui/src/styles.css` (match the existing `.agent-menu` look at lines ~120-123):

```css
.agent-menu .branch-picker { display: flex; flex-direction: column; gap: 4px; padding: 4px 6px 2px; }
.agent-menu .branch-row { display: flex; align-items: center; gap: 8px; }
.agent-menu .branch-row > span { color: var(--o1); font-size: 11px; width: 30px; text-transform: lowercase; }
.agent-menu .branch-row select {
  flex: 1;
  background: var(--crust);
  border: 1px solid var(--s1);
  color: var(--text);
  border-radius: 6px;
  padding: 4px 6px;
  font-family: var(--sans);
  font-size: 12px;
}
.agent-menu .branch-row select:focus { outline: none; border-color: var(--blue); }
.agent-menu .branch-reset {
  background: transparent; border: none; color: var(--o1);
  cursor: pointer; padding: 0 2px; font-size: 13px;
}
.agent-menu .branch-reset:hover { color: var(--text); }
```

Widen the menu so two selects fit — find `.agent-menu { … min-width: 168px; … }` (styles.css ~120) and bump it:

```css
.agent-menu { min-width: 220px; }
```

> If `min-width` is set inline within the existing combined rule, edit that value in place rather than adding a duplicate rule.

- [ ] **Step 3: Typecheck + unit tests + build**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm test && pnpm build`
Expected: PASS — `tsc` clean (AgentAddMenu prop now matches `spawn`), Vitest green, Vite build succeeds.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/AgentAddMenu.tsx ui/src/styles.css
git commit -m "feat(ui): base/merge-target branch pickers in the Agent menu"
```

---

### Task 9: Manual end-to-end verification

**Files:** none (verification only).

- [ ] **Step 1: Build the app**

Run: `cargo build -p agency-app && cd ui && pnpm build`
Expected: both succeed.

- [ ] **Step 2: Run the app and exercise the feature**

Use the project's run flow (see `/run` skill or `pnpm tauri dev` from `ui/` if configured). Then verify:
- Open a project with ≥2 branches. Click **+ Agent** → the menu shows agent list plus **from** / **into** selects, both defaulting to the repo's current branch.
- Change **from** to another branch → **into** follows it (synced).
- Change **into** to a third branch → a `↺` appears; **from** is unchanged; clicking `↺` re-syncs **into** to **from**.
- Pick an agent. Confirm the agent's worktree branch is cut from the chosen **from** (`git -C <repo> log agent/<id> --oneline` shares history with the base) and that the run record stored the target (`merge_target`).
- Let the agent make a commit, then merge. Confirm the merge lands on the chosen **into** branch, not `main` (unless `main` was chosen).
- Open the menu via the **icon variant** (AgentFocus rail) → no branch selects, spawns as before.

- [ ] **Step 3: Final commit (if any verification fixups were needed)**

```bash
git add -A
git commit -m "chore: verification fixups for branch/merge-target selection"
```

---

## Self-Review

**Spec coverage:**
- "Expose base branch in UI" → Tasks 1, 5, 8 (list branches, API, **from** select).
- "Expose merge target in UI" → Tasks 2, 3, 4, 8 (persist, resolve, merge wiring, **into** select).
- "Synced by default unless merge target is edited" → Task 6 (`effectiveMergeTarget`) + Task 8 (`targetOverride` / `↺` reset).
- "Beautiful, matches aesthetic, no clutter" → Task 8 styles reuse `--crust/--s1/--blue` tokens and the `.agent-menu` shell; selectors are collapsed into the existing transient menu, not a new always-on panel; icon variant stays minimal.

**Placeholder scan:** No "TBD"/"handle errors"/"similar to" — every code step is concrete. The two soft spots are flagged explicitly with verification (the `Registry` test constructor in Task 2 Step 1, and any extra `create_run`/`Run{}` callers in Tasks 2/4) and have grep/build steps to catch them rather than assumptions.

**Type consistency:**
- `merge_target: Option<String>` (Rust) ↔ `mergeTarget?: string | null` (TS), serialized via Tauri's camelCase arg convention (`mergeTarget`).
- `ProjectBranches { current, branches }` identical in `git.rs` (camelCase serde) and `api.ts`.
- `opts?: { base: string; mergeTarget: string }` is the same shape in `createAgent`, `spawn`, `pendingSpawn`, and `AgentAddMenu.onSpawn`.
- `resolve_target(Option<&str>, &Path)` called with `run.merge_target.as_deref()` in both `merge_preview` and `merge_task`.
```
