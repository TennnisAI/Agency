# Source Control for Terminals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Source Control tab work for a focused terminal by pointing it at the project's main checkout, with full read-write git, hiding only the agent-only comment-to-agent affordance.

**Architecture:** All git/file commands resolve their working directory through `AppState::worktree_path(id)`. Changing that single method to return the project repo root for `kind == "terminal"` runs makes every git command operate on the live checkout with no other backend edits. The frontend un-gates the Source Control tab for terminals and threads an `allowComments` flag that suppresses the comment-to-agent UI.

**Tech Stack:** Rust (Tauri backend, `cargo test`), React + TypeScript (Vite, `vitest`/`tsc`), pnpm.

## Global Constraints

- **Full read-write scope** for terminals: stage/unstage, commit, discard, push all operate on the project's live branch. No new gating of those actions.
- **Suppress comment-to-agent only**: terminals hide the DiffViewer "Comment" button + box and the `ReviewComments` strip. Everything else in the panel is identical to agents.
- **Agent behavior must be byte-for-byte unchanged**: agent runs keep resolving to `.agency/worktrees/<id>` and keep all comment affordances.
- **UI package manager is pnpm, not npm** (npm churns the lockfile). For typecheck, prefer the direct binary `ui/node_modules/.bin/tsc` — pnpm 11 `exec`/`build` can exit 1 on an esbuild ignored-build even when compilation succeeds.
- **No AI attribution** in commit messages.

---

### Task 1: Backend — `worktree_path` resolves terminals to the project root

**Files:**
- Modify: `crates/agency-app/src/state.rs:763-767` (`worktree_path`)
- Test: `crates/agency-app/tests/state.rs` (add one integration test near the existing `worktree_path_resolves_for_active_run` at `:117`)

**Interfaces:**
- Consumes: `AppState::run_record(id) -> Run` (has `.kind`, `.project_id`), `AppState::project_repo(project_id) -> PathBuf` (private, already used inside `worktree_path`), `AppState::project_repo_path(project_id) -> Result<PathBuf>` (public, test asserts against it), `AppState::create_terminal(project_id) -> RunInfo`.
- Produces: `worktree_path(id)` returns the project repo root for terminal runs and `<repo>/.agency/worktrees/<id>` for agent runs. Signature unchanged.

- [ ] **Step 1: Write the failing test**

Add to `crates/agency-app/tests/state.rs` (uses the existing `init_repo` helper and `AppState`):

```rust
#[test]
fn worktree_path_resolves_to_repo_root_for_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    // Agent run: still resolves under .agency/worktrees/<id>.
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
    }).unwrap();
    let agent = state.create_run(&project.id, "p", "noop", "HEAD", None).unwrap();
    let agent_wt = state.worktree_path(&agent.id).unwrap();
    assert!(agent_wt.ends_with(format!(".agency/worktrees/{}", agent.id)));

    // Terminal run: resolves to the project repo root, NOT a worktrees subdir.
    let term = state.create_terminal(&project.id).unwrap();
    let term_dir = state.worktree_path(&term.id).unwrap();
    assert_eq!(term_dir, state.project_repo_path(&project.id).unwrap());
    assert!(!term_dir.to_string_lossy().contains("worktrees"));

    // Cleanup sessions/records.
    state.discard_run(&agent.id).unwrap();
    state.discard_run(&term.id).unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-app --test state worktree_path_resolves_to_repo_root_for_terminal -- --nocapture`
Expected: FAIL on the `assert_eq!(term_dir, ...repo root...)` — the current method returns `<repo>/.agency/worktrees/terminal-<id>`, so `term_dir` ends in a `worktrees` path and is not equal to the repo root.

- [ ] **Step 3: Implement the change**

Replace `worktree_path` in `crates/agency-app/src/state.rs:763-767`:

```rust
    /// The working directory the run's git/file commands operate on.
    ///
    /// For agents this is the run's isolated worktree
    /// (`<repo>/.agency/worktrees/<id>`). For terminals — which have no worktree
    /// and run the user's shell in the project's main checkout — it is the
    /// project repo root, so Source Control / Files act on the live branch.
    pub fn worktree_path(&self, id: &str) -> Result<std::path::PathBuf> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        if run.kind == "terminal" {
            return Ok(repo);
        }
        Ok(repo.join(".agency").join("worktrees").join(id))
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p agency-app --test state worktree_path_resolves_to_repo_root_for_terminal -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the existing worktree tests to confirm no regression**

Run: `cargo test -p agency-app --test state worktree_path`
Expected: PASS for both `worktree_path_resolves_for_active_run` and the new test.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/tests/state.rs
git commit -m "feat(app): resolve terminal worktree_path to project repo root"
```

