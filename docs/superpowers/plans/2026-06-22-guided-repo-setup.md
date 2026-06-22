# Guided Repo Setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a user adds a folder or spawns an agent in a project that isn't agent-ready, walk them through making it ready (init the repo and/or create the first commit) instead of failing with a dead-end error.

**Architecture:** A new `agency-core::setup` module computes live git readiness (`RepoReadiness`) and performs the mechanical fixes (`init_repo`, `write_default_gitignore`, `initial_commit`). Three Tauri commands (`inspect_repo`, `init_repo`, `commit_repo`) expose them. The React UI gains one adaptive `RepoSetupDialog` driven by readiness, wired into both the add-project flow (`ProjectTree`) and the spawn flow (`runs.tsx`).

**Tech Stack:** Rust (anyhow, serde, `std::process::Command` shelling to `git`), Tauri v2 commands, React + TypeScript, Vite/Vitest.

## Global Constraints

- Readiness is **computed live, never persisted** on the `Project` record.
- A worktree contains only what is in `HEAD`; zero commits = cannot spawn (blocking), dirty tree = optional commit (informational).
- `write_default_gitignore` **never overwrites** an existing `.gitignore`.
- `initial_commit` falls back to `git commit --allow-empty` when nothing is staged.
- Do not force a default branch name; use the user's git config.
- Default `.gitignore` contents (verbatim): `node_modules/`, `.env`, `dist/`, `target/`, `.DS_Store` (one per line, trailing newline).
- Rust git calls shell out via `std::process::Command::new("git")` with `.current_dir(path)`, matching `git.rs`.
- Tauri commands return `Result<T, String>` and map errors with `.map_err(|e| e.to_string())`, matching `commands.rs`.
- DTOs are manual structs with `#[serde(rename_all = "camelCase")]`, matching `StatusDto`.

---

### Task 1: Core readiness detection (`RepoReadiness` + `repo_readiness`)

**Files:**
- Create: `crates/agency-core/src/setup.rs`
- Modify: `crates/agency-core/src/lib.rs:7` (add `pub mod setup;`)
- Test: `crates/agency-core/tests/setup.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum RepoReadiness { NotARepo, NoCommits { stageable: bool }, Ready { dirty: bool } }` (derives `Debug, Clone, PartialEq, Serialize, Deserialize`)
  - `pub fn repo_readiness(path: &std::path::Path) -> RepoReadiness`

- [ ] **Step 1: Declare the module**

In `crates/agency-core/src/lib.rs`, add after `pub mod registry;` (keep alphabetical):

```rust
pub mod setup;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/agency-core/tests/setup.rs`:

```rust
use agency_core::setup::{repo_readiness, RepoReadiness};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
        "git {:?}",
        args
    );
}

fn init_bare_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@e.com"]);
    git(dir, &["config", "user.name", "T"]);
}

#[test]
fn plain_folder_is_not_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NotARepo);
}

#[test]
fn repo_with_no_commits_and_files_is_stageable() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: true });
}

#[test]
fn empty_repo_with_no_commits_is_not_stageable() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: false });
}

#[test]
fn committed_clean_repo_is_ready_not_dirty() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn committed_repo_with_changes_is_ready_dirty() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    std::fs::write(dir.path().join("b.txt"), "new\n").unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: true });
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p agency-core --test setup`
Expected: FAIL — `setup` module / `repo_readiness` not found (compile error).

- [ ] **Step 4: Implement `repo_readiness`**

Create `crates/agency-core/src/setup.rs`:

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RepoReadiness {
    NotARepo,
    NoCommits { stageable: bool },
    Ready { dirty: bool },
}

/// Run a git command in `dir`, returning (success, stdout).
fn git(dir: &Path, args: &[&str]) -> std::io::Result<(bool, String)> {
    let out = Command::new("git").args(args).current_dir(dir).output()?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).to_string()))
}

