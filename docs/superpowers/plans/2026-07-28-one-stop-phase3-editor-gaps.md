# One-Stop Phase 3 — Editor Gaps: Quick-Open, Find-in-Files, Tabs

> **For agentic workers:** Use superpowers:executing-plans style — implement task-by-task, failing-test-first where practical. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** the three VS Code gaps in the Files tab, all riding on the Phase 2 search primitive: file-name quick-open on ⌘P everywhere outside Docs, find-in-files with jump-to-line, and real editor tabs with dirty tracking that survive root switches.

**Architecture (locked in `docs/one-stop-plan.md`):** quick-open lists names via `git ls-files` when possible (walk fallback — both already exist privately in `search.rs`); find-in-files is a thin UI over `search_files`; tabs replace `FilesView`'s single `selected` with `open: string[]` + `active`, per-root persistence like `usePaneWidth`, and an LRU cap (~10) that evicts clean tabs so live CodeMirror instances stay bounded.

**Tech stack:** Rust (no new deps), TS/React, Vitest, cargo test.

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- New API-facing Rust structs use `#[serde(rename_all = "camelCase")]` (this phase adds none — the listing is `Vec<String>`).
- All frontend file access via `FileRoot` + jailed rel paths; the new `list_files` command mirrors `search_files`'s `(root, dir)` shape and resolves through `abs_path`.
- No network calls. No emoji — glyphs are text or stroked SVGs. **No issue-model fields.**

## Decisions pinned here

- **`list_files` caps:** max 20 000 names returned, sorted, `/`-separated, relative to `dir`. Reuses `search.rs`'s `git_files`/`walk_files` (gitignore respected in repos, hidden/`node_modules`/`target` skipped elsewhere; `MAX_WALK_FILES` already bounds the walk). One IPC round-trip per quick-open invocation; no streaming, no incremental refresh — reopening re-lists.
- **Fuzzy semantics** (`ui/src/lib/fuzzy.ts`): `fuzzyScore(query, path)` returns `null` (no match) or a tiered score — basename-prefix beats basename-substring beats path-substring beats in-order character subsequence; shorter paths win ties. `paletteFilter.filterEntries` keeps its exact current tiers and gains subsequence matching as a new *lowest* tier via the shared util, so existing behavior (and tests) are preserved.
- **Open-file routing:** `ui/src/lib/openFile.ts` — a module-level pending slot + `agency:open-file` CustomEvent (`{ path, line? }`), same pattern as `agency:open-note` but with a `consumePending()` for the mount race (quick-open switches to the Files tab *then* asks it to open a file). The request is root-less: FilesView opens it in whatever root it is currently showing, which is by construction the root quick-open listed (both derive focused-run-else-project from the same store).
- **Tab model:** `open: string[]` (strip order = open order), `active: string | null`, activation recency tracked separately for LRU. Cap `MAX_TABS = 10`: opening an 11th evicts the least-recently-activated tab that is *clean and not active*; if every candidate is dirty the cap is exceeded rather than losing edits. Closing a dirty tab prompts (ConfirmDialog, danger) — evictions never target dirty tabs so only explicit closes can discard work.
- **Persistence:** `localStorage["files:tabs:<kind>:<id>"] = JSON.stringify({ open, active })` — paths only. Restored tabs are *cold*: no editor mounts until first activation, so restoring 10 tabs costs zero file reads. A tab whose file has vanished shows FileEditor's normal error state and can be closed.
- **Dirty survival across unmounts** (`ui/src/lib/editorBuffers.ts`): module-level `Map<"kind:id:path", { text: string }>`. FileEditor stashes its doc on unmount iff dirty, prefers a stashed buffer over the disk read on mount (and stays dirty), and drops the buffer on save/revert/clean-unmount. Closing a tab drops it too. This is what makes "switch roots and back, edits intact" work without keeping foreign-root editors mounted. Capped at 50 entries (oldest evicted) as a leak guard.
- **Live editors:** one hidden-not-unmounted FileEditor per *warm* tab (activated this session), `display:none` when inactive — CM state, scroll, and undo history survive tab switches for free. `scrollToLine(line)` is an imperative handle mirroring `scrollToHeading` (`DocsEditor.tsx:133`), applied with the same 150 ms best-effort delay after activation.
- **Find-in-files:** query state lives in FilesView (mirrors DocsView/DocsTree); a non-empty query replaces the tree body with results grouped by file — path header, then per-line rows (`line: text`). Debounced 150 ms with a stale-response token, exactly the `DocsTree.tsx` pattern. Literal, case-insensitive, no globs, `maxHits` 200 (backend defaults). No options UI this phase.
- **⌘P ownership:** AgentsView binds ⌘P (no shift/alt) and opens QuickOpen when `project && tab !== "docs"`; DocsView keeps its existing listener (it is only mounted on the Docs tab, so exactly one handler fires). No project selected → ⌘P is inert. ⌘W binds inside FilesView only (mounted only on the Files tab; `menu.rs` has no ⌘W accelerator to clash with).

