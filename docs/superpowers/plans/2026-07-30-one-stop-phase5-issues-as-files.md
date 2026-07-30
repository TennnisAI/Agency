# One-Stop Phase 5 — Issues as Files

> **For agentic workers:** Use superpowers:executing-plans style — implement task-by-task, failing-test-first where practical. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** the tracker becomes agent-native, versioned, and portable. Canonical
storage moves to one markdown file per issue under `.agency/issues/` (YAML-ish
frontmatter, H1 title, markdown body); SQLite reduces to (a) the `issue_seqs`
high-water mark and (b) a rebuildable index. The Tauri command surface and
`api.ts` stay **unchanged** — the UI does not participate in this phase.

**File format (locked in `docs/one-stop-plan.md`):**

```markdown
---
key: AGE-14
status: in_progress        # backlog|todo|in_progress|in_review|done|cancelled
priority: 2                # 0-4
created: 2026-07-27T09:30:00Z
updated: 2026-07-27T14:02:00Z
---
# Fix terminal resize on reattach

Body markdown, wikilinks allowed.
```

Filename is the key (`AGE-14.md`); title is the H1. `id` (uuid) stays
DB-internal — files are keyed by `key`, which is stable and never reused.

**Tech stack:** Rust only — no TS changes beyond none-at-all (the UI polls
`list_issues` every 1.5 s already and picks up everything). Unit tests inline
per module; orchestration tests in `crates/agency-app/tests/`.

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- SQLite changes are **additive migrations** guarded by `column_exists(...)`.
- No network calls. No emoji. All frontend file access stays inside the
  `resolve_within` jail (backend-internal file IO in `issuefs` takes the
  project root from the registry, same trust level as `read_markdown_corpus`).

## Decisions pinned here

- **No new dependencies.** The frontmatter schema is flat scalars, so
  `issuefs.rs` gets a strict hand-rolled parser (serde_yaml is unmaintained;
  a YAML engine buys nothing for five known keys). No chrono/time crate exists
  in the tree either — RFC3339-UTC ↔ epoch-seconds converters are ~40 lines of
  civil-date math, unit-tested against known fixtures (epoch 0, leap years,
  2026 dates).
- **Parse strictly, serialize canonically.** Known keys: `key`, `status`,
  `priority`, `created`, `updated`. Trailing ` # comment` after a value is
  tolerated on parse (the locked example shows them) but never written back.
  **Unknown keys are preserved verbatim, in order**, through a round-trip —
  that's Phase-6 forward-compat (`due`/`scheduled`/`rank` written by a newer
  build or an agent must survive an older write) and it costs one `Vec` of raw
  lines. A file with a missing/mismatched `key`, unknown `status`, or
  out-of-range `priority` is **rejected** (skipped with `log::warn!`), never
  guessed at — strict frontmatter keeps agents honest; the body is forgiving
  (anything after the closing `---`; first H1 line is the title, remainder is
  the body, leading blank lines trimmed).
- **Filename filter.** `read_issue_dir` only considers files matching
  `^[A-Z][A-Z0-9]*-[1-9][0-9]*\.md$`. Anything else (`README.md`, editor
  droppings, `.DS_Store`) is silently ignored. A file whose frontmatter `key`
  disagrees with its filename is rejected (the filename wins nothing — the
  mismatch is the error).
- **Atomic write is new code**: `issuefs::atomic_write` = temp file in the
  same directory + `fs::rename` (which clobbers on Unix — the existing
  `files::rename_path` refuses to and is not reusable).
- **Registry becomes the index, split not rewritten.** `create_issue`'s seq
  UPSERT is extracted into `alloc_issue_seq(project_id)`; new
  `ensure_issue_seq_at_least(project_id, n)` (reconcile bumps the high-water
  mark when an agent files `AGE-15` by hand) and `upsert_issue_row(&Issue)`
  (keyed on `(project_id, seq)`) join it. All existing read paths
  (`list_issues`, `get_issue`, `runs_for_issue`) and the existing registry
  unit tests stay valid — they now exercise the index layer.
- **Reconcile lives in core**: `issuefs::reconcile(reg, project_id, root)`.
  Files are truth: a DB row whose file is gone is deleted; a file with no row
  is inserted with a fresh uuid and its seq bumps the high-water mark; on
  both-exist the row is overwritten from the file. A file that fails to parse
  is skipped **and its existing row is kept** — absence drops rows, corruption
  doesn't (a half-written agent edit must not vanish an issue from the board).
  Reconcile is gated by an mtime signature: `scan` returns `(path, mtime_ms,
  size)` for the issues dir (the Phase-2 `DocStat` trick), state caches the
  signature per project, and `list_issues` only reconciles on change — idle
  polls stay at one cheap readdir.