/// Inspect a folder and classify how ready it is to host agent worktrees.
/// Worktrees branch from `HEAD`, so a repo needs at least one commit to be `Ready`.
pub fn repo_readiness(path: &Path) -> RepoReadiness {
    let inside = git(path, &["rev-parse", "--is-inside-work-tree"]);
    if !matches!(inside, Ok((true, _))) {
        return RepoReadiness::NotARepo;
    }
    // HEAD resolves only when at least one commit exists.
    let has_head = matches!(git(path, &["rev-parse", "--verify", "HEAD"]), Ok((true, _)));
    if !has_head {
        // Anything that `git add -A` would stage means a non-empty initial commit is possible.
        let stageable = match git(path, &["status", "--porcelain"]) {
            Ok((true, out)) => !out.trim().is_empty(),
            _ => false,
        };
        return RepoReadiness::NoCommits { stageable };
    }
    let dirty = match git(path, &["status", "--porcelain"]) {
        Ok((true, out)) => !out.trim().is_empty(),
        _ => false,
    };
    RepoReadiness::Ready { dirty }
}
```

(`Result`/`anyhow` import is unused by Task 1 alone but consumed in Task 2; if the build warns, leave it — Task 2 adds the user.) To avoid an unused-import warning in the interim, omit the `use anyhow::Result;` line for now and add it in Task 2.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p agency-core --test setup`
Expected: PASS (5 passed).

- [ ] **Step 6: Commit**

```bash
git add crates/agency-core/src/lib.rs crates/agency-core/src/setup.rs crates/agency-core/tests/setup.rs
git commit -m "Add repo_readiness detection for project setup"
```

---

### Task 2: Core mechanical fixes (`init_repo`, `write_default_gitignore`, `initial_commit`)

**Files:**
- Modify: `crates/agency-core/src/setup.rs`
- Test: `crates/agency-core/tests/setup.rs`

**Interfaces:**
- Consumes: `git` helper and `RepoReadiness` from Task 1.
- Produces:
  - `pub fn init_repo(path: &Path) -> Result<()>`
  - `pub fn write_default_gitignore(path: &Path) -> Result<()>`
  - `pub fn initial_commit(path: &Path, add_gitignore: bool) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

Append to `crates/agency-core/tests/setup.rs`:

```rust
use agency_core::setup::{init_repo, initial_commit, write_default_gitignore};

#[test]
fn init_repo_makes_a_repo_with_no_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    assert!(matches!(repo_readiness(dir.path()), RepoReadiness::NoCommits { .. }));
}

#[test]
fn initial_commit_with_files_makes_repo_ready() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    initial_commit(dir.path(), false).unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn initial_commit_on_empty_folder_uses_allow_empty() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    initial_commit(dir.path(), false).unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn initial_commit_with_gitignore_writes_and_excludes() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    std::fs::create_dir(dir.path().join("node_modules")).unwrap();
    std::fs::write(dir.path().join("node_modules/x.js"), "x\n").unwrap();
    std::fs::write(dir.path().join("keep.txt"), "k\n").unwrap();
    initial_commit(dir.path(), true).unwrap();

    let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gi.contains("node_modules/"));
    // node_modules excluded → tree clean, keep.txt + .gitignore committed.
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn write_default_gitignore_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), "custom\n").unwrap();
    write_default_gitignore(dir.path()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(), "custom\n");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agency-core --test setup`
Expected: FAIL — `init_repo` / `initial_commit` / `write_default_gitignore` not found.

- [ ] **Step 3: Implement the three functions**

Add `use anyhow::{bail, Result};` to the top of `crates/agency-core/src/setup.rs` (replacing the bare `use anyhow::Result;` if present), then append:

```rust
const DEFAULT_GITIGNORE: &str = "node_modules/\n.env\ndist/\ntarget/\n.DS_Store\n";

