# Source Control for terminals — design

**Date:** 2026-06-25
**Status:** Approved for planning

## Problem

Focusing a terminal and opening the **Source Control** tab shows the empty-state
message *"Terminals have no source control."* (`ui/src/components/AgentsView.tsx:67`).
The message is misleading: a terminal runs the user's login shell in the project's
**main checkout** (`create_terminal`, `crates/agency-app/src/state.rs:483-512`), which is
a real git repo on a real branch. There is genuine source control to show — the panel
is simply wired to per-agent worktrees and gated to `kind === "agent"`.

## Goal

When a terminal is focused, the Source Control tab shows the source control and history
of the project's main checkout (the actual checked-out branch), with **full read-write**
capability (stage/unstage, commit, discard, push) — the same panel agents get, minus the
agent-only "comment-to-agent" affordance.

## Why this is low-risk

A terminal's panel operates on the same checkout the shell already has unrestricted write
access to. Giving the GUI stage/commit/push against that checkout introduces no new risk
surface — it mirrors what the user could already do by typing `git` in the shell.

## Architecture

### Backend — single chokepoint

Every git command and the Files resolver funnel through `AppState::worktree_path(id)`
(`crates/agency-app/src/state.rs:763`), which already loads the run record:

```rust
pub fn worktree_path(&self, id: &str) -> Result<std::path::PathBuf> {
    let run = self.run_record(id)?;
    let repo = self.project_repo(&run.project_id)?;
    Ok(repo.join(".agency").join("worktrees").join(id))
}
```

**Change:** when `run.kind == "terminal"`, return the project repo root (`repo`) instead of
the `.agency/worktrees/<id>` subdirectory. Add a doc comment noting the method now returns a
non-worktree directory for terminals (it is, in effect, "the working directory the run's
git/file commands operate on").

This single change makes all of the following operate on the live checkout with **no other
backend edits**, because they all resolve their directory through `worktree_path`:
`git_status`, `git_diff`, `git_stage` / `git_unstage` / `git_stage_all` / `git_unstage_all`,
`git_discard` / `git_discard_all`, `git_commit`, `git_push`, `git_branch_info`, the
line/hunk staging commands, and the `FileRoot::Run` resolver in `file_root_dir`.

`git::branch_info` (`crates/agency-core/src/git.rs:267`) and `git::log_graph` (`:238`) derive
branch, upstream, ahead/behind, base, and history generically from whatever directory they
are handed — they do **not** read the agent's stored `base` field — so the panel reflects the
actual checked-out branch and its real upstream correctly.

**Side benefit (in scope, no extra work):** the **Files** tab currently resolves a focused
terminal via `FileRoot::Run → worktree_path(terminalId)` → a nonexistent worktree dir. After
this change it resolves to the project root, so the Files tab starts working for focused
terminals too. This is consistent and desirable; no UI change is required for it.

### Why overloading `worktree_path` is safe

The destructive cleanup paths do **not** use `worktree_path`:

- `discard_run` (`state.rs:581`) gates worktree removal on `run.kind == "agent"` (`:588`) and
  computes the directory via `WorktreeManager::new(repo).remove(id)`, not `worktree_path`.
- `archive_run` (`state.rs:600`) computes the worktree path inline.
- Merge / approve flows are reachable only for agents (the Review/approve and MergeModal UI is
  gated to `kind === "agent"`; terminals cannot be approved or merged).

So `worktree_path` is used only by the git/file command resolvers, none of which delete by
path. Returning the repo root for terminals cannot misdirect a cleanup at the main checkout.

### Naming decision

The method keeps the name `worktree_path` with an updated doc comment (minimal diff; it is
already de facto "the run's working directory" and is called from ~30 sites). A cleaner-named
alternative (`run_dir`/`git_root` with all call sites rerouted) is rejected as unnecessary
churn for this change.

### Frontend

1. **`ui/src/components/AgentsView.tsx` (lines 63-69)** — render `GitPanel` when the focused
   run is a terminal, not only an agent. Concretely, the Source Control tab body shows the
   panel for `focusedRunId && (focused.kind === "agent" || focused.kind === "terminal")`. The
   "Terminals have no source control." branch is removed; the remaining empty state stays
   "Open an agent to review its changes." for the nothing-focused case.

2. **Thread run kind into the panel** — pass the focused run's kind (or a derived
   `allowComments: boolean`, where agents allow and terminals do not) from `AgentsView` →
   `GitPanel`. For terminals:
   - `GitPanel` does not render the `ReviewComments` strip
     (`ui/src/components/git/ReviewComments.tsx`).
   - `DiffViewer` (`ui/src/components/git/DiffViewer.tsx`) hides the **Comment** button (`:151`)
     and its comment box; a new prop (e.g. `allowComments?: boolean`, default `true`) gates it.
     `GitPanel` passes the flag through when it renders `DiffViewer` (`GitPanel.tsx:96`).

   Everything else in `GitPanel` (BranchBar with Sync/push, ChangesPanel stage/commit/discard,
   HistoryPanel, DiffViewer hunk/line staging) renders identically for terminals.

3. **Compact "Review" side-pane stays agent-only** — already gated at `AgentsView.tsx:53` and
   `:95`. Terminals get only the full Source Control tab, not the inline Review pane. No change.

## Out of scope (YAGNI)

- The agent "Review → comment-to-agent" workflow for terminals (deliberately suppressed).
- Project-root source control when **nothing** is focused (the Source Control tab keeps its
  existing "Open an agent…" empty state; only the Files tab falls back to project root today).
- Any change to push, merge, or branch semantics.

## Testing

**Backend (unit):** a test asserting `worktree_path` returns the project repo root for a run
with `kind == "terminal"` and the `.agency/worktrees/<id>` path for a run with `kind == "agent"`.

**Manual / acceptance:**

1. Focus a terminal, open **Source Control** → the branch bar shows the project's actual
   current branch with real upstream ahead/behind.
2. Make an edit in the terminal's checkout → it appears under Changes; stage, write a message,
   and **Commit** → the commit lands on the live branch; **Sync** pushes it.
3. **History** lists the real commit log of the checked-out branch; selecting a commit shows
   its diff.
4. The **comment-to-agent** UI is absent for the terminal: no "Comment" button in the diff
   viewer and no "Review comments" strip.
5. Focus an **agent** and confirm its Source Control panel is unchanged (isolated worktree,
   comment-to-agent affordances present).
6. Focus a terminal and open the **Files** tab → it browses the project root (previously broken).