- **Migration is lazy and per-project**: `ensure_issue_files(project)` runs at
  the top of every issue-touching command (and `list_issues`); when
  `projects.issues_migrated` (new guarded column) is unset it exports every
  SQLite issue to `.agency/issues/`, writes a short `README.md` documenting
  the format (agent-facing — ignored by the filename filter), flips the flag,
  and from then on is a one-boolean check. It also invokes the existing
  `.git/info/exclude` legacy-cleanup (`ensure_excluded` already migrates away
  the broad `.agency/` entry — extract its exclude-file rewrite so migration
  can call it without creating a worktree) so issue files are commit-able even
  on repos never re-opened since the legacy exclude.
- **Workspace joins the tracker**: the `kind <> 'workspace'` exclusion comes
  out of `backfill_issue_keys`; the workspace gets a key on next open. The
  `workspace.rs` assertion that `issue_key == None` flips. The workspace may
  have no git — issue files work regardless (plain dir writes).
- **Write path is file-first** in `state.rs`: create/update/delete/advance/
  rollback each (1) run `ensure_issue_files`, (2) compute the new issue state,
  (3) `atomic_write` the file (or remove it, for delete), (4) update the index
  row. All under the existing registry `Mutex`, so file+index stay coherent.
  The three automation hooks (merge → `Done`, PR → `InReview`, abandonment →
  todo rollback) route through the same functions and keep their semantics —
  including the forward-only rank guard, which now protects the *file* from
  regression too.
- **Conflict toast: consciously deferred.** Policy is last-writer-wins at file
  level with `updated` bumped on every app write; the 1.5 s poll self-heals
  any external edit within a tick. Detecting the race well enough to toast
  would need api-surface changes this phase explicitly freezes — deviation
  from the master plan's aside, documented here.
- **id stability**: the uuid is minted at import/migration and stable for the
  life of the file's row. Removing a project and re-adding it regenerates ids
  (breaking old `runs.issue_id` joins) — that path already destroys the runs
  themselves; noted, accepted.
- **Project delete leaves files.** Issue files are repo content; removing a
  project from Agency clears index rows + seqs exactly as today, and re-adding
  the project reconciles the files straight back (high-water mark restored
  from max file seq).
- **Agents learn the format at dispatch**: `issue_dispatch`'s prompt gains the
  issue's file path and one line each on changing `status:` and filing a
  follow-up (`next unused number for the key; app renumbers never`). This — an
  agent closing and filing issues by editing markdown on its branch, landing
  at merge — is the phase's headline property and the ship-gate scenario. The
  in-app documentation of the merge property lives in the `.agency/issues/`
  README.

## File structure

**Backend (no frontend changes):**
- `crates/agency-core/src/issuefs.rs` (new, unit-tested) — timestamp
  converters, frontmatter parse/serialize, filename pattern, `atomic_write`,
  `read_issue_dir`, `scan_issue_stats`, `reconcile`, `export_project`
  (migration writer), README content.
- `crates/agency-core/src/lib.rs` — module registration.
- `crates/agency-core/src/registry.rs` — `issues_migrated` column (guarded),
  `alloc_issue_seq` / `ensure_issue_seq_at_least` / `upsert_issue_row`,
  workspace inclusion in `backfill_issue_keys`.
- `crates/agency-core/src/worktree.rs` — extract the exclude-file rewrite from
  `ensure_excluded` into a callable helper.
- `crates/agency-app/src/state.rs` — `ensure_issue_files` guard + signature
  cache, file-first mutations, dispatch-prompt addition.
- `crates/agency-app/tests/state.rs` (or new `issues.rs`) — end-to-end
  migration/reconcile/mutation tests.
- `crates/agency-app/tests/workspace.rs` — flipped issue-key assertion.

---

### Task 1: timestamps + frontmatter round-trip

