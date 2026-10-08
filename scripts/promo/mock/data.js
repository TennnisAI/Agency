// Demo backend state and command handlers. Fictional projects only.
(function () {
  const NOW = Math.floor(Date.now() / 1000);
  const NOWMS = Date.now();
  const S = (window.__MOCK_STATE__ = {});

  S.projects = [
    { id: "p-orbit", name: "orbit-api", repo_path: "/Users/demo/code/orbit-api", default_agent: "claude", default_provider: null, color: "blue", issue_key: "ORB", kind: null },
    { id: "p-lumen", name: "lumen-web", repo_path: "/Users/demo/code/lumen-web", default_agent: "codex", default_provider: null, color: "mauve", issue_key: "LUM", kind: null },
    { id: "p-tide", name: "tidepool", repo_path: "/Users/demo/code/tidepool", default_agent: "claude", default_provider: null, color: "green", issue_key: "TDP", kind: null },
    { id: "p-ledger", name: "ledger-cli", repo_path: "/Users/demo/code/ledger-cli", default_agent: "opencode", default_provider: null, color: "peach", issue_key: "LED", kind: null },
  ];
  S.workspace = { id: "p-ws", name: "Workspace", repo_path: "/Users/demo/Workspace", default_agent: "claude", default_provider: null, color: "teal", issue_key: "WS", kind: "workspace" };

  let runSeq = 100;
  function mkRun(o) {
    const id = o.id || `r${runSeq++}`;
    return Object.assign({
      id, projectId: "p-orbit", agent: "claude", prompt: "", title: null, branch: `agent/${id}`,
      status: { state: "running" },
      activity: { state: "working", since: NOWMS - 60000, reported: true },
      agentStatus: null, pinRank: null,
      usage: null, added: 0, deleted: 0, files: 0, port: null, guiPort: null, guiSessionId: null, guiLive: false,
      kind: "agent", runScriptsLive: false, worktree: true,
      createdAt: NOW - 900, archivedAt: null, raceId: null, loopConfig: null, loopState: null,
      issueId: null, model: null, queuedMessages: 0, primaryClosed: false, sessions: [], archived: null,
    }, o, { id });
  }
  S.mkRun = mkRun;
  const usage = (tot, cents) => ({ inputTokens: Math.round(tot * 0.6), outputTokens: Math.round(tot * 0.1), cacheWriteTokens: 0, cacheReadTokens: Math.round(tot * 0.3), totalTokens: tot, cents, costComplete: true });

  S.runs = [
    mkRun({ id: "r-rate", projectId: "p-orbit", agent: "claude", title: "Add per-key rate limiting to the public API", branch: "agent/rate-limiting", added: 214, deleted: 31, files: 6, usage: usage(184000, 142), createdAt: NOW - 1500, agentStatus: { text: "Running the integration suite", since: NOWMS - 20000, stale: false } }),
    mkRun({ id: "r-pagi", projectId: "p-orbit", agent: "codex", title: "Cursor-based pagination for /v2/events", branch: "agent/cursor-pagination", added: 96, deleted: 40, files: 4, createdAt: NOW - 1300 }),
    mkRun({ id: "r-otel", projectId: "p-orbit", agent: "gemini", title: "Wire OpenTelemetry spans through the job queue", branch: "agent/otel-job-queue", added: 158, deleted: 12, files: 7, createdAt: NOW - 1100, activity: { state: "blocked", since: NOWMS - 40000, reported: true } }),
    mkRun({ id: "r-flaky", projectId: "p-orbit", agent: "opencode", title: "Fix the flaky webhook retry test", branch: "agent/flaky-webhook-retry", added: 18, deleted: 9, files: 2, createdAt: NOW - 900, activity: { state: "done", since: NOWMS - 120000, reported: false } }),
    mkRun({ id: "r-mig", projectId: "p-orbit", agent: "cursor", title: "Migrate config loading to typed settings", branch: "agent/typed-settings", added: 302, deleted: 188, files: 11, createdAt: NOW - 700 }),
    mkRun({ id: "r-docs", projectId: "p-orbit", agent: "pi", title: "Document the auth flow in docs/auth.md", branch: "agent/auth-docs", added: 74, deleted: 3, files: 1, createdAt: NOW - 500 }),

    mkRun({ id: "r-lum1", projectId: "p-lumen", agent: "claude", title: "Dark mode for the settings pages", branch: "agent/settings-dark-mode", added: 120, deleted: 44, files: 9, usage: usage(98000, 77) }),
    mkRun({ id: "r-lum2", projectId: "p-lumen", agent: "copilot", title: "Replace moment with date-fns", branch: "agent/date-fns", added: 61, deleted: 83, files: 14, activity: { state: "done", since: NOWMS - 300000, reported: false } }),
    mkRun({ id: "r-tide1", projectId: "p-tide", agent: "codex", title: "Stream sensor readings over websockets", branch: "agent/ws-stream", added: 233, deleted: 19, files: 8 }),
    mkRun({ id: "r-tide2", projectId: "p-tide", agent: "kimi", title: "Speed up the nightly rollup query", branch: "agent/rollup-speedup", added: 37, deleted: 52, files: 3 }),
    mkRun({ id: "r-led1", projectId: "p-ledger", agent: "crush", title: "CSV import for bank statements", branch: "agent/csv-import", added: 141, deleted: 6, files: 5 }),
  ];

  // Issues --------------------------------------------------------------
  let issueSeq = 1;
  function mkIssue(projectId, seq, title, status, priority, body, extra) {
    return Object.assign({
      id: `i-${projectId}-${seq}`, projectId, seq, title, body: body || "", status, priority,
      due: null, scheduled: null, rank: issueSeq++, links: [], comments: [],
      createdAt: NOW - 86400 * (10 - (seq % 9)), updatedAt: NOW - 3600 * (seq % 7),
    }, extra || {});
  }
  S.issues = [
    mkIssue("p-orbit", 41, "Add per-key rate limiting to the public API", "in_progress", 3, "Token bucket per API key, configurable per plan. Return 429 with Retry-After."),
    mkIssue("p-orbit", 42, "Cursor-based pagination for /v2/events", "in_progress", 2, "Offset pagination falls over past page 200. Switch to opaque cursors."),
    mkIssue("p-orbit", 43, "Wire OpenTelemetry spans through the job queue", "in_progress", 2),
    mkIssue("p-orbit", 44, "Fix the flaky webhook retry test", "in_review", 3, "Fails about 1 run in 20 on CI. Looks like a timing assumption in the backoff."),
    mkIssue("p-orbit", 45, "Support idempotency keys on POST /payments", "todo", 4,
      "Clients retry on timeouts and we create duplicate payments.\n\n## Acceptance\n\n- Accept an `Idempotency-Key` header on POST /payments\n- Store the first response for 24h and replay it for repeats\n- A repeat with a different body returns 422\n- Tests for replay, mismatch and expiry"),
    mkIssue("p-orbit", 46, "Return Problem Details (RFC 9457) for validation errors", "todo", 2),
    mkIssue("p-orbit", 47, "Health check should report queue lag", "todo", 1),
    mkIssue("p-orbit", 48, "Drop the legacy v1 auth middleware", "backlog", 1),
    mkIssue("p-orbit", 39, "Upgrade to Postgres 17 in CI", "done", 2),
    mkIssue("p-orbit", 38, "Structured logging for request handlers", "done", 2),
    mkIssue("p-orbit", 37, "Seed script for local development", "done", 1),
    mkIssue("p-lumen", 12, "Dark mode for the settings pages", "in_progress", 2),
    mkIssue("p-lumen", 13, "Replace moment with date-fns", "in_review", 1),
    mkIssue("p-lumen", 14, "Keyboard shortcuts for the editor", "todo", 2),
    mkIssue("p-tide", 7, "Stream sensor readings over websockets", "in_progress", 3),
    mkIssue("p-tide", 8, "Speed up the nightly rollup query", "in_progress", 2),
  ];
  S.issues.find((i) => i.seq === 41).links = ["ORB-45"];
  S.issues.find((i) => i.seq === 45).comments = [
    { author: "Sam Rivera", createdAt: NOW - 7200, body: "Same key, same response, for 24 hours. Keep the store in Postgres, not Redis." },
  ];
  for (const r of S.runs) {
    const i = S.issues.find((x) => x.projectId === r.projectId && x.title === r.title);
    if (i) r.issueId = i.id;
  }

  // Docs ------------------------------------------------------------------
  S.docs = {
    "p-orbit": {
      dir: "docs",
      files: {
        "architecture.md": "---\ntags: [design, api]\nstatus: living\n---\n# Architecture\n\norbit-api is a Go service in front of Postgres, with a job queue for anything slower than a request.\n\n## Request path\n\nRequests pass through [[auth]], then [[rate-limits]], then the handler. Every handler returns a typed result; errors map to Problem Details.\n\n## Job queue\n\nWebhooks, exports and rollups run on the queue. Workers are stateless and safe to scale out.\n\n- [x] Move webhooks onto the queue\n- [x] Retry with exponential backoff\n- [ ] Trace spans across enqueue and dequeue\n\n## Storage\n\nOne Postgres database. Migrations live in `db/migrations` and run on deploy.\n\n## Open questions\n\n- Do we shard events by tenant before 1.0?\n- Should rate limits live in Postgres or in memory per node?\n",
        "auth.md": "# Auth\n\nAPI keys are hashed with argon2id and scoped per project. See [[architecture]].\n",
        "rate-limits.md": "---\ntags: [api]\n---\n# Rate limits\n\nToken bucket per API key. Plans set the refill rate. See [[architecture]] and ORB-41.\n\n## Defaults\n\n| Plan | Requests / min | Burst |\n|---|---|---|\n| Free | 60 | 120 |\n| Team | 600 | 1200 |\n| Scale | 6000 | 12000 |\n",
        "runbooks/deploys.md": "# Deploys\n\nTag a release, CI builds the image, and the rollout waits on the health check. Links back to [[architecture]].\n",
        "runbooks/incidents.md": "# Incidents\n\nPage the on-call, open a channel, write the timeline as you go.\n",
        "decisions/0004-cursor-pagination.md": "# 0004 Cursor pagination\n\nAccepted. Offset pagination does not scale past a few thousand rows. See [[architecture]].\n",
      },
    },
    "p-ws": {
      dir: ".",
      files: {
        "Welcome.md": "# Welcome\n\nThis is your workspace.\n",
        "journal/2026-10-07.md": "# Tuesday 7 October\n\n- [ ] Review the rate limiting branch\n- [x] Merge the flaky test fix\n",
      },
    },
  };

  // Settings / catalog ------------------------------------------------------
  const ALL = ["claude", "codex", "pi", "opencode", "copilot", "cursor", "hermes", "gemini", "kimi", "crush", "dsh"];
  S.catalog = ALL.map((id) => ({ id, command: id === "cursor" ? "cursor-agent" : id, resumeArgs: null, loopArgs: null, enabled: true, installed: true, supportsMcp: true, supportsMcpAuth: false, acceptsPrompt: true, servesWebUi: false }));

  const H = (window.__MOCK_HANDLERS__ = {});
  const projectsAll = () => [...S.projects, S.workspace];
  const projectOf = (id) => projectsAll().find((p) => p.id === id);
  const runOf = (id) => S.runs.find((r) => r.id === String(id).split("--")[0]);

  H.list_projects = () => projectsAll().map((p) => ({ ...p }));
  H.get_workspace = () => ({ ...S.workspace });
  H.default_workspace_location = () => "/Users/demo/Workspace";
  H.agent_onboarding_needed = () => false;
  H.list_agent_catalog = () => S.catalog;
  H.list_profiles = () => ALL.map((name) => ({ name, command: name, args: [], env: [], resume_args: null, loop_args: name === "codex" ? ["exec", "--full-auto"] : name === "claude" ? ["-p", "--permission-mode", "acceptEdits"] : null }));
  H.agent_installed = () => true;
  H.get_settings = () => ({ lmStudioBaseUrl: "http://localhost:1234/v1", defaultAgent: null, defaultWorktree: true, shareOpenFile: false });
  H.get_notif_settings = () => ({ agentFinished: true, agentIdle: true, agentBlocked: true, runCrashed: true, mergeAttention: true, loopEvents: true, onlyWhenWatching: true, idleSecs: 60 });
  H.last_update_check = () => null;
  H.check_for_update = () => ({ current: "0.2.3", latest: "0.2.3", updateAvailable: false, url: "", notes: null, installKind: "mac-app", canInstall: false, manualHint: null, staged: null, installing: false, installError: null, error: null });
  H.get_update_check_enabled = () => true;
  H.list_agent_models = () => ALL.map((agent) => ({ agent, supported: ["claude", "codex", "opencode", "gemini"].includes(agent), suggested: agent === "claude" ? ["opus", "sonnet"] : [], recent: [], selected: null, listCommand: null }));
  H.tool_status = () => [];
  H.list_installs = () => [];
  H.list_mcp_servers = () => [];
  H.inspect_repo = () => ({ state: "ready", stageable: false, dirty: false, blocked: null });
  H.folder_missing = () => false;
  H.set_ui_state = () => null;
  H.set_menu_context = () => null;
  H.set_open_file = () => null;
  H.set_preview_rect = () => null;
  H.preview_targets = () => [];
  H.list_popouts = () => [];
  H.running_sessions = () => S.runs.length;
  H.agent_cli_info = () => [];
  H.get_git_identity = () => ({ name: "Sam Rivera", email: "sam@example.com" });

  H.list_runs = ({ projectId }) => S.runs.filter((r) => r.projectId === projectId && !r.archivedAt).map((r) => ({ ...r }));
  H.list_archived_runs = () => [];
  H.run_scripts_live = () => false;
  H.run_scripts_status = () => [];
  H.run_script_config = ({ target }) => ({ scripts: [{ name: "dev", command: "make dev", web: false, nonconcurrent: false }, { name: "test", command: "go test ./...", web: false, nonconcurrent: false }], shared: true, suggestions: [], workspace: "/Users/demo/code/orbit-api", port: null, previewPort: null, previewToolsEnabled: false });
  H.list_run_sessions = () => [];
  H.list_queued_messages = () => [];
  H.list_review_comments = () => [];
  H.run_status = ({ id }) => (runOf(id) ? runOf(id).status : { state: "gone" });
  H.ensure_run_active = () => null;
  H.resize_run = () => null;
  H.detach_run = () => null;
  H.run_input = ({ id, data }) => { window.__TERM__ && window.__TERM__.input(id, data); return null; };
  H.set_run_title = () => null;
  H.shell_status = () => ({ state: "gone" });

  // Issues
  H.list_issues = ({ projectId }) => S.issues.filter((i) => i.projectId === projectId).map((i) => ({ ...i }));
  H.update_issue = ({ id, patch }) => { const i = S.issues.find((x) => x.id === id); Object.assign(i, patch, { updatedAt: Math.floor(Date.now() / 1000) }); return { ...i }; };
  H.get_issue_sync_config = ({ projectId }) => ({ sync: false, auto: false, remote: "origin", remotes: ["origin"], fromRepo: false, issueKey: projectOf(projectId)?.issue_key || "ORB", keySharedWith: [] });

  // Docs / files
  function rootPid(root) { if (!root) return null; if (root.kind === "run") { const r = runOf(root.id); return r ? r.projectId : null; } return root.id; }
  S.rootPid = rootPid;
  function docsFor(root) { return S.docs[rootPid(root)]; }
  H.detect_docs_dir = ({ projectId, root }) => { const d = S.docs[rootPid(root) || projectId]; return d ? d.dir : null; };
  H.docs_corpus_stats = ({ root }) => {
    const d = docsFor(root); if (!d) return { files: [], dirs: [], attachments: [] };
    const files = Object.entries(d.files).map(([path, text]) => ({ path, mtimeMs: 1759800000000, size: text.length }));
    const dirs = [...new Set(Object.keys(d.files).filter((p) => p.includes("/")).map((p) => p.split("/").slice(0, -1).join("/")))].sort();
    return { files, dirs, attachments: [] };
  };
  H.read_docs_corpus = ({ root }) => { const d = docsFor(root); if (!d) return []; return Object.entries(d.files).map(([path, text]) => ({ path, text, tooLarge: false })); };
  H.read_docs_files = ({ root, paths }) => { const d = docsFor(root); if (!d) return []; return paths.filter((p) => d.files[p] != null).map((path) => ({ path, text: d.files[path], tooLarge: false })); };
  H.scan_tasks = ({ root }) => {
    const d = docsFor(root); if (!d) return [];
    const out = [];
    for (const [path, text] of Object.entries(d.files)) {
      text.split("\n").forEach((l, line) => { const m = l.match(/^\s*- \[( |x)\] (.*)$/); if (m) out.push({ path, line, checked: m[1] === "x", text: m[2] }); });
    }
    return out;
  };
  function fileText(root, relPath) {
    const d = S.docs[rootPid(root)];
    if (d) {
      const p = d.dir === "." ? relPath : relPath.replace(new RegExp("^" + d.dir + "/"), "");
      if (d.files[p] != null) return d.files[p];
    }
    if (window.__MOCK_FILES__ && window.__MOCK_FILES__[relPath] != null) return window.__MOCK_FILES__[relPath];
    return null;
  }
  H.read_file = ({ root, relPath }) => {
    const t = fileText(root, relPath);
    if (t == null) throw "No such file or directory (os error 2)";
    return { text: t, binary: false, tooLarge: false, mtimeMs: 1759800000000, size: t.length };
  };
  H.stat_file = ({ root, relPath }) => { const t = fileText(root, relPath); if (t == null) throw "not found"; return { mtimeMs: 1759800000000, size: t.length }; };
  H.write_file = ({ root, relPath, contents }) => { const d = S.docs[rootPid(root)]; if (d) { const p = d.dir === "." ? relPath : relPath.replace(new RegExp("^" + d.dir + "/"), ""); d.files[p] = contents; } return null; };
  H.list_dir = () => [];
  H.resolve_term_paths = ({ paths }) => paths.map(() => null);
  H.list_files = () => [];
  H.search_files = () => [];

  // Git
  H.git_status = () => [];
  H.git_branch_info = ({ taskId }) => {
    const r = runOf(taskId);
    return { branch: r ? r.branch : "main", upstream: null, ahead: r ? 3 : 0, behind: 0, base: r ? "main" : null, hasRemote: true, merging: false };
  };
  H.run_branches = ({ taskId }) => ({ branch: runOf(taskId)?.branch || "main", base: "main" });
  H.list_project_branches = () => ({ current: "main", branches: ["main", "develop"], remote: [] });
  H.git_list_branches = () => ({ current: "main", branches: ["main"], remote: [] });
  H.git_log_graph = () => [];
  H.git_stash_list = () => [];
  H.git_auto_fetch = () => false;
  H.gh_readiness = () => "ready";
  H.gh_auth_readiness = () => "ready";
  H.pr_status = () => ({ pr: null, checks: [] });
  H.pr_number_for_run = () => null;
  H.list_gh_prs = () => [];
  H.merge_status = () => ({ merging: false, unresolved: [], merged: false, blockedBy: null });
  H.checkpoint_list = () => [];
  H.get_files_config = () => ({ copy: [], detectedEnv: [] });
  H.get_knowledge_config = () => null;
  H.knowledge_graph_view = () => null;

  window.__MOCK_FALLBACK__ = (cmd) => {
    if (cmd.startsWith("list_") || cmd.endsWith("_list")) return [];
    return null;
  };
})();
