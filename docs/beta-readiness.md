# Beta readiness plan

Status of findings from the 2026-07-07 pre-beta review (bug hunt + UX pass + gap
analysis). Items marked **[in progress]** are being fixed now; everything else is
planned work, ordered by phase.

## Done / in progress

- **[in progress]** Diagnosability: file logging (tauri-plugin-log), panic hook,
  supervised notifier/loop threads, setup-failure dialog, React error boundary,
  "Open logs" + version in Settings.
- **[in progress]** termd protocol-mismatch recovery (kill+respawn old daemon
  instead of crash-looping after an app upgrade).
- **[in progress]** TermClient request/reply desync after a 5s timeout (all runs
  flip to Gone, loops kill healthy attempts).
- **[in progress]** `git status` C-quoted paths (spaces/unicode break
  stage/discard/diff and merge preview).
- **[in progress]** Merge resolver terminal never opens (null ref at `term.open()`).
- **[in progress]** Commit message wiped on failed commit.
- **[in progress]** Archived-section permanent discard: confirm + error handling.
- **[in progress]** Catch/toast sweep over bare awaits (AgentTile, AgentFocus,
  MergeModal abort, Settings deleteProfile, RunPanel stop, ProjectTree confirm,
  FocusTerminal attach).
- **[in progress]** Escape-to-close + role=dialog + backdrop stopPropagation +
  autofocus across all seven modals.
- **[in progress]** Spawn feedback (placeholder tile / disabled add menu while
  createRun is in flight).

## Phase 1 — distribution gate (before any build goes out)

1. **Code signing + notarization.** Developer ID cert; `signingIdentity` in
   tauri.conf.json; hardened runtime + signing for the bundled `agency-termd`
   sidecar; notarytool step in a release script or CI. Without this, testers get
   "Agency is damaged." (M)
2. **Release pipeline.** No CI exists. Minimal GitHub Actions workflow: build,
   sign, notarize, attach DMG to a GitHub Release. (M)
3. **Version + issue link.** Visible app version (Settings/About) and a
   "Report an issue" link. Partially covered by the in-progress Settings work. (S)

## Phase 2 — update + recovery (before build #2)

4. **Updater.** tauri-plugin-updater against a static manifest (GitHub Releases).
   Privacy note: frame as "checks a static manifest, sends nothing" to stay
   consistent with the zero-first-party-data-collection positioning. Fallback if
   time-boxed: passive new-version check + download link. (M)
5. **DB safety net.** Stamp `PRAGMA user_version`; copy `agency.db` →
   `agency.db.bak` once per app-version change before migrations run. (S)

## Phase 3 — remaining verified bugs (backend)

6. **Knowledge-graph serve command split on whitespace** — breaks graphify MCP
   for any repo path containing a space. Store argv or run via `sh -lc` like the
   build command. (state.rs:1060, config.rs:125) (S)
7. **`loops_active` flag race** can permanently park a freshly created loop —
   set the flag under `loop_gate` before insert, or generation-check before
   clearing. (state.rs:826 vs 2060) (S)
8. **Port collision on restore** — reallocate `AGENCY_PORT` base if taken.
   (state.rs:1395, registry.rs:478) (S)
9. **`delete_project`/`close_project` leak companion-shell daemon sessions** —
   also kill `agency-shell-<id>`. (state.rs:634) (S)
10. **`archive_run` ordering** — do the WIP commit first, session/tab teardown
    after, so a failed commit doesn't half-archive. (state.rs:1358) (S)
11. **Spawn-failure rollback** — remove worktree + branch when `start_session`
    fails in `create_run_spec`. (state.rs:749) (S)
12. **Zombie `sh` per KG rebuild** — reap the child on a thread.
    (state.rs:1121) (S)
13. **Merge-resolver handle never released** — stop/cleanup on modal close or
    Exited. (state.rs:2291) (S)
14. **Main-thread stalls on `loop_gate`** — make `stop_loop`/`create_loop`/
    `ensure_run_active` async commands or shrink the gate's critical section so
    a slow daemon call can't freeze the UI. (state.rs:1974) (M)

## Phase 4 — remaining verified bugs (frontend)

15. **Stale-project race in runs store** — guard `projectRef.current === pid`
    after the await in `refreshRuns`. (store/runs.tsx:37) (S)