- [x] Failing tests in `issuefs.rs`: epoch↔RFC3339 fixtures (0 → `1970-01-01T00:00:00Z`, a 2026 date, a leap-day, round-trip both directions; parse rejects non-`Z`/malformed).
- [x] Implement `epoch_to_rfc3339` / `rfc3339_to_epoch` (civil-date math, no deps).
- [x] Failing tests: serialize an `Issue` → exact canonical text; parse the locked example (incl. trailing `# comments`) → correct fields; round-trip preserves unknown frontmatter keys verbatim and in order; rejection cases (missing key, bad status, priority 9, filename/key mismatch, no H1 → title falls back to key? **no — empty title rejected**, matching the app's own validation); body edge cases (H1 only, body with its own `#` headings, leading blank lines).
- [x] Implement parse/serialize (`IssueFile` carrying `Issue`-shaped data + preserved unknown lines).

### Task 2: file IO — atomic write, read dir, stats

- [x] Failing tests: `atomic_write` replaces an existing file and leaves no temp droppings; `read_issue_dir` applies the filename filter (README/stray files ignored), skips unparseable files while reporting them, returns parsed issues; `scan_issue_stats` returns `(path, mtime_ms, size)` and an empty vec for a missing dir.
- [x] Implement `atomic_write` (temp + rename in-dir), `read_issue_dir`, `scan_issue_stats` (mirror `scan_markdown_stats`, no walk needed — flat dir).

### Task 3: registry index refactor + workspace key

- [x] Guarded migration: `projects.issues_migrated INTEGER NOT NULL DEFAULT 0` (follow the `issue_key` pattern at `registry.rs:283`).
- [x] Extract `alloc_issue_seq(project_id) -> i64` from `create_issue`; add `ensure_issue_seq_at_least(project_id, n)` (UPSERT, monotonic max) and `upsert_issue_row(&Issue)` keyed `(project_id, seq)` preserving `id` on update; failing tests for each (esp. seq never regressing).
- [x] Remove the workspace exclusion from `backfill_issue_keys` (SQL at `registry.rs:331-335` + comment); update the two inline workspace tests; flip `crates/agency-app/tests/workspace.rs:18`.

### Task 4: reconcile

- [x] Failing tests (registry + tempdir fixture): external file added → row appears with fresh uuid, seq high-water bumped past it; file edited → row follows (status/title/body/updated); file deleted → row dropped; corrupt file → row kept, issue reported; `key` collision with different case/filename mismatch → skipped; ids stable across repeated reconciles.
- [x] Implement `reconcile(reg, project_id, issues_dir)` per the pinned decision; returns a summary (`imported/updated/dropped/skipped`) for logging.

### Task 5: migration (`ensure_issue_files`)

- [x] Extract the exclude-file rewrite from `WorktreeManager::ensure_excluded` (`worktree.rs:167-200`) into a helper callable without a worktree; keep existing behavior + tests.
- [x] Failing tests: a project with SQLite issues → files appear (canonical serialization, correct keys), `issues_migrated` flips, README written; second call is a no-op; a project with zero issues still flips but gets **no directory** (no surprise folders — the README arrives with the first issue file); workspace (no git) migrates without touching excludes.
- [x] Implement `export_project` in `issuefs` + `ensure_issue_files` in `state.rs` (checks flag under the registry lock, exports, calls the exclude cleanup when the project has git, flips flag).
- [x] README content: format spec, the filename-is-key rule, the merge property ("edits on an agent branch land at merge; the board reads the main checkout"), never-renumber rule.

### Task 6: file-first write path + dispatch prompt

- [x] Failing state-level tests: `create_issue` writes file then row (file exists with canonical content; seq from `alloc_issue_seq`); `update_issue` bumps `updated` in both; `delete_issue` removes file + row, seq not reused; `advance_issue_status` forward-only guard still holds and writes the file; `rollback_issue_to_todo` only fires from agent states.
- [x] Swap the five mutations in `state.rs` to file-first (each begins with `ensure_issue_files`); merge/PR/abandonment hooks compile through unchanged.
- [x] `list_issues` (state layer): `ensure_issue_files` → `scan_issue_stats` signature check against the per-project cache → `reconcile` only on change → serve index rows. Signature cache in a new `Mutex<HashMap<String, String>>` on state.
- [x] `issue_dispatch` (`state.rs:1220`): append the issue-file path + status/follow-up conventions to the prompt.

### Task 7: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [ ] Ship gate (dev app): migrate a project with existing issues — `git status` shows `.agency/issues/*.md` as commit-able (not excluded); create/edit/dispatch/merge flows unchanged in the UI; edit a file in `$EDITOR` → board updates within a tick; delete a file → issue gone; dispatch an agent with "mark AGE-N done and file a follow-up issue" → it edits/creates files on its branch, and after merge the board shows both.
- [ ] Update `docs/one-stop-plan.md` status line for Phase 5.

**Non-goals (resist):** dates/rank/labels and index-backed issue search (P6),
cross-domain links and issue-corpus wikilinks (P7), an MCP ergonomics layer
(explicitly later per D1), conflict-toast UI, filesystem watchers (polling
stays), any change to `api.ts` types or Tauri command signatures, renaming
issue files when a project's `issue_key` changes.