---

### Task 2: Frontend — thread `allowComments` through GitPanel and DiffViewer

This is a no-op-for-agents plumbing change: it adds an `allowComments` flag (default `true`) that gates the comment-to-agent UI. With the default, every existing caller behaves exactly as before, so the build stays green and agents are unchanged. Task 3 sets the flag to `false` for terminals.

**Files:**
- Modify: `ui/src/components/git/DiffViewer.tsx:15-24` (props), `:151` (Comment button), `:158` (comment box)
- Modify: `ui/src/components/git/GitPanel.tsx:18-30` (props), `:80` and `:92` (ReviewComments), `:96` (DiffViewer render)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `DiffViewer` accepts `allowComments?: boolean` (default `true`). When `false`, the "Comment" button and comment box are not rendered.
  - `GitPanel` accepts `allowComments?: boolean` (default `true`). When `false`, the `ReviewComments` strip is not rendered and it passes `allowComments={false}` to `DiffViewer`.

- [ ] **Step 1: Add `allowComments` to DiffViewer props**

In `ui/src/components/git/DiffViewer.tsx`, change the destructure + type (`:15-24`):

```tsx
export default function DiffViewer({
  taskId, path, mode, hash, onChanged, onCommentAdded, allowComments = true,
}: {
  taskId: string;
  path: string;
  mode: Mode;
  hash?: string;
  onChanged: () => void;
  onCommentAdded?: () => void;
  allowComments?: boolean;
}) {
```

- [ ] **Step 2: Gate the Comment button and box on `allowComments`**

In `ui/src/components/git/DiffViewer.tsx`, line `:151`, change:

```tsx
            <button className="git-iconbtn" onClick={() => setCommenting(true)}>Comment</button>
```

to:

```tsx
            {allowComments && <button className="git-iconbtn" onClick={() => setCommenting(true)}>Comment</button>}
```

And the comment box at `:158`, change the opening condition:

```tsx
      {commenting && sel && sel.lines.size > 0 && (
```

to:

```tsx
      {allowComments && commenting && sel && sel.lines.size > 0 && (
```

- [ ] **Step 3: Add `allowComments` to GitPanel props and gate ReviewComments + DiffViewer**

In `ui/src/components/git/GitPanel.tsx`, add the prop to the destructure and type (`:18-30`):

```tsx
export default function GitPanel({
  taskId,
  layout,
  selection,
  onSelect,
  width,
  allowComments = true,
}: {
  taskId: string;
  layout: "compact" | "full";
  selection: GitSelection;
  onSelect: (sel: GitSelection) => void;
  width?: number;
  allowComments?: boolean;
}) {
```

Gate the compact `ReviewComments` (`:80`):

```tsx
        {allowComments && <ReviewComments key={commentsKey} taskId={taskId} />}
```

Gate the full-layout `ReviewComments` (`:92`):

```tsx
          {allowComments && <ReviewComments key={commentsKey} taskId={taskId} />}
```

Pass the flag to `DiffViewer` (`:96`) — add `allowComments={allowComments}` to the existing render:

```tsx
          {selection?.kind === "file" && <DiffViewer taskId={taskId} path={selection.path} mode={diffMode(selection.group)} onChanged={refresh} onCommentAdded={() => setCommentsKey((k) => k + 1)} allowComments={allowComments} />}
```

- [ ] **Step 4: Typecheck (no behavior change expected)**

