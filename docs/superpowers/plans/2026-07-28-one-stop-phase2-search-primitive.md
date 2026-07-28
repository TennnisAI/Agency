# One-Stop Phase 2 — Search Primitive Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans style — implement task-by-task, failing-test-first where practical. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** One backend search command that Files, Docs, the palette, and (later) issues all consume — `rg --json` when available, a bounded Rust scan everywhere else — plus the docs-index scaling fix: steady-state polls stat the corpus instead of re-reading every body.

**Architecture (locked in `docs/one-stop-plan.md`):** new `crates/agency-core/src/search.rs` with `search_files(root, SearchQuery) -> Vec<SearchHit>`. Hard caps on hits, per-file hits, bytes scanned, and wall-clock; async Tauri command so a search can never wedge the IPC thread. `rg` resolves from the process PATH (already repaired at startup by `pathenv::repair`, the same mechanism agent detection relies on). Fallback respects `.gitignore` via `git ls-files --cached --others --exclude-standard` in repo roots, plain walk otherwise. Docs polls switch to an mtime/size signature pass; bodies are re-read only for changed files; the Docs search box keeps title/tag matching in the TS index and gets body hits from the backend.

**Tech stack:** Rust (`regex`, `globset` — new offline deps in agency-core), TS/React, Vitest, cargo test.

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- New API-facing Rust structs use `#[serde(rename_all = "camelCase")]`.
- All frontend file access via `FileRoot` + `resolve_within` — the search command takes `(root: FileRoot, dir: String)` and resolves through the jail; hit paths come back relative to `dir`.
- No network calls. No emoji. **No issue-model fields.**

## Decisions pinned here

