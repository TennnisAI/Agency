# Agency Phase 5 Implementation Plan — Approve → Merge & Conflict Resolver

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the approve→merge flow: merge a task's `agent/<task-id>` branch into the repo's base branch from the UI; on a clean merge, commit and offer to remove the worktree; on conflicts, run a configurable **resolver agent** (default Claude Code) in the repo, guided by an authored merge-resolver skill, that reconciles the conflicts and commits.

**Architecture:** A deterministic `agency_core::merge` module (Tauri-free, tested) does the git mechanics: attempt the merge, detect conflicts, abort. `agency-app` exposes `merge_task` / `abort_merge_task` and a `resolve_merge` that spawns the resolver profile in the repo (reusing the PTY supervisor) with a prompt built from the merge-resolver skill. The frontend adds an "Approve & merge" modal.

**Tech Stack:** Rust (std `Command` → git, portable-pty via the existing supervisor), Tauri v2, React + TypeScript.

## Global Constraints

- **Local-only / zero first-party data collection:** no network in Agency's own code. The resolver agent may call its provider (user-configured), like any task.
- **Deterministic merge core is Tauri-free and tested** in `agency-core`. Commands stay thin.
- **Safety:** the merge runs in the repo's main working tree on the base branch. On conflict the merge is left IN PROGRESS (so the resolver can act); `abort_merge` cleanly backs it out. Destructive/again-able operations surface errors rather than panicking. Never force-push; never auto-remove a worktree without the user confirming.
- **Base branch:** detected as `main`, else `master` (helper `detect_base`). The task branch is `agent/<task-id>`.
- **Resolver:** default profile name `claude`; the user can change it. The resolver runs with `cwd` = repo root (NOT a worktree), mid-merge.
- **Merge-resolver skill:** authored as `skills/merge-resolver/SKILL.md` in the repo (portable markdown); its body is also used to build the runtime prompt.
- **Frontend↔Rust naming:** agency-app DTOs use `#[serde(rename_all = "camelCase")]`.
- TDD for the Rust merge module; commit after each green task.

---

## File Structure

```
crates/agency-core/
├── src/merge.rs       # NEW: merge / abort_merge / is_merging / detect_base
├── src/lib.rs         # MODIFY: pub mod merge;
└── tests/merge.rs     # NEW: clean merge, conflict detect, abort

crates/agency-app/
├── src/state.rs       # MODIFY: merge_task/abort_merge_task; resolve_merge (resolvers map) + resolver_input/status
├── src/commands.rs    # MODIFY: merge commands + MergeOutcomeDto + resolver commands
├── src/lib.rs         # MODIFY: register commands
└── tests/state.rs     # MODIFY: merge clean/conflict + resolver-spawn-in-repo tests

skills/merge-resolver/SKILL.md   # NEW: the authored resolver skill (also used for the runtime prompt)

ui/src/
├── api.ts                       # MODIFY: merge/resolver wrappers + types
├── components/MergeModal.tsx     # NEW: approve & merge modal
├── components/GitPanel.tsx       # MODIFY: "Approve & merge" button opens the modal
└── styles.css                    # MODIFY: modal styles
```

---

### Task 1: `agency_core::merge` — merge mechanics

**Files:**
- Create: `crates/agency-core/src/merge.rs`
- Modify: `crates/agency-core/src/lib.rs` (`pub mod merge;`)
- Test: `crates/agency-core/tests/merge.rs`

**Interfaces:**
- Produces:
  - `enum MergeOutcome { Clean { commit: String }, Conflicts { files: Vec<String> } }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`; `#[serde(rename_all = "camelCase")]` on variants is not needed but add `#[serde(tag = "kind", rename_all = "camelCase")]` on the enum so it serializes as `{ "kind": "clean", "commit": ... }` / `{ "kind": "conflicts", "files": [...] }`).
  - `fn detect_base(repo: &Path) -> anyhow::Result<String>` — returns `"main"` if that ref exists, else `"master"`, else errors.
  - `fn merge(repo: &Path, branch: &str, base: &str) -> anyhow::Result<MergeOutcome>` — checks out `base`, runs `git merge --no-ff <branch>`; on success returns `Clean { commit = HEAD }`; on conflict (unmerged files present) returns `Conflicts { files }` leaving the merge in progress; on other failure, errors.
  - `fn is_merging(repo: &Path) -> anyhow::Result<bool>` — true if `MERGE_HEAD` exists.
  - `fn abort_merge(repo: &Path) -> anyhow::Result<()>` — `git merge --abort`.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/merge.rs`:

```rust
use agency_core::merge::{self, MergeOutcome};
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
        "git {:?}",
        args
    );
}

