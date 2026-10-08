// User actions that create or change runs: dispatching an issue, racing,
// looping. Each new run gets a scripted session.
(function () {
  const S = window.__MOCK_STATE__;
  const H = window.__MOCK_HANDLERS__;
  const T = window.__TERM__;
  const { C } = T;
  const R = "\x1b[0m", B = "\x1b[1m";
  const nowS = () => Math.floor(Date.now() / 1000);
  const slug = (t) => t.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").split("-").slice(0, 4).join("-");
  const proj = (id) => [...S.projects, S.workspace].find((p) => p.id === id);
  let seq = 500;

  function newRun(projectId, agent, title, extra) {
    const run = S.mkRun(Object.assign({
      id: `r${seq++}`, projectId, agent, title, prompt: title, branch: `agent/${slug(title)}`,
      createdAt: nowS(), added: 0, deleted: 0, files: 0,
      activity: { state: "working", since: Date.now(), reported: true },
    }, extra || {}));
    S.runs.push(run);
    return run;
  }
  // Diffstat grows while the agent works.
  function grow(run, add, del, files, ms) {
    const steps = 12;
    for (let i = 1; i <= steps; i++) setTimeout(() => {
      run.added = Math.round((add * i) / steps); run.deleted = Math.round((del * i) / steps); run.files = Math.max(1, Math.round((files * i) / steps));
    }, (ms * i) / steps);
  }

  // ---- Issue dispatch ----
  const ISSUE_WORK = {
    "Support idempotency keys on POST /payments": [
      ["say", "I'll read the payments handler and how responses are written, then add an idempotency store.", { w: 1300 }],
      ["read", "internal/http/handlers/payments.go", 164, { w: 900 }],
      ["read", "internal/store/payments.go", 97, { w: 900 }],
      ["todo", [[false, "idempotency_keys table + migration"], [false, "Middleware: replay stored response"], [false, "422 on body mismatch"], [false, "Tests: replay, mismatch, expiry"]], { w: 1500 }],
      ["busy", "Writing the migration", { w: 1800 }],
      ["edit", "db/migrations/0032_idempotency_keys.sql", ["+ CREATE TABLE idempotency_keys (", "+   key         text        NOT NULL,", "+   tenant_id   uuid        NOT NULL,", "+   body_sha256 bytea       NOT NULL,", "+   response    jsonb       NOT NULL,", "+   created_at  timestamptz NOT NULL DEFAULT now(),", "+   PRIMARY KEY (tenant_id, key)", "+ );"], 9, 0, { w: 1600 }],
      ["busy", "Writing the middleware", { w: 60000 }],
    ],
  };

  H.start_issue_run = ({ issueId, agent, model }) => {
    const issue = S.issues.find((i) => i.id === issueId);
    issue.status = "in_progress";
    issue.updatedAt = nowS();
    const run = newRun(issue.projectId, agent, issue.title, { issueId, model: model || null, branch: `agent/${proj(issue.projectId).issue_key.toLowerCase()}-${issue.seq}-${slug(issue.title).split("-").slice(0, 2).join("-")}` });
    window.__startSession(run, {
      hist: [],
      live: [["prompt", `${proj(issue.projectId).issue_key}-${issue.seq}: ${issue.title}`, { w: 400 }], ["busy", "Reading the issue", { w: 1400 }], ["idle", { w: 10 }], ...(ISSUE_WORK[issue.title] || [])],
    });
    grow(run, 142, 3, 4, 12000);
    return { ...run };
  };

  // ---- Race ----
  const RACE_FLAVOUR = {
    claude: [["say", "Let me look at how the editor handles key events today."], ["read", "src/editor/Editor.tsx", 312], ["search", "onKeyDown", 6], ["busy", "Designing a keymap", { w: 2200 }], ["edit", "src/editor/keymap.ts", ["+ export const KEYMAP: Binding[] = [", "+   { keys: \"mod+b\", run: toggleBold },", "+   { keys: \"mod+i\", run: toggleItalic },", "+   { keys: \"mod+k\", run: insertLink },", "+ ];"], 46, 0], ["busy", "Wiring it into the editor", { w: 60000 }]],
    codex: [["read", "src/editor/Editor.tsx"], ["say", "I'll add a small keymap module and a hook that installs it."], ["busy", "Working", { w: 2000 }], ["edit", "src/editor/useShortcuts.ts", ["+ export function useShortcuts(view: EditorView) {", "+   useEffect(() => {", "+     const off = bindKeys(view.dom, KEYMAP);", "+     return off;", "+   }, [view]);", "+ }"], 31, 0], ["busy", "Working", { w: 60000 }]],
    gemini: [["say", "I will first examine the editor component and its tests."], ["read", "src/editor/Editor.tsx"], ["read", "src/editor/Editor.test.tsx"], ["busy", "Planning the shortcut handler", { w: 2400 }], ["edit", "src/editor/shortcuts.ts", ["+ const isMod = (e: KeyboardEvent) => (isMac ? e.metaKey : e.ctrlKey);", "+ export function handleShortcut(e: KeyboardEvent, ed: Editor) {", "+   if (isMod(e) && e.key === \"b\") return ed.toggle(\"bold\");"], 38, 2], ["busy", "Writing tests", { w: 60000 }]],
    opencode: [["read", "src/editor/Editor.tsx", 312], ["read", "package.json", 64], ["search", "keydown", 3], ["busy", "Thinking", { w: 2400 }], ["edit", "src/editor/Editor.tsx", ["-   <div className=\"editor\" ref={ref}>", "+   <div className=\"editor\" ref={ref} onKeyDown={onShortcut}>"], 18, 1], ["busy", "Adding tests", { w: 60000 }]],
    cursor: [["read", "src/editor/Editor.tsx"], ["busy", "Thinking", { w: 2400 }], ["edit", "src/editor/hotkeys.ts", ["+ export const hotkeys = {", "+   bold: \"Mod-b\",", "+   italic: \"Mod-i\",", "+ };"], 22, 0], ["busy", "Working", { w: 60000 }]],
  };
  H.create_race = ({ projectId, prompt, attempts }) => {
    const raceId = `race-${seq++}`;
    const runs = attempts.map((a, i) => {
      const run = newRun(projectId, a.agent, prompt, { raceId, model: a.model || null, branch: `agent/${slug(prompt)}-${a.agent}` });
      window.__startSession(run, { hist: [], live: [["prompt", prompt, { w: 300 + i * 250 }], ["busy", "Starting", { w: 900 + i * 300 }], ["idle", { w: 10 }], ...(RACE_FLAVOUR[a.agent] || RACE_FLAVOUR.opencode)] });
      grow(run, 60 + i * 23, 2 + i * 3, 2 + (i % 3), 14000);
      return { ...run };
    });
    return runs;
  };
  H.start_issue_race = ({ issueId, attempts }) => {
    const issue = S.issues.find((i) => i.id === issueId);
    issue.status = "in_progress";
    return H.create_race({ projectId: issue.projectId, prompt: issue.title, attempts });
  };

  // ---- Loop ----
  // Headless exec-style output: timestamped lines, no TUI footer.
  // Local time, not toISOString(): UTC printed 12:32 under an app clock reading 14:32.
  const p2 = (n) => String(n).padStart(2, "0");
  const ts = () => { const d = new Date(); return `${C.dim}[${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}T${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}]${R}`; };
  T.styles.exec = {
    header: (s) => [`${ts()} OpenAI Codex (exec)`, `${C.dim}--------${R}`, `${B}workdir:${R} ${s.cwd}`, `${B}approval:${R} never`, `${C.dim}--------${R}`],
    prompt: (t) => [`${ts()} ${C.cyan}${B}User instructions:${R}`, ...t.split("\n")],
    say: (t) => [`${ts()} ${C.purple}${B}codex${R}`, t],
    read: (f) => [`${ts()} ${C.purple}${B}exec${R} ${B}bash -lc 'sed -n 1,200p ${f}'${R}`, `${ts()} ${C.green}bash -lc 'sed -n 1,200p ${f}' succeeded${R}`],
    search: (q, n) => [`${ts()} ${C.purple}${B}exec${R} ${B}rg -n '${q}'${R} ${C.dim}(${n} matches)${R}`],
    edit: (f, lines, add, del) => [`${ts()} ${C.yellow}${B}apply_patch${R} ${f} ${C.green}+${add}${R} ${C.red}-${del}${R}`, ...lines.map((l) => (l[0] === "+" ? `${C.green}${l}${R}` : l[0] === "-" ? `${C.red}${l}${R}` : `${C.dim}${l}${R}`))],
    bash: (cmd, o) => [`${ts()} ${C.purple}${B}exec${R} ${B}bash -lc '${cmd}'${R}`, ...o],
    todo: () => [],
    footer: (s) => (s.busy ? [`${C.dim}${["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][s.tick % 10]} ${s.busy}${R}`] : [""]),
  };

  H.create_loop = ({ projectId, prompt, agent, checkCommand, maxAttempts, maxWallSecs, maxTokens }) => {
    const run = newRun(projectId, agent, prompt, {
      loopConfig: { checkCommand, maxAttempts, checkTimeoutSecs: 600, maxWallSecs: maxWallSecs ?? null, maxTokens: maxTokens ?? null },
      loopState: { status: "awaitingAgent", attempt: 1, consecutiveFailures: 0, lastCheckExit: null, startedAt: nowS(), tokensUsed: 0, stallReason: null, updatedAt: nowS() },
    });
    const s = T.mk(run.id, "exec", { cwd: proj(projectId).repo_path, branch: run.branch });
    const st = (patch) => () => Object.assign(run.loopState, patch, { updatedAt: nowS() });
    const call = (fn) => ["call", fn];
    const FAIL = [`${C.red}${B}FAIL${R} test/ingest.test.ts`, `  ${C.red}●${R} ingest › rejects readings with no station id`, `    Expected: 400   Received: 500`, ``, `Tests: ${C.red}1 failed${R}, ${C.green}23 passed${R}, 24 total`];
    const PASS = [`${C.green}${B}PASS${R} test/ingest.test.ts`, `${C.green}${B}PASS${R} test/rollup.test.ts`, `${C.green}${B}PASS${R} test/stations.test.ts`, ``, `Tests: ${C.green}24 passed${R}, 24 total`];
    T.play(run.id, [
      ["prompt", prompt, { w: 600 }],
      ["busy", "thinking", { w: 1400 }],
      ["idle", { w: 10 }],
      ["say", "Validating readings at the ingest boundary with the shared zod schema.", { w: 900 }],
      ["read", "src/ingest/handler.ts", { w: 700 }],
      ["edit", "src/ingest/handler.ts", ["+  const parsed = ReadingSchema.safeParse(body);", "+  if (!parsed.success) return reply.code(400).send(parsed.error);"], 6, 1, { w: 1000 }],
      ["say", "Done. Handing off to the check.", { w: 500 }],
      call(st({ status: "checking" })), ["raw", [""], { w: 10 }],
      ["bash", checkCommand, FAIL, { w: 1400 }],
      call(st({ status: "awaitingAgent", attempt: 2, consecutiveFailures: 1, lastCheckExit: 1, tokensUsed: 41200 })),
      ["raw", ["", `${C.dim}${"─".repeat(30)} attempt 2 ${"─".repeat(30)}${R}`, ""], { w: 300 }],
      ["prompt", prompt + `\n\nThe check failed: ${checkCommand} exited 1.`, { w: 600 }],
      ["busy", "thinking", { w: 1200 }],
      ["idle", { w: 10 }],
      ["say", "The handler returns 500 because a missing station id throws in the lookup before validation runs. Moving the parse first.", { w: 900 }],
      ["edit", "src/ingest/handler.ts", ["-  const station = await stations.get(body.stationId);", "   const parsed = ReadingSchema.safeParse(body);", "   if (!parsed.success) return reply.code(400).send(parsed.error);", "+  const station = await stations.get(parsed.data.stationId);"], 1, 1, { w: 1000 }],
      ["say", "Done. Handing off to the check.", { w: 500 }],
      call(st({ status: "checking" })), ["raw", [""], { w: 10 }],
      ["bash", checkCommand, PASS, { w: 900 }],
      call(st({ status: "complete", lastCheckExit: 0, consecutiveFailures: 0, tokensUsed: 77800 })),
      call(() => { run.activity = { state: "done", since: Date.now(), reported: true }; }),
    ], { gap: 900 });
    grow(run, 9, 2, 1, 12000);
    return { ...run };
  };
  H.start_issue_loop = (a) => { const issue = S.issues.find((i) => i.id === a.issueId); return H.create_loop({ ...a, projectId: issue.projectId, prompt: issue.title }); };
  H.stop_loop = ({ id }) => { const r = S.runs.find((x) => x.id === id); if (r) r.loopState.status = "stopped"; return null; };

  H.create_run = ({ projectId, agent }) => {
    const run = newRun(projectId, agent, null, { branch: `agent/${seq}` });
    window.__startSession(run, { hist: [], live: [] });
    return { ...run };
  };
})();