- **Hit shape:** `SearchHit { path, line (1-based), col (1-based), text }`. The optional `contextBefore/After` from the master plan is deferred to Phase 3 (find-in-files) — additive, nothing depends on it yet.
- **Both engines skip hidden files** (rg's default) — matches the docs-corpus convention and keeps engine outputs comparable.
- **Glob semantics are gitignore-style** ("`*.md`" matches at any depth). The fallback normalizes slash-less globs to `**/…` for `globset`; rg gets them raw.
- **Caps (compile-time):** max 500 hits total (default 200, caller can lower), 20 hits/file, 2 MB/file, 64 MB total scanned (fallback), 3 s wall clock. Hitting any cap returns what was collected — search is best-effort, never an error.
- **rg failure ≠ no matches:** exit 1 is "no matches", success. Exit 2 / spawn failure falls back to the Rust scan.
- **`AGENCY_NO_RG=1`** env override forces the fallback (tests, ship-gate timing, emergency).

## File structure

**Backend:**
- `crates/agency-core/src/search.rs` (new) — query/hit types, engine pick, rg engine, fallback engine, caps; unit tests on a fixture tree + an `#[ignore]`d 10k-file timing test.
- `crates/agency-core/src/lib.rs` — `pub mod search;`
- `crates/agency-core/Cargo.toml` — add `regex`, `globset`.
- `crates/agency-core/src/files.rs` — `DocStat { path, mtimeMs, size }`, `scan_markdown_stats` (stat-only corpus walk, same caps/skips), `read_markdown_files` (read a named subset).
- `crates/agency-app/src/commands.rs` — async `search_files`, `docs_corpus_stats`, `read_docs_files`.
- `crates/agency-app/src/lib.rs` — register the three commands.

**Frontend:**
- `ui/src/api.ts` — `SearchQuery`, `BackendSearchHit`, `DocStat` types + `searchFiles`, `docsCorpusStats`, `readDocsFiles`.
- `ui/src/hooks/useDocs.ts` — incremental corpus: stats diff → selective body reads → index rebuild only on change; dev-only re-read counter log.
- `ui/src/lib/docsIndex.ts` — split `searchDocs` into local (tags + titles) and a merge helper for backend body hits; keep the old substring scan exported as the error fallback.
- `ui/src/components/DocsTree.tsx` — debounced (150 ms) async search effect replacing the sync `useMemo`.

---

### Task 1: search.rs — types, caps, fallback engine

- [x] Failing tests (fixture tree in tempdir, `AGENCY_NO_RG` forced): substring hit with correct 1-based line/col + line text; case-insensitive by default, `case: true` distinguishes; `regex: true` patterns; slash-less glob (`*.md`) filters at any depth; per-file and total hit caps honored; binary (NUL) files skipped; hidden dirs skipped; in a git repo, `.gitignore`d files are excluded while untracked files are found.
- [x] `SearchQuery` (Deserialize, camelCase, defaults) / `SearchHit` (Serialize, camelCase, PartialEq).
- [x] Fallback: file list via `git ls-files -z --cached --others --exclude-standard` when the root is a repo (walk otherwise, skipping hidden/`node_modules`/`target`), globset filter, `regex::Regex` matcher (escaped unless `regex`, `(?i)` unless `case`), per-line scan with caps + deadline; text capped at 500 chars.
- [x] `cargo test -p agency-core search` green.

### Task 2: search.rs — rg engine + engine pick

- [x] `rg_on_path()` PATH probe (honors `AGENCY_NO_RG`); spawn `rg --json --no-config` with `--fixed-strings`/`-i`/`--max-count`/`--max-filesize`/`-g` mapped from the query, `current_dir = root`.
- [x] Stream stdout, parse `type == "match"` events (path, `line_number`, `lines.text`, first submatch `start` → col), stop at caps/deadline, kill the child on early exit; exit-code-2/spawn-failure → fallback.
- [x] Parity test (skipped when rg absent): rg and fallback return the same hits on the fixture tree for substring + glob queries.
- [x] `#[ignore]`d timing test: generate ~10k small files, assert both engines return within the deadline (run manually for the ship gate).

### Task 3: files.rs — corpus stats + selective read

- [x] `DocStat` (camelCase) + `scan_markdown_stats(root, rel_dir)` — identical walk/skips/caps to `read_markdown_corpus`, but stat-only (no body reads).
- [x] `read_markdown_files(root, rel_dir, paths)` — jailed reads of a named subset, same utf-8/too-large handling; missing files silently skipped (deleted between stat and read).
- [x] Tests: stats match the corpus walk's file set; selective read returns exactly the asked-for files; a path escaping the jail errors.

### Task 4: commands + api

- [x] Async commands: `search_files(root, dir, query)` (resolve `dir` through the jail; hits relative to it), `docs_corpus_stats(root, docsDir)`, `read_docs_files(root, docsDir, paths)`; register in `lib.rs`.
- [x] `api.ts`: types + wrappers.

### Task 5: docs index scaling (useDocs)

- [x] `useDocs` keeps a `Map<path, DocFile>` + `Map<path, "mtime:size">` in refs; each poll fetches stats, diffs, reads only changed/new files, drops removed ones, and rebuilds the index only when something changed. First load = stats + read-all (one extra round-trip over today, then cheap forever).
- [x] Dev-only counter: `console.debug` the number of bodies re-read, logged only when > 0 — an idle corpus logs nothing (this is the ship-gate probe).
- [x] `createDocsDir` / detection flow unchanged.

### Task 6: Docs search → backend body hits

- [x] `docsIndex.ts`: export `searchLocal(index, query)` (tag + title hits, exact current semantics) and `mergeBodyHits(index, local, backendHits)` (backend line is 1-based → TS 0-based, `text.trim()` → snippet, dedupe against title hits, keep the 5-per-doc / 200-total caps); keep `searchDocs` as the error-path fallback.
- [x] `DocsTree.tsx`: replace the sync `useMemo` with a 150 ms-debounced effect — `#tag` queries stay fully local; otherwise `searchFiles(root, docsDir, { query, globs: ["*.md", "*.markdown"] })` merged via `mergeBodyHits`; backend error → `searchDocs` substring fallback. Stale responses guarded by query token.
- [x] `vitest` green (new tests for `mergeBodyHits` mapping/dedupe).

### Task 7: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [x] Ship gate A (automated): run the `#[ignore]`d 10k-file timing test twice — with rg on PATH and with `AGENCY_NO_RG=1` — both within the timeout, identical shapes.
- [x] Ship gate B (dev app): Docs tab on a large corpus idles without body re-reads (dev console counter silent), search box returns body hits via the backend. *Passed 2026-07-28.*

**Non-goals (resist):** quick-open, find-in-files UI, palette providers (P3/P4 ride on this), context lines, issue-model fields.