Run: `cd ui && ./node_modules/.bin/tsc --noEmit`
Expected: exit 0, no type errors. (Per Global Constraints, use the direct `tsc` binary, not `pnpm build`.)

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/git/DiffViewer.tsx ui/src/components/git/GitPanel.tsx
git commit -m "refactor(ui): gate comment-to-agent UI behind allowComments flag"
```

---

### Task 3: Frontend — render Source Control for focused terminals

**Files:**
- Modify: `ui/src/components/AgentsView.tsx:63-69` (Source Control tab body)

**Interfaces:**
- Consumes: `GitPanel`'s new `allowComments` prop (from Task 2); `focused` (`RunInfo | null`, has `.kind: "agent" | "terminal"`).
- Produces: the Source Control tab renders `GitPanel` for any focused run (agent or terminal); terminals get `allowComments={false}`.

- [ ] **Step 1: Un-gate the Source Control tab**

In `ui/src/components/AgentsView.tsx`, replace the block at `:63-69`:

```tsx
      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId && focused?.kind === "agent"
            ? <GitPanel taskId={focusedRunId} layout="full" selection={gitSel} onSelect={setGitSel} />
            : <div className="board empty">{focused?.kind === "terminal" ? "Terminals have no source control." : "Open an agent to review its changes."}</div>}
        </div>
      )}
```

with:

```tsx
      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId && focused
            ? <GitPanel taskId={focusedRunId} layout="full" selection={gitSel} onSelect={setGitSel} allowComments={focused.kind === "agent"} />
            : <div className="board empty">Open an agent or terminal to view its source control.</div>}
        </div>
      )}
```

- [ ] **Step 2: Typecheck**

Run: `cd ui && ./node_modules/.bin/tsc --noEmit`
Expected: exit 0, no type errors.

- [ ] **Step 3: Full UI build (catches anything tsc-only misses)**

Run: `cd ui && pnpm build`
Expected: build succeeds. If it exits 1 with only an esbuild "ignored build" notice (see Global Constraints), confirm `tsc --noEmit` was clean and `dist/` was produced; that exit code is the known pnpm/esbuild quirk, not a real failure.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/AgentsView.tsx
git commit -m "feat(ui): show Source Control for focused terminals"
```

---

### Task 4: Manual acceptance verification

No code changes — this task runs the app and confirms the feature end-to-end against the spec's acceptance criteria. Fold any fixes back into Tasks 1-3.

**Files:** none.

- [ ] **Step 1: Launch the app**

Use the project's run path (e.g. `cargo tauri dev` from the repo root, or the project's documented launch command). Open a project that is a git repo.

- [ ] **Step 2: Terminal → Source Control shows the live branch**

Add a terminal (rail `+` → New terminal), focus it, click **⎇ Source Control**.
Expected: the branch bar shows the project's actual current branch with real upstream ahead/behind. No "Terminals have no source control." message.

- [ ] **Step 3: Full read-write works on the live checkout**

In the terminal, edit a tracked file (e.g. `echo x >> README.md`).
Expected: it appears under Changes. Stage it, type a commit message, click **Commit** → the commit lands on the live branch (verify with `git log -1` in the terminal). **Sync** pushes it (if an upstream exists).

- [ ] **Step 4: History reflects the real log**

Open **History**.
Expected: the real commit log of the checked-out branch; selecting a commit shows its diff.

- [ ] **Step 5: Comment-to-agent UI is absent for terminals**

Select lines in a diff.
Expected: Stage/Unstage/Revert selection buttons appear, but **no "Comment" button**, and there is **no "Review comments" strip**.

- [ ] **Step 6: Agent panel is unchanged**

Focus an agent, open Source Control.
Expected: isolated worktree changes only; the "Comment" button and Review comments strip are present as before.

- [ ] **Step 7: Files tab works for a focused terminal (side benefit)**

Focus the terminal, open **▤ Files**.
Expected: it browses the project root tree (previously this resolved to a nonexistent worktree dir).

- [ ] **Step 8: Record the result**

Note pass/fail for each step. If all pass, the feature is complete. If anything fails, return to the relevant task, fix, and re-verify.

---

## Self-Review Notes

- **Spec coverage:** Backend chokepoint change → Task 1. Un-gate tab → Task 3. Hide comment-to-agent (DiffViewer Comment + ReviewComments) → Task 2. Files-tab side benefit → Task 4 Step 7. Agent-unchanged guarantee → Task 1 test (agent branch) + Task 4 Step 6. Full read-write → Tasks 2/3 (no action gating) + Task 4 Step 3. All spec sections covered.
- **Out-of-scope items** (project-root SC when nothing focused; comment-to-agent for terminals; push/merge semantics) are intentionally not implemented; the nothing-focused empty state is preserved with reworded copy.
- **Type consistency:** `allowComments?: boolean` (default `true`) is identical across `DiffViewer` and `GitPanel`; `AgentsView` passes `allowComments={focused.kind === "agent"}`. `worktree_path` signature is unchanged.