/// Repo on `main` with one commit and a file `f.txt`.
fn init_repo(dir: &Path) {
    run(dir, &["init", "-q", "-b", "main"]);
    run(dir, &["config", "user.email", "t@e.com"]);
    run(dir, &["config", "user.name", "T"]);
    std::fs::write(dir.join("f.txt"), "line1\n").unwrap();
    run(dir, &["add", "-A"]);
    run(dir, &["commit", "-q", "-m", "init"]);
}

#[test]
fn clean_merge_commits_branch() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // Branch adds a new file; no conflict.
    run(dir.path(), &["checkout", "-q", "-b", "agent/x"]);
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "add new"]);
    run(dir.path(), &["checkout", "-q", "main"]);

    let outcome = merge::merge(dir.path(), "agent/x", "main").unwrap();
    match outcome {
        MergeOutcome::Clean { commit } => assert!(!commit.is_empty()),
        other => panic!("expected clean, got {other:?}"),
    }
    assert!(dir.path().join("new.txt").exists());
    assert!(!merge::is_merging(dir.path()).unwrap());
}

#[test]
fn conflicting_merge_detected_and_abortable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // Branch changes f.txt line1.
    run(dir.path(), &["checkout", "-q", "-b", "agent/y"]);
    std::fs::write(dir.path().join("f.txt"), "branch-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "branch edit"]);
    // main changes the same line differently.
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);

    let outcome = merge::merge(dir.path(), "agent/y", "main").unwrap();
    match outcome {
        MergeOutcome::Conflicts { files } => assert_eq!(files, vec!["f.txt".to_string()]),
        other => panic!("expected conflicts, got {other:?}"),
    }
    assert!(merge::is_merging(dir.path()).unwrap());

    merge::abort_merge(dir.path()).unwrap();
    assert!(!merge::is_merging(dir.path()).unwrap());
    // main's content restored.
    assert_eq!(std::fs::read_to_string(dir.path().join("f.txt")).unwrap(), "main-change\n");
}

#[test]
fn detect_base_finds_main() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    assert_eq!(merge::detect_base(dir.path()).unwrap(), "main");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test merge`
Expected: FAIL — module `merge` not found.

- [ ] **Step 3: Implement**

Create `crates/agency-core/src/merge.rs`:

```rust
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MergeOutcome {
    Clean { commit: String },
    Conflicts { files: Vec<String> },
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git").args(args).current_dir(repo).output()?)
}