/// Run a git command in `dir`, returning Err with stderr on failure.
fn git_checked(dir: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git").args(args).current_dir(dir).output()?;
    if !out.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// `git init` in `path`. Uses the user's configured default branch name.
pub fn init_repo(path: &Path) -> Result<()> {
    git_checked(path, &["init"])
}

/// Write a sensible default `.gitignore`, but never overwrite an existing one.
pub fn write_default_gitignore(path: &Path) -> Result<()> {
    let gi = path.join(".gitignore");
    if !gi.exists() {
        std::fs::write(&gi, DEFAULT_GITIGNORE)?;
    }
    Ok(())
}

/// Stage everything and make the initial commit. Optionally writes a default
/// `.gitignore` first. Falls back to `--allow-empty` when nothing is staged
/// (empty or fully-ignored folder), so the repo still gains a usable `HEAD`.
pub fn initial_commit(path: &Path, add_gitignore: bool) -> Result<()> {
    if add_gitignore {
        write_default_gitignore(path)?;
    }
    git_checked(path, &["add", "-A"])?;
    // Nothing staged → empty commit so HEAD exists and worktrees can branch.
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(path)
        .status()?;
    let mut commit_args = vec!["commit", "-m", "Initial commit"];
    if staged.success() {
        // exit 0 from --quiet means no staged changes.
        commit_args.push("--allow-empty");
    }
    git_checked(path, &commit_args)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-core --test setup`
Expected: PASS (10 passed total).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/setup.rs crates/agency-core/tests/setup.rs
git commit -m "Add init_repo, default .gitignore, and initial_commit helpers"
```

---

### Task 3: Tauri commands + relax `validate_repo`

**Files:**
- Modify: `crates/agency-app/src/commands.rs` (add three commands + `ReadinessDto`)
- Modify: `crates/agency-app/src/state.rs` (add `inspect_repo`/`init_repo`/`commit_repo` passthroughs; relax `validate_repo`)
- Modify: `crates/agency-app/src/lib.rs:18` (register the three commands)
- Test: `crates/agency-app/tests/project_lifecycle.rs` (or new `crates/agency-app/tests/setup_cmds.rs`)

**Interfaces:**
- Consumes: `agency_core::setup::{repo_readiness, init_repo, initial_commit, RepoReadiness}`.
- Produces (Tauri commands):
  - `inspect_repo(repo_path: String) -> Result<ReadinessDto, String>`
  - `init_repo(repo_path: String) -> Result<(), String>`
  - `commit_repo(repo_path: String, add_gitignore: bool) -> Result<(), String>`
  - `ReadinessDto { state: String, stageable: bool, dirty: bool }` (camelCase serde) where `state` ∈ `"notARepo" | "noCommits" | "ready"`.

- [ ] **Step 1: Add the readiness inspection to `AppState`**

In `crates/agency-app/src/state.rs`, add these methods to the `impl AppState` block (near `add_project`):

```rust
pub fn inspect_repo(&self, repo_path: &Path) -> agency_core::setup::RepoReadiness {
    agency_core::setup::repo_readiness(repo_path)
}

pub fn init_repo(&self, repo_path: &Path) -> Result<()> {
    agency_core::setup::init_repo(repo_path)
}

pub fn commit_repo(&self, repo_path: &Path, add_gitignore: bool) -> Result<()> {
    agency_core::setup::initial_commit(repo_path, add_gitignore)
}
```

- [ ] **Step 2: Relax `validate_repo` to reject only "not a repo"**

In `crates/agency-app/src/state.rs`, replace the body of `validate_repo` (currently bails on both no-repo and no-commits) with one that only blocks the unrecoverable case — a gated (no-commits) project is now allowed:

```rust
/// A project path is addable as long as it is a git repository. A repo with no
/// commits is allowed (it lands "gated": the UI walks the user through the
/// first commit before any agent can spawn). Non-repos are rejected because the
/// UI runs `init_repo` *before* calling `add_project`.
fn validate_repo(repo_path: &Path) -> Result<()> {
    if !repo_path.exists() {
        bail!("{} does not exist", repo_path.display());
    }
    if matches!(
        agency_core::setup::repo_readiness(repo_path),
        agency_core::setup::RepoReadiness::NotARepo
    ) {
        bail!("{} is not a git repository", repo_path.display());
    }
    Ok(())
}
```

Remove the now-unused inner `git` closure and the `--is-inside-work-tree` / `HEAD` checks it replaced.

- [ ] **Step 3: Write the failing command-layer test**

Append to `crates/agency-app/tests/project_lifecycle.rs`. The sibling tests construct state inline as `AppState::new(&dir.path().join("agency.db"))` — match that pattern exactly (there is no shared helper):

```rust
#[test]
fn add_project_allows_repo_without_commits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // A git repo with NO commits — previously rejected, now must be addable (gated).
    std::process::Command::new("git").args(["init", "-q"]).current_dir(&repo).output().unwrap();

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let p = state.add_project("repo", &repo).unwrap();
    assert_eq!(p.name, "repo");

    // Inspection reports it as not-ready (no commits yet).
    assert!(matches!(
        state.inspect_repo(&repo),
        agency_core::setup::RepoReadiness::NoCommits { .. }
    ));
}
```

Note: `project_lifecycle.rs` uses `use agency_app_lib::AppState;`. Add `agency-core` to the test's available crates if not already a dev-dependency of `agency-app` (check `crates/agency-app/Cargo.toml`; the sibling tests already reference `agency_core`, so it should resolve).

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p agency-app --test project_lifecycle add_project_allows_repo_without_commits`
Expected: FAIL — currently `add_project` bails with "has no commits yet".

- [ ] **Step 5: Add the Tauri commands**

In `crates/agency-app/src/commands.rs`, add the DTO near `StatusDto`:

```rust
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessDto {
    pub state: String,
    pub stageable: bool,
    pub dirty: bool,
}

fn readiness_dto(r: agency_core::setup::RepoReadiness) -> ReadinessDto {
    use agency_core::setup::RepoReadiness::*;
    match r {
        NotARepo => ReadinessDto { state: "notARepo".into(), stageable: false, dirty: false },
        NoCommits { stageable } => ReadinessDto { state: "noCommits".into(), stageable, dirty: false },
        Ready { dirty } => ReadinessDto { state: "ready".into(), stageable: false, dirty },
    }
}
```

Then add the three commands (anywhere among the other `#[tauri::command]` fns):

```rust
#[tauri::command]
pub fn inspect_repo(state: State<'_, AppState>, repo_path: String) -> Result<ReadinessDto, String> {
    Ok(readiness_dto(state.inspect_repo(std::path::Path::new(&repo_path))))
}

#[tauri::command]
pub fn init_repo(state: State<'_, AppState>, repo_path: String) -> Result<(), String> {
    state.init_repo(std::path::Path::new(&repo_path)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn commit_repo(
    state: State<'_, AppState>,
    repo_path: String,
    add_gitignore: bool,
) -> Result<(), String> {
    state
        .commit_repo(std::path::Path::new(&repo_path), add_gitignore)
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register the commands**

In `crates/agency-app/src/lib.rs`, add inside `generate_handler![ ... ]` (after `commands::add_project,`):

```rust
            commands::inspect_repo,
            commands::init_repo,
            commands::commit_repo,
```

- [ ] **Step 7: Run tests + build to verify**

Run: `cargo test -p agency-app --test project_lifecycle add_project_allows_repo_without_commits`
Expected: PASS.
Run: `cargo build -p agency-app`
Expected: builds cleanly (no unused-import / dead-code errors from the `validate_repo` edit).

- [ ] **Step 8: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/state.rs crates/agency-app/src/lib.rs crates/agency-app/tests/project_lifecycle.rs
git commit -m "Expose inspect_repo/init_repo/commit_repo; allow gated projects"
```

---

### Task 4: TypeScript API bindings

**Files:**
- Modify: `ui/src/api.ts`

**Interfaces:**
- Consumes: the three Tauri commands from Task 3.
- Produces:
  - `export type RepoReadiness = { state: "notARepo" | "noCommits" | "ready"; stageable: boolean; dirty: boolean }`
  - `inspectRepo(repoPath: string): Promise<RepoReadiness>`
  - `initRepo(repoPath: string): Promise<void>`
  - `commitRepo(repoPath: string, addGitignore: boolean): Promise<void>`

- [ ] **Step 1: Add the bindings**

In `ui/src/api.ts`, after the `addProject` export, add:

```ts
export type RepoReadiness = {
  state: "notARepo" | "noCommits" | "ready";
  stageable: boolean;
  dirty: boolean;
};

export const inspectRepo = (repoPath: string) =>
  invoke<RepoReadiness>("inspect_repo", { repoPath });
export const initRepo = (repoPath: string) =>
  invoke<void>("init_repo", { repoPath });
export const commitRepo = (repoPath: string, addGitignore: boolean) =>
  invoke<void>("commit_repo", { repoPath, addGitignore });
```

- [ ] **Step 2: Typecheck**

Run: `cd ui && pnpm exec tsc --noEmit`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add ui/src/api.ts
git commit -m "Add TS bindings for repo setup commands"
```

---

### Task 5: `RepoSetupDialog` component

**Files:**
- Create: `ui/src/components/RepoSetupDialog.tsx`
- Test: `ui/src/components/RepoSetupDialog.test.tsx`

**Interfaces:**
- Consumes: `RepoReadiness` from `api.ts`.
- Produces: default export `RepoSetupDialog` with props:
  ```ts
  {
    readiness: RepoReadiness;
    context: "add" | "spawn";
    onResolved: () => void;   // repo is now ready (or user committed/spawned-anyway)
    onCancel: () => void;     // user dismissed; caller decides what that means
    repoPath: string;
  }
  ```
  Internally calls `initRepo` / `commitRepo` and re-inspects after init so a `NotARepo` flows into the commit prompt.

- [ ] **Step 1: Write the failing test**

Create `ui/src/components/RepoSetupDialog.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import RepoSetupDialog from "./RepoSetupDialog";

const initRepo = vi.fn();
const commitRepo = vi.fn();
const inspectRepo = vi.fn();

vi.mock("../api", () => ({
  initRepo: (...a: unknown[]) => initRepo(...a),
  commitRepo: (...a: unknown[]) => commitRepo(...a),
  inspectRepo: (...a: unknown[]) => inspectRepo(...a),
}));

beforeEach(() => {
  initRepo.mockReset().mockResolvedValue(undefined);
  commitRepo.mockReset().mockResolvedValue(undefined);
  inspectRepo.mockReset();
});

describe("RepoSetupDialog", () => {
  it("shows the init prompt for a non-repo", () => {
    render(
      <RepoSetupDialog
        readiness={{ state: "notARepo", stageable: false, dirty: false }}
        context="add" repoPath="/x" onResolved={() => {}} onCancel={() => {}}
      />,
    );
    expect(screen.getByText(/No git repository found/i)).toBeTruthy();
  });

  it("init then commit resolves, calling both APIs", async () => {
    inspectRepo.mockResolvedValue({ state: "noCommits", stageable: true, dirty: false });
    const onResolved = vi.fn();
    render(
      <RepoSetupDialog
        readiness={{ state: "notARepo", stageable: false, dirty: false }}
        context="add" repoPath="/x" onResolved={onResolved} onCancel={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Initialize/i }));
    await waitFor(() => expect(initRepo).toHaveBeenCalledWith("/x"));
    // now in commit step
    fireEvent.click(await screen.findByRole("button", { name: /Create initial commit|Commit/i }));
    await waitFor(() => expect(commitRepo).toHaveBeenCalled());
    await waitFor(() => expect(onResolved).toHaveBeenCalled());
  });

  it("dirty + spawn context offers Spawn anyway", () => {
    const onResolved = vi.fn();
    render(
      <RepoSetupDialog
        readiness={{ state: "ready", stageable: false, dirty: true }}
        context="spawn" repoPath="/x" onResolved={onResolved} onCancel={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Spawn anyway/i }));
    expect(onResolved).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && pnpm exec vitest run src/components/RepoSetupDialog.test.tsx`
Expected: FAIL — module `./RepoSetupDialog` not found.

- [ ] **Step 3: Implement the component**

Create `ui/src/components/RepoSetupDialog.tsx`:

```tsx
import { useState } from "react";
import { RepoReadiness, initRepo, commitRepo, inspectRepo } from "../api";

type Props = {
  readiness: RepoReadiness;
  context: "add" | "spawn";
  repoPath: string;
  onResolved: () => void;
  onCancel: () => void;
};

export default function RepoSetupDialog({ readiness, context, repoPath, onResolved, onCancel }: Props) {
  const [current, setCurrent] = useState<RepoReadiness>(readiness);
  const [addGitignore, setAddGitignore] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function doInit() {
    setBusy(true); setError("");
    try {
      await initRepo(repoPath);
      // After init the folder always has no commits — advance to the commit step.
      setCurrent(await inspectRepo(repoPath));
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function doCommit() {
    setBusy(true); setError("");
    try {
      await commitRepo(repoPath, addGitignore);
      onResolved();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  let title = "";
  let body: React.ReactNode = null;
  let actions: React.ReactNode = null;

  if (current.state === "notARepo") {
    title = "Set up this folder for agents";
    body = "No git repository found. Agency runs each agent in an isolated git worktree, so this folder needs to be a repository. Initialize one now?";
    actions = <button className="btn-primary" disabled={busy} onClick={doInit}>Initialize repository</button>;
  } else if (current.state === "noCommits") {
    title = "Create an initial commit";
    body = "Agency needs at least one commit — each agent starts from your latest commit. Create the initial commit now?";
    actions = <button className="btn-primary" disabled={busy} onClick={doCommit}>Create initial commit</button>;
  } else if (current.state === "ready" && current.dirty) {
    title = "Uncommitted changes";
    body = "Agents work from your last commit, so they won't see your current uncommitted changes until you commit them. Commit now?";
    actions = (
      <>
        <button className="btn-secondary" disabled={busy} onClick={onResolved}>
          {context === "spawn" ? "Spawn anyway" : "Add anyway"}
        </button>
        <button className="btn-primary" disabled={busy} onClick={doCommit}>Commit now</button>
      </>
    );
  } else {
    // Already ready & clean — nothing to do.
    onResolved();
    return null;
  }

  const showGitignore = current.state === "noCommits" || (current.state === "ready" && current.dirty);

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal confirm" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button className="modal-x" onClick={onCancel}>✕</button>
        </div>
        <div className="modal-body">
          {body}
          {showGitignore && (
            <label className="setup-gitignore">
              <input type="checkbox" checked={addGitignore} onChange={(e) => setAddGitignore(e.target.checked)} />
              Add a .gitignore (node_modules, .env, dist, target, .DS_Store)
            </label>
          )}
          {error && <div className="git-error">{error}</div>}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" disabled={busy} onClick={onCancel}>Cancel</button>
          {actions}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && pnpm exec vitest run src/components/RepoSetupDialog.test.tsx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/RepoSetupDialog.tsx ui/src/components/RepoSetupDialog.test.tsx
git commit -m "Add adaptive RepoSetupDialog component"
```

---

### Task 6: Wire add-project flow + sidebar badge

**Files:**
- Modify: `ui/src/components/ProjectTree.tsx`

**Interfaces:**
- Consumes: `inspectRepo`, `RepoReadiness`, `RepoSetupDialog`.
- Produces: add-project flow that inspects before adding and shows `RepoSetupDialog` when not ready; a per-project live readiness badge.

- [ ] **Step 1: Replace `handleAdd` to inspect first**

In `ui/src/components/ProjectTree.tsx`, add imports:

```tsx
import { inspectRepo, RepoReadiness } from "../api";
import RepoSetupDialog from "./RepoSetupDialog";
```

Add state for a pending setup. `existing` distinguishes the **add** flow (project not yet created → call `addProject` on resolve) from the **badge** flow (project already exists → only `refresh`, because `registry.add_project` is NOT idempotent and would create a duplicate row):

```tsx
const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness; existing: boolean } | null>(null);
```

Replace `handleAdd` with:

```tsx
async function handleAdd() {
  const sel = await open({ directory: true, multiple: false });
  if (typeof sel !== "string") return;
  const name = sel.split("/").filter(Boolean).pop() ?? sel;
  setError("");
  try {
    const r = await inspectRepo(sel);
    if (r.state === "ready" && !r.dirty) {
      await addProject(name, sel);
      await refresh();
    } else {
      setSetup({ path: sel, name, readiness: r, existing: false });
    }
  } catch (e) {
    setError(String(e));
  }
}

async function finishSetup() {
  if (!setup) return;
  try {
    // Only create the record for the add flow; the badge flow's project already exists.
    if (!setup.existing) await addProject(setup.name, setup.path);
    await refresh();
  } catch (e) {
    setError(String(e));
  }
  setSetup(null);
}
```

Note: `RepoSetupDialog`'s `onResolved` fires for both "made ready" and "add anyway" (dirty case), so `finishSetup` always finalizes. For the `notARepo`-cancelled case, `onCancel` just clears `setup` without adding.

- [ ] **Step 2: Render the dialog**

Just before the closing `</aside>`, add:

```tsx
{setup && (
  <RepoSetupDialog
    readiness={setup.readiness}
    context="add"
    repoPath={setup.path}
    onResolved={finishSetup}
    onCancel={() => setSetup(null)}
  />
)}
```

- [ ] **Step 3: Add a live "needs a commit" badge**

The sidebar already lists projects. Track readiness per project. Add state and a refresh that inspects each project's repo:

```tsx
const [readiness, setReadiness] = useState<Record<string, RepoReadiness>>({});

async function refresh() {
  const ps = await listProjects();
  setProjects(ps);
  const entries = await Promise.all(
    ps.map(async (p) => [p.id, await inspectRepo(p.repo_path).catch(() => null)] as const),
  );
  setReadiness(Object.fromEntries(entries.filter(([, r]) => r) as [string, RepoReadiness][]));
}
```

In the project row JSX, after the project name span, add a badge when gated:

```tsx
{readiness[p.id]?.state === "noCommits" && (
  <button
    className="row-badge warn"
    title="Needs a commit before agents can run"
    onClick={(e) => { e.stopPropagation(); setSetup({ path: p.repo_path, name: p.name, readiness: readiness[p.id]!, existing: true }); }}
  >⚠ commit</button>
)}
```

The `existing: true` flag is essential: `registry.add_project` always INSERTs a new UUID row (confirmed — not idempotent), so the badge flow must skip `addProject` and only `refresh()`, which `finishSetup` already handles.

- [ ] **Step 4: Typecheck + run UI tests**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm exec vitest run`
Expected: no type errors; existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/ProjectTree.tsx
git commit -m "Guide repo setup when adding a project; badge gated projects"
```

---

### Task 7: Wire spawn-time gate + dirty notice

**Files:**
- Modify: `ui/src/components/AgentsView.tsx`
- Modify: `ui/src/store/runs.tsx` (expose selected project's repo path if not already available)

**Interfaces:**
- Consumes: `inspectRepo`, `RepoReadiness`, `RepoSetupDialog`, the existing `createAgent` from the runs store.
- Produces: spawn flow that inspects readiness before calling `createAgent`; blocks on `noCommits`, shows dirty notice on `ready+dirty`.

- [ ] **Step 1: Make the selected project's repo path reachable in AgentsView**

In `ui/src/components/AgentsView.tsx`, determine the selected project's `repo_path`. If the runs store already exposes the selected project object, use it; otherwise look it up via `listProjects()` filtered by `selectedProjectId`. Add at the top of the component:

```tsx
const [pendingSpawn, setPendingSpawn] = useState<{ agentId: string; readiness: RepoReadiness; repoPath: string } | null>(null);
```

- [ ] **Step 2: Gate the `spawn` handler**

Replace the body of `spawn(agentId)` so it inspects first:

```tsx
async function spawn(agentId: string) {
  const repoPath = selectedRepoPath; // from store or lookup; see Step 1
  if (!repoPath) return;
  const r = await inspectRepo(repoPath);
  if (r.state === "ready" && !r.dirty) {
    await createAgent(agentId);
  } else {
    setPendingSpawn({ agentId, readiness: r, repoPath });
  }
}
```

- [ ] **Step 3: Render the dialog; resolve into a real spawn**

Add near the component's returned JSX:

```tsx
{pendingSpawn && (
  <RepoSetupDialog
    readiness={pendingSpawn.readiness}
    context="spawn"
    repoPath={pendingSpawn.repoPath}
    onResolved={async () => {
      const { agentId } = pendingSpawn;
      setPendingSpawn(null);
      await createAgent(agentId);
    }}
    onCancel={() => setPendingSpawn(null)}
  />
)}
```

Rationale: for `noCommits`, `onResolved` only fires after `commitRepo` succeeds (now `HEAD` exists → `createAgent` works). For `ready+dirty`, "Spawn anyway" and "Commit now" both end in `onResolved` → `createAgent`.

- [ ] **Step 4: Add imports**

```tsx
import { inspectRepo, RepoReadiness } from "../api";
import RepoSetupDialog from "./RepoSetupDialog";
```

- [ ] **Step 5: Typecheck + run UI tests**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm exec vitest run`
Expected: no type errors; tests pass.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/AgentsView.tsx ui/src/store/runs.tsx
git commit -m "Gate agent spawn on repo readiness; warn on uncommitted changes"
```

---

### Task 8: Full-suite verification

**Files:** none (verification only).

- [ ] **Step 1: Rust suite**

Run: `cargo test`
Expected: all crates pass.

- [ ] **Step 2: UI typecheck + tests + build**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm exec vitest run && pnpm build`
Expected: clean.

- [ ] **Step 3: Manual smoke (documented, run by a human)**

1. Launch the app (`pnpm tauri dev` or the project's run command).
2. Add a brand-new empty folder → expect "No git repository found" → Initialize → "Create initial commit" → project appears, no badge.
3. Add a folder with files but no repo → Initialize → commit with `.gitignore` checked → confirm `.gitignore` written and `node_modules`/`.env` not committed.
4. Add an existing repo with uncommitted changes → expect the dirty notice → "Add anyway" → project added.
5. On a gated project (declined commit), click the ⚠ commit badge → commit → badge clears.
6. Spawn an agent on a dirty repo → expect dirty notice with "Spawn anyway" / "Commit now".

- [ ] **Step 4: Final commit (if any verification fixups were needed)**

```bash
git add -A
git commit -m "Verification fixups for guided repo setup"
```

---

## Self-Review Notes

- **Spec coverage:** §1 readiness→Task 1/3; §2 fixes→Task 2/3; §3 add-flow→Task 6, spawn-flow→Task 7; §4 RepoSetupDialog→Task 5, sidebar badge→Task 6; §5 edge cases→`--allow-empty` (Task 2), `.agency/` already excluded (no work), gitignore-no-overwrite (Task 2), default branch untouched (Task 2). All covered.
- **Type consistency:** `RepoReadiness` Rust enum ↔ `ReadinessDto {state,stageable,dirty}` ↔ TS `RepoReadiness` union; `inspect_repo`/`init_repo`/`commit_repo` command names match `inspectRepo`/`initRepo`/`commitRepo` bindings and arg casing (`repoPath`, `addGitignore`).
- **Resolved during planning:** `registry.add_project` is not idempotent (always inserts a new UUID) → Task 6 badge flow uses `existing: true` to skip re-adding. `project_lifecycle.rs` has no shared helper → Task 3 test constructs `AppState::new(&db_path)` inline, matching siblings.
```
