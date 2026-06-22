# Guided repo setup for Agency projects

Date: 2026-06-22
Status: Approved, ready for planning

## Problem

Agency runs every agent in a git **worktree** branched from the project's
`HEAD` (`WorktreeManager::create` → `git worktree add … HEAD`). That has two
hard prerequisites that a freshly-picked folder often fails:

1. The folder must be a **git repository**.
2. The repository must have **at least one commit** (so `HEAD` resolves).

Today `validate_repo` (in `crates/agency-app/src/state.rs`) detects both
failures and bails with a text error, which `ProjectTree.handleAdd` dumps into
a red `git-error` div. The user is told what's wrong but given **no way
forward**. Older project records that predate `validate_repo` fail even later,
with the raw `git worktree add … failed: fatal: not a git repository` error.

Goal: replace dead-end errors with a **guided setup flow** that detects the
folder's git state and walks the user through making it agent-ready —
initializing the repo and/or creating the first commit — with informed consent.

## Key constraint that shapes everything

A new worktree's working directory contains **only what is committed in
`HEAD`**. Untracked / uncommitted files in the main checkout do **not**
propagate into an agent's worktree. Therefore:

- **Zero commits** is not "some files are ignored" — it is "no agent can start
  at all." The first commit is *required* to use the project.
- **Has commits + dirty tree** genuinely is "agents work from the last commit
  and won't see the dirty edits until they're committed." Committing here is
  *optional*.

These two states share a symptom (files not in git) but have opposite stakes,
so they get different copy and different blocking behavior.

## Design

### 1. Live readiness, computed never stored

A folder's git state can change at any time (the user may commit in their own
terminal), so readiness is **always computed live** and never persisted on the
`Project` record.

New function in `agency-core` (`git.rs`):

```rust
pub enum RepoReadiness {
    NotARepo,
    NoCommits { stageable: bool }, // stageable=false → empty/all-ignored → needs --allow-empty
    Ready { dirty: bool },         // dirty = `git status --porcelain` non-empty
}

pub fn repo_readiness(path: &Path) -> RepoReadiness;
```

`repo_readiness` becomes the single source of truth. `validate_repo` in
`state.rs` shrinks to "is this even addable" and rejects only `NotARepo`.

### 2. Mechanical fixes (core, operate on a raw path)

- `init_repo(path)` → `git init`.
- `write_default_gitignore(path)` → writes a sensible default `.gitignore`
  (`node_modules/`, `.env`, `dist/`, `target/`, `.DS_Store`, …) **only if one
  does not already exist** (never overwrites).
- `initial_commit(path, add_gitignore: bool)` → optional gitignore write,
  `git add -A`, then commit; if nothing was staged, retry with `--allow-empty`.

Exposed as Tauri commands: `inspect_repo`, `init_repo`, `commit_repo`.

### 3. Workflow — bucket-aware two-prompt model

**At add-project time** (`ProjectTree.handleAdd`), after the folder picker →
`inspect_repo(path)`:

| Bucket | Dialog | If declined |
|---|---|---|
| `NotARepo` | "No git repository found. A repo is required for the Agency workflow. Initialize one now?" → on confirm runs `init_repo`, then **re-inspects** (always becomes `NoCommits`) and continues to the commit prompt | project **not added** |
| `NoCommits` | "Agency needs at least one commit — each agent starts from your latest commit. Create the initial commit now?" + **☐ Add a .gitignore** toggle | project **added but gated** |
| `Ready { dirty: true }` | "You have uncommitted changes. Agents work from your last commit and won't see these until committed. Commit now?" + gitignore toggle | project added, no problem |
| `Ready { dirty: false }` | none — add silently | — |

**At agent-spawn time** (`ui/src/store/runs.tsx → createAgent`, the single
spawn chokepoint), re-inspect first:

- `NoCommits` → **required** commit dialog; cannot spawn until resolved.
- `Ready { dirty: true }` → informational notice with **[Commit first] /
  [Spawn anyway]**.
- `Ready { dirty: false }` → spawn immediately.

This gives the gate at add-time and the dirty-notice live at spawn-time (which
also catches trees that went dirty after the project was added).

### 4. UI

- One **adaptive** `RepoSetupDialog` component (not three): takes a
  `RepoReadiness` plus context (`add` vs `spawn`) and renders the right copy,
  the optional `.gitignore` toggle, and the right buttons. Styled after the
  existing `ConfirmDialog`.
- Sidebar badge in `ProjectTree`: projects whose live readiness is `NoCommits`
  show a "⚠ needs a commit" affordance; clicking it opens the setup dialog.

### 5. Edge cases

- Empty / fully-ignored folder → `--allow-empty` initial commit (`stageable:
  false`).
- `.agency/` is already added to `.git/info/exclude` by `WorktreeManager`, so it
  never pollutes the dirty check or the initial commit.
- An existing `.gitignore` is never overwritten.
- `git init` uses the user's configured default branch name; we do not force
  `main`/`master`.

## Out of scope (YAGNI)

- Multi-step wizard (file-by-file commit selection, remote setup).
- Editing `.gitignore` contents in-app beyond the single default toggle.
- Remote / push configuration during setup.
```