## File structure

**Backend:**
- `crates/agency-core/src/search.rs` — `pub fn list_root_files(root, max) -> Vec<String>` wrapping the existing `git_files`/`walk_files`; unit tests.
- `crates/agency-app/src/commands.rs` — async `list_files(root, dir, maxFiles)`.
- `crates/agency-app/src/lib.rs` — register it.

**Frontend:**
- `ui/src/api.ts` — `listFiles` wrapper.
- `ui/src/lib/fuzzy.ts` (new, tested) — shared scoring; `paletteFilter.ts` delegates its new lowest tier to it.
- `ui/src/lib/editorBuffers.ts` (new, tested) — dirty-doc stash.
- `ui/src/lib/fileTabs.ts` (new, tested) — pure tab-state transitions: open/close/activate/evict/rename/remove-under-dir + (de)serialization.
- `ui/src/lib/openFile.ts` (new, tested) — pending-open slot + event.
- `ui/src/components/FileEditor.tsx` — forwardRef `{ scrollToLine }`, `onDirtyChange`, buffer stash/restore.
- `ui/src/components/FileTabs.tsx` (new) — the strip.
- `ui/src/components/FilesView.tsx` — tabs + search state + open-file listener + ⌘W.
- `ui/src/components/FileTree.tsx` — search input + grouped results (query mode), `onRenamed`/`onDeleted` callbacks.
- `ui/src/components/QuickOpen.tsx` (new) — ⌘P modal.
- `ui/src/components/AgentsView.tsx` — ⌘P binding + QuickOpen mount.
- `ui/src/styles.css` — tab strip + files-search styles (share the `docs-search` look via comma selectors).

---

### Task 1: backend listing — `list_root_files` + `list_files` command

- [x] Failing tests in `search.rs`: `list_root_files` returns sorted rel paths; respects `.gitignore` in a repo and includes untracked files; skips hidden dirs outside a repo; truncates to `max`.
- [x] `pub fn list_root_files(root: &Path, max: usize) -> Vec<String>` — `list_files(root)` truncated to `max.min(20_000)`.
- [x] `commands.rs`: `async fn list_files(state, root: FileRoot, dir: String, max_files: usize)` resolving `dir` via `abs_path` (jail); register in `lib.rs`.
- [x] `api.ts`: `listFiles(root: FileRoot, dir: string, maxFiles: number): Promise<string[]>`.
- [x] `cargo test -p agency-core search` green.

### Task 2: fuzzy util shared with the palette

- [x] Failing Vitest specs for `fuzzyScore`: basename-prefix < basename-substring < path-substring < subsequence (lower = better); non-matching returns null; case-insensitive; tie broken by target length; `fuzzyFilter(query, items, key)` returns rank-sorted matches capped at a limit.
- [x] Implement `ui/src/lib/fuzzy.ts`.
- [x] `paletteFilter.ts`: keep tiers 0–2 verbatim; entries matching only as a subsequence (via `fuzzyScore`) join as tier 3. Existing `paletteFilter.test.ts` untouched and green; add a spec for the new tier.

### Task 3: FileEditor — scrollToLine, dirty callback, buffer stash

- [x] `editorBuffers.ts` + tests: `stash/take/drop/has` keyed `kind:id:path`; 50-entry cap evicts oldest.
- [x] `FileEditor` → `forwardRef` exposing `scrollToLine(line)` (1-based; clamps to doc length; selects line start, scrolls `y:"start"` margin 12, focuses — mirror of `scrollToHeading`).
- [x] New props `onDirtyChange?: (dirty: boolean) => void` (fired from the existing `setDirty` sites) and buffer integration: mount prefers `take()`d stash (sets dirty), unmount stashes iff dirty, save/revert `drop()`.
- [x] `AgentsView`'s existing usage unchanged (props are optional).