fn git_ok(repo: &Path, args: &[&str]) -> Result<String> {
    let out = git(repo, args)?;
    if !out.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn ref_exists(repo: &Path, name: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", name])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn detect_base(repo: &Path) -> Result<String> {
    for candidate in ["main", "master"] {
        if ref_exists(repo, &format!("refs/heads/{candidate}")) {
            return Ok(candidate.to_string());
        }
    }
    bail!("no main/master branch found in repo")
}

pub fn is_merging(repo: &Path) -> Result<bool> {
    Ok(ref_exists(repo, "MERGE_HEAD"))
}

pub fn abort_merge(repo: &Path) -> Result<()> {
    git_ok(repo, &["merge", "--abort"])?;
    Ok(())
}

fn unmerged_files(repo: &Path) -> Result<Vec<String>> {
    let out = git_ok(repo, &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(out.lines().map(|l| l.to_string()).collect())
}

pub fn merge(repo: &Path, branch: &str, base: &str) -> Result<MergeOutcome> {
    git_ok(repo, &["checkout", base])?;
    let out = git(repo, &["merge", "--no-ff", branch])?;
    if out.status.success() {
        let commit = git_ok(repo, &["rev-parse", "HEAD"])?.trim().to_string();
        return Ok(MergeOutcome::Clean { commit });
    }
    // Distinguish conflicts from other failures.
    let conflicts = unmerged_files(repo)?;
    if !conflicts.is_empty() {
        Ok(MergeOutcome::Conflicts { files: conflicts })
    } else {
        bail!(
            "merge failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    }
}
```

Add to `crates/agency-core/src/lib.rs`:

```rust
pub mod merge;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test merge`
Expected: PASS (3 passed), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/merge.rs crates/agency-core/src/lib.rs crates/agency-core/tests/merge.rs
git commit -m "Add merge mechanics (clean/conflict/abort) to agency-core"
```

---

### Task 2: `agency-app` merge commands

**Files:**
- Modify: `crates/agency-app/src/state.rs`, `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- On `AppState`:
  - `fn merge_task(&self, task_id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome>` — resolves repo_path from the session, `base = detect_base(repo)`, `branch = format!("agent/{task_id}")`, calls `agency_core::merge::merge`.
  - `fn abort_merge_task(&self, task_id: &str) -> anyhow::Result<()>` — resolves repo_path, `abort_merge`.
  - (both need the repo_path; add a private `fn repo_path_for(&self, task_id) -> Result<PathBuf>` mirroring `worktree_path` but returning `session.repo_path`.)
- Commands (thin): `merge_task(task_id) -> MergeOutcome` (the enum serializes directly), `abort_merge_task(task_id) -> ()`. Register both.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-app/tests/state.rs`:

```rust
use agency_core::merge::MergeOutcome;

#[test]
fn merge_task_clean_merges_branch_into_base() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // existing helper: inits repo on default branch with a commit

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.register_profile(AgentProfile {
        name: "fake".into(),
        command: fake_agent_command(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    // Start a task → worktree on agent/<id>; make a non-conflicting commit in it.
    let info = state.start_task(&project.id, "p", "fake", "HEAD", |_| {}).unwrap();
    let wt = state.worktree_path(&info.task_id).unwrap();
    std::fs::write(wt.join("feature.txt"), "x\n").unwrap();
    std::process::Command::new("git").args(["add","-A"]).current_dir(&wt).status().unwrap();
    std::process::Command::new("git").args(["commit","-qm","feat"]).current_dir(&wt).status().unwrap();

    let outcome = state.merge_task(&info.task_id).unwrap();
    assert!(matches!(outcome, MergeOutcome::Clean { .. }));
    // feature.txt now on the repo's base branch working tree.
    assert!(repo.join("feature.txt").exists());
}
```

Note: the existing `init_repo` helper in tests/state.rs must produce a repo whose default branch is `main` (so `detect_base` finds it). If it currently uses `git init` (which may default to `master`), update the helper to `git init -q -b main` — make that change as part of this task and note it. The task's worktree branch is `agent/<id>`; merging it into `main` is the flow under test.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `merge_task` not found (and/or base detection if init_repo uses master).

- [ ] **Step 3: Implement**

In `crates/agency-app/src/state.rs`, add a repo-path resolver and the two methods:

```rust
    fn repo_path_for(&self, task_id: &str) -> anyhow::Result<std::path::PathBuf> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("unknown task: {task_id}"))?;
        Ok(session.repo_path.clone())
    }

    pub fn merge_task(&self, task_id: &str) -> anyhow::Result<agency_core::merge::MergeOutcome> {
        let repo = self.repo_path_for(task_id)?;
        let base = agency_core::merge::detect_base(&repo)?;
        let branch = format!("agent/{task_id}");
        agency_core::merge::merge(&repo, &branch, &base)
    }

    pub fn abort_merge_task(&self, task_id: &str) -> anyhow::Result<()> {
        let repo = self.repo_path_for(task_id)?;
        agency_core::merge::abort_merge(&repo)
    }
```

In `crates/agency-app/src/commands.rs`:

```rust
use agency_core::merge::MergeOutcome;

#[tauri::command]
pub fn merge_task(state: State<'_, AppState>, task_id: String) -> Result<MergeOutcome, String> {
    state.merge_task(&task_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn abort_merge_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.abort_merge_task(&task_id).map_err(|e| e.to_string())
}
```

Register both in `lib.rs`'s `generate_handler!`.

If `init_repo` in tests/state.rs used `git init` without `-b main`, change it to `git init -q -b main` (and remove any now-redundant `-q`). Run the full `agency-app` suite to confirm nothing else broke.

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-app`
Expected: PASS (all, including the new merge test), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs crates/agency-app/tests/state.rs
git commit -m "Add merge_task / abort_merge_task commands"
```

---

### Task 3: Resolver skill + resolver-agent spawn

**Files:**
- Create: `skills/merge-resolver/SKILL.md`
- Modify: `crates/agency-app/src/state.rs` (resolvers map + resolve/input/status), `crates/agency-app/src/commands.rs` (resolver commands), `crates/agency-app/src/lib.rs`
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- `skills/merge-resolver/SKILL.md` — the authored resolver instructions (see Step 1).
- On `AppState` (a new `resolvers: Mutex<HashMap<String, AgentHandle>>` field, initialized in `new()`):
  - `fn resolve_merge<F>(&self, task_id: &str, resolver_profile: &str, on_output: F) -> anyhow::Result<()>` where `F: Fn(Vec<u8>) + Send + 'static` — looks up repo_path + branch + base + the conflicted files (via `agency_core::merge`/`git`), builds the resolver prompt from the skill body, spawns the resolver profile in `cwd = repo` with that prompt, stores the handle in `resolvers[task_id]`.
  - `fn resolver_input(&self, task_id: &str, data: &[u8]) -> anyhow::Result<()>`
  - `fn resolver_status(&self, task_id: &str) -> anyhow::Result<agency_core::supervisor::AgentStatus>`
- Commands: `resolve_merge(task_id, resolver_profile, on_chunk: Channel<TerminalChunk>) -> ()`, `resolver_input(task_id, data: String) -> ()`, `resolver_status(task_id) -> StatusDto`. Register.
- The resolver prompt = the SKILL.md body with the branch/base/conflicted-files appended. Embed the skill body at compile time with `include_str!("../../../skills/merge-resolver/SKILL.md")`.

- [ ] **Step 1: Author the skill**

Create `skills/merge-resolver/SKILL.md`:

```markdown
---
name: merge-resolver
description: Resolve an in-progress git merge by reconciling conflict markers, preserving the intent of both sides, then committing.
---

# Merge Resolver

You are resolving an **in-progress git merge** in the current repository. A feature
branch is being merged into a base branch and there are conflicts.

## What to do

1. For each conflicted file, open it and find the conflict markers
   (`<<<<<<<`, `=======`, `>>>>>>>`).
2. Reconcile each conflict so the result preserves the **intent of both sides** —
   do not blindly pick one side; integrate the changes where they are compatible,
   and where they truly conflict, prefer the change that keeps the code correct and
   compiling. Remove all conflict markers.
3. `git add` each file once resolved.
4. When every conflict is resolved, run `git commit --no-edit` to complete the merge.
5. Print a short, plain-language summary of what each conflict was and how you
   resolved it.

## Rules

- Touch only the conflicted files. Do not modify unrelated files.
- Do not run `git push`. Do not amend or rebase other commits.
- If a conflict is genuinely ambiguous, resolve it the safest way and call it out
  explicitly in your summary so a human can double-check.
```

- [ ] **Step 2: Write the failing test (resolver spawns in the repo and streams)**

Append to `crates/agency-app/tests/state.rs`:

```rust
#[test]
fn resolve_merge_spawns_resolver_in_repo_and_streams() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    // Worker + a resolver profile that just echoes its cwd marker file and the prompt.
    state.register_profile(AgentProfile {
        name: "fake".into(),
        command: fake_agent_command(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    }).unwrap();
    state.register_profile(AgentProfile {
        name: "fakeresolver".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "echo RESOLVING; pwd; echo DONE".into()],
        env: vec![],
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.start_task(&project.id, "p", "fake", "HEAD", |_| {}).unwrap();

    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let b = buf.clone();
    state.resolve_merge(&info.task_id, "fakeresolver", move |bytes| {
        b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    }).unwrap();

    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if buf.lock().unwrap().contains("DONE") { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = buf.lock().unwrap().clone();
    assert!(out.contains("RESOLVING"), "got: {out}");
    // The resolver ran with cwd = repo root (its `pwd` contains the repo dir name).
    assert!(out.contains("repo"), "expected repo cwd in: {out}");
}
```

- [ ] **Step 3: Implement**

In `crates/agency-app/src/state.rs`:

Add the field to the struct and init it in `new()`:

```rust
    resolvers: Mutex<HashMap<String, AgentHandle>>,
```
```rust
            resolvers: Mutex::new(HashMap::new()),
```

Add the skill constant near the top:

```rust
const MERGE_RESOLVER_SKILL: &str = include_str!("../../../skills/merge-resolver/SKILL.md");
```

Add the methods:

```rust
    pub fn resolve_merge<F>(
        &self,
        task_id: &str,
        resolver_profile: &str,
        on_output: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let repo = self.repo_path_for(task_id)?;
        let base = agency_core::merge::detect_base(&repo).unwrap_or_else(|_| "main".to_string());
        let branch = format!("agent/{task_id}");
        let conflicts = agency_core::git::status(&repo)
            .map(|cs| {
                cs.into_iter()
                    .filter(|c| c.index == "U" || c.worktree == "U")
                    .map(|c| c.path)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let prompt = format!(
            "{skill}\n\n## This merge\n- Base branch: {base}\n- Feature branch: {branch}\n- Conflicted files: {files}\n",
            skill = MERGE_RESOLVER_SKILL,
            files = if conflicts.is_empty() { "(detect with `git status`)".to_string() } else { conflicts.join(", ") },
        );

        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(resolver_profile)?
                .ok_or_else(|| anyhow!("unknown resolver profile: {resolver_profile}"))?
        };

        let handle = spawn_agent(&profile, &repo, &prompt, on_output)?;
        self.resolvers.lock().unwrap().insert(task_id.to_string(), handle);
        Ok(())
    }

    pub fn resolver_input(&self, task_id: &str, data: &[u8]) -> anyhow::Result<()> {
        let resolvers = self.resolvers.lock().unwrap();
        let handle = resolvers
            .get(task_id)
            .ok_or_else(|| anyhow!("no resolver for task: {task_id}"))?;
        handle.write_input(data)
    }

    pub fn resolver_status(&self, task_id: &str) -> anyhow::Result<agency_core::supervisor::AgentStatus> {
        let resolvers = self.resolvers.lock().unwrap();
        let handle = resolvers
            .get(task_id)
            .ok_or_else(|| anyhow!("no resolver for task: {task_id}"))?;
        Ok(handle.status())
    }
```

Note: the resolver profile's env does NOT get provider injection in this minimal version (the resolver test uses /bin/sh). If you want the real `claude` resolver to receive provider env, mirror the env-merge from `start_task` — do so only if it keeps the test green; otherwise leave a clear `// TODO(phase6): inject provider env into resolver` and note it in the report.

In `crates/agency-app/src/commands.rs` add the three resolver commands (mirroring `start_task`/`send_input`/`task_status` but resolver-flavored), using the existing `TerminalChunk`/`StatusDto`/`status_dto`. Register all three in `lib.rs`.

```rust
#[tauri::command]
pub fn resolve_merge(
    state: State<'_, AppState>,
    task_id: String,
    resolver_profile: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    state
        .resolve_merge(&task_id, &resolver_profile, move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolver_input(state: State<'_, AppState>, task_id: String, data: String) -> Result<(), String> {
    state.resolver_input(&task_id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolver_status(state: State<'_, AppState>, task_id: String) -> Result<StatusDto, String> {
    state.resolver_status(&task_id).map(status_dto).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-app` and `cargo build -p agency-app`.
Expected: PASS (incl. `resolve_merge_spawns_resolver_in_repo_and_streams`), warning-free.

- [ ] **Step 5: Commit**

```bash
git add skills/merge-resolver/SKILL.md crates/agency-app/src/state.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs crates/agency-app/tests/state.rs
git commit -m "Add merge-resolver skill and resolver-agent spawn"
```

---

### Task 4: Frontend — merge API + Approve & merge modal

**Files:**
- Modify: `ui/src/api.ts`
- Create: `ui/src/components/MergeModal.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- `api.ts`: `MergeOutcome = { kind: "clean"; commit: string } | { kind: "conflicts"; files: string[] }`; wrappers `mergeTask(taskId)`, `abortMergeTask(taskId)`, `resolveMerge(taskId, resolverProfile, onBytes)` (Channel, like `startTask`), `resolverInput(taskId, data)`, `resolverStatus(taskId)`.
- `MergeModal({ taskId, onClose })`: on open, calls `mergeTask`. If `clean`, shows success + commit + a Close button (and the parent can offer "remove worktree"). If `conflicts`, lists the files and shows two actions: "Resolve with agent" (calls `resolveMerge` with profile `claude`, streams into an xterm terminal in the modal, polls `resolverStatus`; when the resolver exits, shows "Re-check / Confirm" which calls `mergeTask` again) and "Abort merge" (calls `abortMergeTask`, closes).

- [ ] **Step 1: Add API wrappers**

Append to `ui/src/api.ts`:

```ts
export type MergeOutcome =
  | { kind: "clean"; commit: string }
  | { kind: "conflicts"; files: string[] };

export const mergeTask = (taskId: string) => invoke<MergeOutcome>("merge_task", { taskId });
export const abortMergeTask = (taskId: string) =>
  invoke<void>("abort_merge_task", { taskId });
export const resolverInput = (taskId: string, data: string) =>
  invoke<void>("resolver_input", { taskId, data });
export const resolverStatus = (taskId: string) =>
  invoke<StatusDto>("resolver_status", { taskId });

export function resolveMerge(
  taskId: string,
  resolverProfile: string,
  onBytes: (bytes: Uint8Array) => void,
): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (msg) => onBytes(b64ToBytes(msg.b64));
  return invoke<void>("resolve_merge", { taskId, resolverProfile, onChunk });
}
```

- [ ] **Step 2: Create MergeModal**

Create `ui/src/components/MergeModal.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import {
  MergeOutcome,
  abortMergeTask,
  mergeTask,
  resolveMerge,
  resolverStatus,
} from "../api";

export default function MergeModal({ taskId, onClose }: { taskId: string; onClose: () => void }) {
  const [outcome, setOutcome] = useState<MergeOutcome | null>(null);
  const [error, setError] = useState("");
  const [resolving, setResolving] = useState(false);
  const [resolverDone, setResolverDone] = useState(false);
  const termRef = useRef<HTMLDivElement>(null);

  async function attempt() {
    setError("");
    try {
      setOutcome(await mergeTask(taskId));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    attempt();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function startResolver() {
    setResolving(true);
    setResolverDone(false);
    const term = new Terminal({ convertEol: true, fontSize: 12 });
    const fit = new FitAddon();
    term.loadAddon(fit);
    if (termRef.current) {
      term.open(termRef.current);
      requestAnimationFrame(() => {
        try {
          fit.fit();
        } catch {
          /* not laid out */
        }
      });
    }
    resolveMerge(taskId, "claude", (bytes) => term.write(bytes)).catch((e) =>
      setError(String(e)),
    );
    const timer = window.setInterval(async () => {
      try {
        const s = await resolverStatus(taskId);
        if (s.state === "exited" || s.state === "crashed") {
          setResolverDone(true);
          window.clearInterval(timer);
        }
      } catch {
        /* not started yet */
      }
    }, 1000);
  }

  return (
    <div className="settings-overlay">
      <div className="merge-modal">
        <div className="settings-head">
          <h2>Approve &amp; merge</h2>
          <button onClick={onClose}>Close</button>
        </div>
        {error && <div className="git-error">{error}</div>}

        {!outcome && !error && <p>Merging…</p>}

        {outcome?.kind === "clean" && (
          <div>
            <p className="merge-ok">✓ Merged cleanly.</p>
            <code>{outcome.commit.slice(0, 10)}</code>
            <div className="git-actions">
              <button onClick={onClose}>Done</button>
            </div>
          </div>
        )}

        {outcome?.kind === "conflicts" && (
          <div>
            <p className="merge-warn">Conflicts in {outcome.files.length} file(s):</p>
            <ul className="profile-list">
              {outcome.files.map((f) => (
                <li key={f}>
                  <code>{f}</code>
                </li>
              ))}
            </ul>
            {!resolving ? (
              <div className="git-actions">
                <button onClick={startResolver}>Resolve with agent</button>
                <button onClick={() => abortMergeTask(taskId).then(onClose)}>Abort merge</button>
              </div>
            ) : (
              <div className="resolver">
                <div className="terminal merge-term" ref={termRef} />
                <div className="git-actions">
                  <button disabled={!resolverDone} onClick={attempt}>
                    Re-check merge
                  </button>
                  <button onClick={() => abortMergeTask(taskId).then(onClose)}>Abort merge</button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Styles**

Append to `ui/src/styles.css`:

```css
.merge-modal { background: #14161a; border: 1px solid #2a2e36; border-radius: 12px; padding: 16px; width: 620px; max-height: 86vh; overflow: auto; display: flex; flex-direction: column; gap: 12px; }
.merge-ok { color: #a6e3a1; }
.merge-warn { color: #f9e2af; }
.merge-term { min-height: 280px; }
```

- [ ] **Step 4: Build**

Run: `pnpm --dir ui build` and `pnpm --dir ui test`.
Expected: clean build (MergeModal not mounted yet), vitest green.

- [ ] **Step 5: Commit**

```bash
git add ui/src/api.ts ui/src/components/MergeModal.tsx ui/src/styles.css
git commit -m "Add merge API and Approve & merge modal"
```

---

### Task 5: Wire the Approve & merge button into the git panel

**Files:**
- Modify: `ui/src/components/GitPanel.tsx`

**Interfaces:**
- `GitPanel` gains a button "Approve & merge" that opens `MergeModal` for its `taskId`.

- [ ] **Step 1: Mount the modal from GitPanel**

In `ui/src/components/GitPanel.tsx`, import `MergeModal` and `useState` (already imported), add a `showMerge` state, render an "Approve & merge" button near the commit/push actions, and conditionally render `<MergeModal taskId={taskId} onClose={() => setShowMerge(false)} />`.

Concretely: add `import MergeModal from "./MergeModal";`, add `const [showMerge, setShowMerge] = useState(false);`, add a button in the `.git-commit` actions area:

```tsx
          <button onClick={() => setShowMerge(true)}>Approve &amp; merge</button>
```

and at the end of the returned JSX (before the closing `</div>` of `.git-panel`):

```tsx
      {showMerge && <MergeModal taskId={taskId} onClose={() => setShowMerge(false)} />}
```

- [ ] **Step 2: Build + test**

Run: `pnpm --dir ui build` and `pnpm --dir ui test`.
Expected: clean build, vitest green.

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/GitPanel.tsx
git commit -m "Add Approve & merge entry point to the git panel"
```

---

## Self-Review

**Spec coverage (Phase 5 scope):**
- Clean merge → commit → Task 1 (core), Task 2 (command), Task 4 (modal success). ✓
- Conflict detection + abort → Task 1, Task 2, Task 4 (abort). ✓
- Resolver agent runs in repo with the authored skill → Task 3 (skill + spawn, tested with fake resolver), Task 4 (modal terminal). ✓
- Approve & merge entry point → Task 5. ✓
- Deferred / addressed-when-they-arise (per the design): the full designed modal visuals (timeline, per-conflict status) — Phase 6 design pass; "Confirm merge / Keep worktree" nuance maps to "Done" + the existing stop_task/discard; provider-env injection into the resolver (noted TODO).

**Placeholder scan:** No TBD; every code step is complete. The resolver-env TODO is explicitly flagged for Phase 6, not a silent gap.

**Type consistency:** `MergeOutcome` is one Serde-tagged enum in `agency_core::merge`, surfaced unchanged through `merge_task`; the TS `MergeOutcome` mirrors the `{kind, ...}` shape. Resolver commands reuse `TerminalChunk`/`StatusDto`. `resolve_merge`/`resolver_input`/`resolver_status` names match across state ↔ commands ↔ api.

**Notes for the executor:**
- The real `claude` resolver requires Claude Code installed + authed; tests use `/bin/sh` and the fake agent — they verify spawn/stream/cwd, not actual conflict resolution.
- `init_repo` in tests/state.rs must create a `main` branch (`git init -b main`) so `detect_base` works; update it in Task 2 if needed.
- The merge runs in the repo's main working tree; if that tree is dirty or on another branch, `git checkout base` may fail and surface as an error in the modal — acceptable for this phase.
