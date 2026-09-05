# Beta readiness plan

Tracks the findings from the 2026-07-07 pre-beta review (bug hunt + UX pass +
gap analysis). Updated 2026-07-08 after the bug-fix pass: every verified bug in
Phases 3–5 is fixed, plus the diagnosability/distribution work and the first
Phase 6 item (background fetch). What remains is listed under **Still open**.

## Status at a glance

- **Phase 0 (diagnosability sweep):** done.
- **Phase 1 (distribution gate):** done.
- **Phase 2 (update + recovery):** DB safety net done; updater still open.
- **Phase 3 (backend bugs, #6–14):** done (9/9).
- **Phase 4 (frontend bugs, #15–22):** done (8/8).
- **Phase 5 (UX polish, #23–34):** done (12/12).
- **Phase 6 (feature gaps):** #35, #40, #42, #43 and #45 done; the rest of
  #36–44 open (post-first-beta).

Verification for the fix pass: `cargo build` clean, `cargo test` green (136
tests: 82 core + 54 app), `tsc --noEmit` exit 0, `vite build` exit 0.

## Done

### Phase 0 — diagnosability sweep

- **[done]** File logging (tauri-plugin-log), panic hook, supervised
  notifier/loop threads, setup-failure dialog, React error boundary, "Open
  logs" + version in Settings.
- **[done]** termd protocol-mismatch recovery (kill+respawn old daemon instead
  of crash-looping after an app upgrade).
- **[done]** TermClient request/reply desync after a 5s timeout (seq-tagged
  requests; stale replies dropped).
- **[done]** `git status` C-quoted paths — switched to `--porcelain -z`.
- **[done]** Merge resolver terminal null ref at `term.open()` (xterm created in
  an effect after the container commits).
- **[done]** Commit message preserved on failed commit.
- **[done]** Archived-section permanent discard: confirm + error handling.
- **[done]** Catch/toast sweep over bare awaits (AgentTile, AgentFocus,
  MergeModal abort, Settings deleteProfile, RunPanel stop, ProjectTree confirm,
  FocusTerminal attach).
- **[done]** Escape-to-close + role=dialog across all modals; backdrop
  stopPropagation + autofocus extended to the settings-overlay modals.
- **[done]** Spawn feedback (placeholder tile / disabled add menu while
  createRun is in flight).

### Phase 1 — distribution gate

1. **[done]** Code signing + notarization (Developer ID + hardened runtime +
   `Agency.entitlements`; deep-signs the `agency-termd` sidecar; notarize +
   staple in `scripts/release-macos.sh`).
2. **[done]** Release pipeline (`.github/workflows/release.yml`): `v*` tag or
   manual dispatch builds on the macOS Apple Silicon runner, signs + notarizes,
   attaches the DMG to a draft Release. (Apple Silicon only; universal build is
   a follow-up.)
3. **[done]** Visible app version (Settings ▸ Diagnostics) and a "Report an
   issue" link (opens the GitHub issues page).

### Phase 3 — backend bugs

6. **[done]** Knowledge-graph serve command no longer whitespace-split — proper
   argv via `config::default_serve_argv` + quote-aware `split_command`
   (unit-tested), so repo paths with spaces work. (state.rs, config.rs)
7. **[done]** `loops_active` park-forever race — a `loop_generation` counter;
   `drive_loops` won't clear the flag if a loop was created mid-pass. (state.rs)
8. **[done]** Port collision on restore — `restore_run` reallocates the port
   block if a live run took it. (state.rs, registry.rs `set_port_base`)
9. **[done]** `delete_project`/`close_project` now kill the companion-shell
   daemon session (`agency-shell-<id>`). (state.rs)
10. **[done]** `archive_run` ordering — WIP commit happens before session
    teardown, so a failed commit no longer half-archives. (state.rs)
11. **[done]** Spawn-failure rollback — a failed `start_session` removes the
    just-cut worktree + branch. (state.rs)
12. **[done]** Zombie `sh` per KG rebuild — child reaped on a thread. (state.rs)
13. **[done]** Merge-resolver handle released via a new `resolver_close` command
    the modal calls on close/abort. (state.rs, commands.rs, MergeModal.tsx)
14. **[done]** Main-thread `loop_gate` stalls — `stop_loop` and
    `ensure_run_active` are now async commands (off the UI thread). (commands.rs)

### Phase 4 — frontend bugs

15. **[done]** Stale-project race — `refreshRuns` drops a response if the
    project changed during the await. (store/runs.tsx)
16. **[done]** Stale `approveRunId` reset in `setSelectedProject`. (store/runs.tsx)
17. **[done]** AgentFocus extra-tab sessions: liveness guard on the fetch +
    4s re-poll; gone tabs drop out of the strip. (AgentFocus.tsx)
18. **[done]** FocusTerminal attach/detach race — `disposed` guard + detach on
    unmount. (already landed pre-pass)
19. **[done]** MCP-server / agent-profile rename dedupes by the original name.
    (Settings.tsx)
20. **[done]** Cmd+F responds only in the focused terminal. (FocusTerminal.tsx)
21. **[done]** LoopDialog max-attempts allows empty-while-editing, clamps on
    blur. (LoopDialog.tsx)
22. **[done]** gh readiness distinguishes a probe error from gh being absent,
    with a retry. (GhImportDialog.tsx)

### Phase 5 — UX polish

23. **[done]** Archive confirm dialog. (AgentFocus.tsx)
24. **[done]** Cmd+Enter guarded against editable/terminal targets. (useShortcuts.ts)
25. **[done]** Terminology sweep — user-facing "workspace" → "agent"/"worktree"
    across GhImportDialog, MergeModal, RunPanel.
26. **[done]** ⌘K palette copy scoped to real "projects and agents" entries.
27. **[done]** Status labels map exited(0)/non-zero/gone → finished/failed/
    session-ended. (AgentTile.tsx, HomeView.tsx)
28. **[done]** Emoji-presentation risk fixed with U+FE0E on ⚠ ⚙ ☁.
    (ProjectTree.tsx, git/CommitBox.tsx)
29. **[done]** Light-theme overlays via per-theme `--overlay`/`--shadow` vars.
    (styles.css)
30. **[done]** A11y basics — global `:focus-visible` outline + aria-labels on
    icon-only buttons. (styles.css, ProjectTree.tsx, BranchBar.tsx, …)
31. **[done]** GhSetupHint offers Homebrew install *and* a manual cli.github.com
    download fallback. (GhSetupHint.tsx)
32. **[done]** Rail AgentAddMenu threads `projectId` so the branch picker works
    from the rail. (AgentFocus.tsx)
33. **[done]** Settings save failures route to toasts; KG toggle is optimistic
    with revert-on-failure. (Settings.tsx)
34. **[done]** HomeView holds the header until the first poll (no zero-count
    flash) and adds an "Add project" button to the welcome hero. (HomeView.tsx)

### Phase 2 — update + recovery

4. **[partial]** Passive update check shipped (the documented fallback). On
   launch — unless disabled in Settings ▸ Diagnostics — `update::check` curls
   GitHub's latest-release API, compares tags via `version::is_newer`, and dots
   the Settings button. Agency downloads and installs nothing, so an update can
   never restart the app over running agents. Full `tauri-plugin-updater`
   auto-update is still open; see **Still open** for what it needs.
   (update.rs, version.rs, App.tsx, ProjectTree.tsx, Settings.tsx)
5. **[done]** DB safety net. `registry::backup_before_migrations` stamps
   `PRAGMA user_version` with the encoded app version and copies `agency.db` →
   `agency.db.bak` once per version change, before `Registry::open` migrates.
   Called from `lib.rs` setup; best-effort (logs on failure, never blocks
   launch). (registry.rs, lib.rs)

### Phase 6 — feature gaps

35. **[done]** Background remote fetch + sync affordance. A `remote-fetch`
    thread fetches each project's `origin` every 5 min (capped exponential
    backoff per project on failure/offline). The git panel's BranchBar gains a
    Fetch button (updates ahead/behind) and a Pull button (fast-forward,
    `--ff-only`) when the branch is behind an upstream. (git.rs, state.rs,
    commands.rs, lib.rs, BranchBar.tsx, GitPanel.tsx)

43. **[done]** Message queueing while an agent is mid-turn (`sendq.rs`). The
    three features that type a prompt the human never wrote — review comments,
    failing checks, a conflicted merge — now hand their text to a per-session
    queue instead of writing it straight into the pty. `sendq::decide` is the
    pure transition function, in the `looper.rs` / `notifier.rs` mold, and the
    notifier tick drains it: a message goes out only when `activity::classify`
    says the pane is not `Working`, a second has passed since the human's last
    keystroke, and nothing they typed is sitting unsent on the prompt line. The
    draft block is one-directional by construction — keystroke classification
    (which tells typing apart from the mouse and focus traffic the pane also
    emits) sets it, and a pane read may only lift it. Nothing ever clears the
    line: no Escape, no `^U`. At the 5-minute timeout the text is appended after
    whatever is there and submitted with it, rather than thrown away. The write
    itself is unchanged, still `send_text`'s text-pause-`\r`. The three commands
    now report whether the text went in or is waiting, and the three call sites
    say so. (sendq.rs, state.rs, commands.rs, lib.rs, api.ts, MergeModal.tsx,
    PrSection.tsx, ReviewComments.tsx)

45. **[done]** Per-worktree skills kit (`skills.rs`). Two skills, namespaced
    `agency-*`, written into each worktree wherever the agent reads
    project-local skills from: `.claude/skills/` for Claude Code, and the
    vendor-neutral `.agents/skills/` for every other agent Agency ships except
    Crush, which has only global skills roots and gets nothing. Each agent was
    probed on 2026-09-05 (AGE-145); the per-agent record is on the table in
    `skills.rs`. Claude Code is the one that does not read `.agents/skills`,
    so the two-root split stays. `agency-date-range` ships a POSIX-shell resolver
    that turns "last quarter" or "the past 30 days" into exact inclusive dates,
    doing the arithmetic on day numbers so leap years and quarter edges fall
    out of the algorithm. `agency-workspace` is the boot-time catalog: worktree
    and branch, project checkout, the tracker's absolute path, the setup and
    run commands, `AGENCY_*`, a loop's check command, and what a merge does
    with the work. Emitted beside `emit_mcp` at run creation, on an extra tab's
    agent, on restore, and on every loop attempt (which re-uses its worktree
    and never re-emitted anything before). A skill directory the repo tracks is
    left alone, sibling skills are never touched, and the kit is excluded via
    the repo's shared `.git/info/exclude`. (skills.rs, state.rs)

46. **[done]** A held message is visible, cancellable, and survives a quit
    (AGE-123, follow-up to 43). `RunInfo` carries a `queuedMessages` count, and
    while it is non-zero the tile's foot and the run header wear an amber
    marker beside the activity readout. Opening it lists what is waiting
    (origin and the whole text) and drops any of it: the queue is the only
    thing in Agency that types into a session with nobody watching, so
    cancelling has to be possible. A drop is matched on the message text, not
    on a position, because the drain runs on the notifier tick and an index the
    popover read a second ago can point at a different message; a drop that
    finds nothing left says so instead of appearing to do nothing. The queue is
    also mirrored into a `send_queues` table on every change, so quitting with
    something held no longer loses it with no trace — the daemon outlives the
    app, so the agent it was queued for is usually still sitting there. On
    restore the origin comes back through a default-deny allowlist
    (`sendq::ORIGINS`) and every message is re-stamped to launch time: keeping
    the original stamp would put a message held overnight past `MAX_HOLD_MS`,
    and that branch types regardless of what the pane looks like. Nothing is
    delivered at launch; the first tick re-decides each message against the
    session as it is now, and discards for a session the daemon no longer
    hosts. (sendq.rs, registry.rs, state.rs, commands.rs, lib.rs, api.ts,
    QueuedMarker.tsx, AgentTile.tsx, AgentFocus.tsx)

47. **[done]** A queued message that is dropped, or appended, says so (AGE-127,
    follow-up to 46). The queue had one silent exit: `sendq::decide` discards
    everything waiting for a session that is gone, and that went to the log and
    nowhere else — the marker appeared for a tick and then vanished, which from
    the user's side is what a message going in looks like. Persistence made it
    easy to hit: quit with something held, relaunch, and if the session did not
    survive the whole queue goes on the first tick. `drain_session` now hands
    its caller a `sendq::Notice` for the two outcomes nobody can see for
    themselves, and the notifier tick emits them as a `send-queue-notice` event
    the UI toasts. The sentences are composed in `sendq.rs` (with an origin
    phrase per sender, because a toast arriving minutes later cannot lean on the
    popover's bare tag) so they are unit-testable without a running app. The
    second outcome is the delivery side of the same hole: `Decision::Send` now
    carries a `SendReason`, and a message that waited out `MAX_HOLD_MS` with the
    agent still working or a draft still on the line is reported as appended,
    while one that merely went out late is not. A discard racing a send from an
    open sender surface is answered there instead, as an error: `queue_send`
    used to return "it went out now" for a message nothing was ever typed of.
    (sendq.rs, state.rs, lib.rs, api.ts, App.tsx, toast.ts)

## Still open

### Phase 2 — update + recovery (before build #2)

4. **Auto-updater.** The passive check (above) covers the beta; this is the
   upgrade to real one-click updates via tauri-plugin-updater against a static
   manifest on GitHub Releases. Outstanding work: (M)
   - `tauri-plugin-updater` + `tauri-plugin-process`, `updater:default`
     capability, `bundle.createUpdaterArtifacts: true`.
   - A Tauri signing keypair, distinct from the Apple Developer ID. **The
     private key is unrecoverable** — lose it and every installed client is
     stranded on its current version permanently. Back it up before first use.
   - `release.yml`: signing env vars, upload `.app.tar.gz` + `.sig`, generate
     `latest.json` (only a `darwin-aarch64` key while builds are Apple Silicon
     only).
   - Requires a **public repo** — `releases/latest/download/` won't serve
     private assets.
   - **Must not install while agents are running.** `TermClient::connect_or_spawn`
     handles a `PROTOCOL_VERSION` bump by killing the old daemon, and its own
     log says "its sessions are lost" — an eager install would destroy in-flight
     work on relaunch. Needs a gate, and overlaps with #36.
   - Can't be verified without two real releases; plan a throwaway manifest test
     before testers depend on it.

### Phase 6 — feature gaps (post-first-beta unless testers scream)

36. **Quit without killing agents.** Daemon already outlives the app; offer
    "Quit and leave agents running" in the quit confirm. (M)
37. **termd crash recovery affordance** — proactive "N agents stopped — resume
    all?" instead of per-run resume-on-focus. (S/M)
38. **Missing-repo flow** — "project folder missing — relocate?" + project
    rename. (S/M)
39. **Window state persistence** — tauri-plugin-window-state. (S)
40. **[done]** **Loop spend/time caps** — wall-clock and token caps alongside the attempt
    cap. Design settled 2026-08-17: the caps live in `looper::step`, beside the
    existing `attempt >= max_attempts` branches, so the policy stays a pure
    function with no side effects and no repo or daemon needed to test it.
    `LoopConfig` grows `max_wall_secs` and `max_tokens` as `Option`, both
    defaulting to `None`; a cap you did not ask for never trips. `LoopState`
    grows a `stall_reason` so "attempt cap", "crash loop", "wall clock" and
    "budget" stop being four things that all render as *Stalled*. Token
    observations come from #42.

    Two rules worth stating because they are what makes a guardrail usable
    rather than annoying: **hard stops are off by default**, and the
    engineering budget goes on false positives, not on detection. Velocity,
    no-progress and repeated-tool-call heuristics are deliberately out of
    scope; they are the part that misfires on a compaction burst or on a
    subagent whose progress is invisible to the parent. Caps first. (M)
    Shipped 2026-08-23: caps gate spawning only (never a running attempt, and
    a passing check completes even over-cap); see "As shipped" in
    `agentic-loops.md`. Tracked in AGE-110.
41. **Prereq checks** — git/CLT presence at launch with guidance. (S)
42. **[done]** **Per-run cost/token display.** Design settled 2026-08-17, and it replaces
    the original "parse headless agent JSON output" sketch: read the agent's own
    transcript JSONL instead. For Claude Code that is
    `~/.claude/projects/<encoded-cwd>/*.jsonl`, whose per-record `message.usage`
    and `message.model` cover **interactive** sessions too, not just headless
    loop attempts. Price each record by its own model, because a session can
    change model mid-run. Dedupe by request id; duplicate rows in these files
    are an observed condition, not a hypothetical. Re-read incrementally keyed
    on `(len, mtime)` so the existing 2s notifier tick absorbs the poll for
    free.

    Agency gets a simplification the shape of this idea does not usually
    afford: one worktree per run means the transcript directory is already
    scoped to the run, and extra agent tabs sharing that worktree belong to the
    same run anyway. So per-directory aggregation *is* per-run aggregation, and
    no session-id filtering is needed.

    The ledger is a plain JSONL file, consistent with everything-is-a-file.
    Only `claude` and `pi` have a transcript format we can read; for the other
    eight the UI shows nothing rather than zero, and an unrecognized model
    yields token counts with no cost figure. We do not display a number we
    cannot stand behind. (M) Tracked in AGE-109.
44. **Structured transcript view** — the biggest expectation gap against the
    session-viewer tools in this category; terminal-first is a legitimate
    positioning choice but should be a stated one. (L / decision)