### Task 4: tabs — pure model, strip, FilesView wiring

- [x] Failing Vitest specs for `fileTabs.ts`: open (new → appended + active + recency bump; existing → just activated); close (picks next active: nearest right, else left, else null); evict on 11th open (least-recently-activated, clean, non-active; all-dirty → no eviction, 11 tabs); `renamePath` retargets file + dir-prefix moves for open/active/dirty/recency; `removePath` closes file-or-dir-descendant tabs; `serialize/deserialize` round-trip drops dirty/recency.
- [x] Implement `ui/src/lib/fileTabs.ts` (pure functions over a `TabState { open, active, recency, dirty }`).
- [x] `FileTabs.tsx`: strip above the editor — per-tab basename (disambiguate duplicate basenames with their parent dir), dirty dot `●`, hover close `×` button, middle-click (auxclick button 1) closes, click activates; overflow scrolls horizontally. Stroked-SVG/text glyphs only.
- [x] `FilesView.tsx`: replace `selected` with tab state (persisted per `files:tabs:<kind>:<id>`, restored on root switch); warm-set rendering (mount FileEditor per activated tab, `display:none` inactive, keyed by root+path); dirty set fed by `onDirtyChange`; dirty-close ConfirmDialog; ⌘W (window keydown while mounted) closes the active tab; FileTree `selected` mirrors `active`.
- [x] `FileTree.tsx`: add `onRenamed(from,to)` / `onDeleted(path)` callbacks (fired where rename/delete already adjust selection); FilesView retargets tabs, dirty set, and buffers.
- [x] Tab strip styles in `styles.css`.

### Task 5: find-in-files

- [x] `FileTree.tsx` query mode: search input above the root label (shared styling with `docs-search`); when `query` non-empty render grouped results instead of the tree — file header row (file icon + path, click opens at first hit) and line rows (`<line>: <trimmed text>`, click opens at that line).
- [x] Debounced 150 ms `searchFiles(root, "", { query })` effect with stale-token guard (mirror `DocsTree.tsx:244`); error → empty results with a quiet "Search failed" row; group flat hits by path preserving backend order.
- [x] `FilesView.tsx`: `openAtLine(path, line)` — open/activate tab, then `scrollToLine` via the ref map after the 150 ms activation delay (skip scroll for non-text views).
- [x] Escape clears the query; clearing restores the tree.

### Task 6: quick-open

- [x] `openFile.ts` + tests: `requestOpenFile({ path, line? })` stores pending + dispatches `agency:open-file`; `consumePending()` returns-and-clears; subscribe helper.
- [x] `QuickOpen.tsx`: palette-style modal (reuse `palette-overlay/palette/palette-input/palette-list/palette-row` classes + `useModalKeys` + arrow/Enter navigation à la `DocsQuickSwitcher`); on mount `listFiles(root, "", 20000)`; rows = `fuzzyFilter` capped at 50 (empty query: first 50 alphabetically); row = file icon + basename + dimmed dir; Enter/click → `onOpen(path)`.
- [x] `AgentsView.tsx`: ⌘P keydown (no shift/alt, `project && tab !== "docs"`) toggles QuickOpen with the same root FilesView derives (focused run else project); `onOpen` → `setTab("files")` + `requestOpenFile({ path })`.
- [x] `FilesView.tsx`: listen for `agency:open-file` + consume pending on mount/root-switch → `openAtLine`.
- [ ] Manual check: ⌘P in Docs still opens the note switcher; ⌘P on Agents/Issues/Source/Files opens quick-open; ⌘K palette unaffected.

### Task 7: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [ ] Ship gate (dev app): open 5 files in tabs, edit two, switch roots and back — dirty state and active tab survive; find-in-files jumps to the exact line; ⌘P works from every tab of a project and still does the Docs switcher inside Docs; 11th open evicts a clean tab, never a dirty one.
- [ ] Update `docs/one-stop-plan.md` status line.

**Non-goals (resist):** palette providers / `/` content search in ⌘K (Phase 4), search options UI (regex/case/globs), context lines in results, recency-weighted quick-open ranking (Phase 4's activation store), drag-to-reorder tabs, split editors, issue-model fields.