16. **Stale `approveRunId`** re-opens MergeModal after project switch — reset it
    in `setSelectedProject`. (store/runs.tsx:81) (S)
17. **AgentFocus extra-tab sessions**: liveness guard on the fetch + occasional
    re-poll so dead tabs disappear. (AgentFocus.tsx:90) (S)
18. **FocusTerminal attach/detach race** — cancel or detach an in-flight attach
    on unmount so a disposed xterm never receives writes. (FocusTerminal.tsx:98) (S)
19. **MCP server / agent-profile rename duplicates the entry** — dedupe by the
    original name on edit-save. (Settings.tsx:113, :212) (S)
20. **Cmd+F opens search in every mounted terminal** — only the focused terminal
    should respond. (FocusTerminal.tsx:147) (S)
21. **LoopDialog max-attempts coercion** — allow empty-while-editing, clamp on
    blur. (LoopDialog.tsx:124) (S)
22. **gh readiness failure mapped to "notInstalled"** — distinguish errors from
    absence. (GhImportDialog.tsx:39) (S)

## Phase 5 — UX polish (first-session quality)

23. **Archive needs a confirm or an undo toast** ("Archived — Restore") — it's
    one-click destructive while Discard confirms. (AgentFocus.tsx:226)
24. **Cmd+Enter guard** — don't intercept while typing in a terminal/editable,
    and require a focused agent. (useShortcuts.ts:27)
25. **Terminology sweep** — pick two words (suggest "agent" for the actor,
    "worktree" for the place); today run/agent/task/workspace/worktree/attempt
    all appear in user-facing strings.
26. **⌘K palette honesty** — placeholder says "projects, tasks, files"; either
    scope the copy or add file/action entries.
27. **Status labels** — map `exited (0)`/`gone` to "finished"/"failed"/"session
    ended". (AgentTile.tsx:13)
28. **Emoji-presentation risk** — append U+FE0E to ⚠ ⚙ ☁ or swap glyphs
    (no-emoji rule). (ProjectTree.tsx:149,175; CommitBox.tsx:53)
29. **Light-theme overlays** — replace hardcoded dark backdrop/shadow colors
    with per-theme `--overlay`/`--shadow` vars. (styles.css:97,747)
30. **A11y basics** — aria-labels on icon-only buttons; global `:focus-visible`
    outline.
31. **GhSetupHint** — probe for Homebrew before offering `brew install gh`;
    show manual install URL as fallback. Same for npm-based agent installs.
32. **Rail AgentAddMenu** — thread projectId so the branch picker works from the
    rail, not just the header. (AgentFocus.tsx:160)
33. **Settings section errors** — route save failures to toasts (single top
    banner is off-screen for bottom sections); make the KG toggle move
    optimistically or show pending. (Settings.tsx:317)
34. **HomeView first-run flash** — don't render the zero-counts header before
    the first poll; put an "Add project" button in the welcome hero.
    (HomeView.tsx:72)

## Phase 6 — feature gaps (post-first-beta unless testers scream)

35. **Background remote fetch + sync affordance.** Nothing ever fetches:
    ahead/behind compares against local tracking refs, so "behind" is stale
    forever. Add a low-frequency `git fetch` (e.g. every 5 min, only when a
    remote exists, backoff on failure/offline) + a manual fetch/pull button in
    the git panel. There is currently no pull at all. (M)
36. **Quit without killing agents.** Daemon already outlives the app; offer
    "Quit and leave agents running" in the quit confirm. (M)
37. **termd crash recovery affordance** — proactive "N agents stopped — resume
    all?" instead of per-run resume-on-focus. (S/M)
38. **Missing-repo flow** — "project folder missing — relocate?" + project
    rename. (S/M)
39. **Window state persistence** — tauri-plugin-window-state. (S)
40. **Loop spend/time caps** — wall-clock cap alongside attempt cap (spec calls
    cost compounding the main failure mode). (M)
41. **Prereq checks** — git/CLT presence at launch with guidance. (S)
42. **Per-run cost/token display** (parse headless agent JSON output). (M)
43. **Message queueing** while agent is mid-turn. (M)
44. **Structured transcript view** — biggest expectation gap vs
    Conductor/Crystal; terminal-first is a legitimate positioning choice but
    should be a stated one. (L / decision)
