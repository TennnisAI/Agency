// What each demo run's agent is doing: history already on screen, then a
// live tail that plays out during capture.
(function () {
  const T = window.__TERM__;
  const { C } = T;
  const R = "\x1b[0m", B = "\x1b[1m";
  const S = window.__MOCK_STATE__;
  const proj = (id) => [...S.projects, S.workspace].find((p) => p.id === id);
  const ok = (s) => `${C.green}ok${R}  ${s}`;
  const pass = (n, t) => `${C.green}${B}PASS${R} ${n} ${C.dim}(${t})${R}`;

  function session(run) {
    const p = proj(run.projectId);
    return T.mk(run.id, run.agent, { cwd: `/Users/demo/code/${p.name}`, branch: run.branch });
  }

  const SCRIPTS = {};
  window.__SCRIPTS__ = SCRIPTS;

  // ---- orbit-api ----
  SCRIPTS["r-rate"] = {
    hist: [
      ["prompt", "Add per-key rate limiting to the public API. Token bucket per key, plan-configurable, 429 with Retry-After. ORB-41."],
      ["say", "I'll start by looking at how requests flow through the middleware chain."],
      ["read", "internal/http/router.go", 188],
      ["read", "internal/http/middleware.go", 142],
      ["search", "PlanLimits", 3],
      ["todo", [[true, "Token bucket keyed by API key"], [true, "Plan limits from config"], [false, "Middleware returning 429 + Retry-After"], [false, "Integration tests"]]],
      ["edit", "internal/ratelimit/bucket.go", [
        "+ type Bucket struct {", "+ 	mu       sync.Mutex", "+ 	tokens   float64", "+ 	capacity float64", "+ 	refill   float64 // tokens per second", "+ 	last     time.Time", "+ }", "+ ",
        "+ func (b *Bucket) Allow(now time.Time) (bool, time.Duration) {", "+ 	b.mu.Lock()", "+ 	defer b.mu.Unlock()"], 48, 0],
    ],
    live: [
      ["busy", "Writing middleware", { w: 2600 }],
      ["edit", "internal/http/middleware.go", [
        "  func RateLimit(store *ratelimit.Store) Middleware {", "  	return func(next http.Handler) http.Handler {",
        "+ 		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {", "+ 			key := auth.KeyFrom(r.Context())",
        "+ 			ok, wait := store.Allow(key, time.Now())", "+ 			if !ok {", "+ 				w.Header().Set(\"Retry-After\", strconv.Itoa(int(wait.Seconds())+1))",
        "+ 				problem.Write(w, http.StatusTooManyRequests, \"rate_limited\")", "+ 				return", "+ 			}"], 31, 4, { w: 1800 }],
      ["busy", "Running the integration suite", { w: 2800 }],
      ["bash", "go test ./internal/... -run RateLimit", [ok("orbit-api/internal/ratelimit   0.412s"), ok("orbit-api/internal/http        1.873s")], { w: 1600 }],
      ["busy", "Checking the burst behaviour", { w: 3000 }],
      ["bash", "go test ./... -count=1", [ok("orbit-api/internal/auth        0.208s"), ok("orbit-api/internal/http        1.902s"), ok("orbit-api/internal/ratelimit   0.398s"), ok("orbit-api/internal/store       3.114s")], { w: 1800 }],
      ["idle"],
      ["say", `Rate limiting is in. Keys get a token bucket sized from their plan; over the limit returns ${B}429${R} with ${B}Retry-After${R}. All tests pass.`, { w: 60000 }],
    ],
  };

  SCRIPTS["r-pagi"] = {
    hist: [
      ["prompt", "Cursor-based pagination for /v2/events. Offset pagination falls over past page 200. ORB-42"],
      ["read", "internal/http/handlers/events.go"],
      ["read", "internal/store/events.go"],
      ["say", "Plan: encode (created_at, id) as an opaque cursor, keep `limit`, drop `page` from v2 only."],
      ["todo", [[true, "Cursor encode/decode"], [false, "Store query with keyset predicate"], [false, "Handler + tests"]]],
    ],
    live: [
      ["busy", "Working", { w: 2200 }],
      ["edit", "internal/store/events.go", [
        "  func (s *Store) ListEvents(ctx context.Context, q EventQuery) ([]Event, error) {",
        "- 	rows, err := s.db.Query(ctx, listSQL+\" OFFSET $2 LIMIT $3\", q.Tenant, q.Offset, q.Limit)",
        "+ 	rows, err := s.db.Query(ctx, listSQL+` AND (created_at, id) < ($2, $3)", "+ 		ORDER BY created_at DESC, id DESC LIMIT $4`,",
        "+ 		q.Tenant, q.After.CreatedAt, q.After.ID, q.Limit)"], 42, 18, { w: 2000 }],
      ["busy", "Running tests", { w: 2400 }],
      ["bash", "go test ./internal/store/... ./internal/http/...", ["ok  orbit-api/internal/store  2.871s", "ok  orbit-api/internal/http   1.644s"], { w: 1800 }],
      ["busy", "Updating the OpenAPI spec", { w: 2600 }],
      ["edit", "api/openapi.yaml", ["      parameters:", "-       - $ref: '#/components/parameters/Page'", "+       - $ref: '#/components/parameters/Cursor'", "+       - $ref: '#/components/parameters/Limit'"], 9, 3, { w: 2000 }],
      ["idle", { w: 60000 }],
    ],
  };

  SCRIPTS["r-otel"] = {
    hist: [
      ["prompt", "Wire OpenTelemetry spans through the job queue so a webhook delivery shows up under the request that enqueued it."],
      ["say", "I'll look at how jobs are enqueued and where the worker picks them up."],
      ["read", "internal/queue/enqueue.go"],
      ["read", "internal/queue/worker.go"],
      ["edit", "internal/queue/enqueue.go", ["  func (q *Queue) Enqueue(ctx context.Context, j Job) error {", "+ 	carrier := propagation.MapCarrier{}", "+ 	otel.GetTextMapPropagator().Inject(ctx, carrier)", "+ 	j.Trace = carrier"], 14, 0],
      ["say", "I need to add the OpenTelemetry SDK to go.mod."],
      ["block", "go get go.opentelemetry.io/otel/sdk@v1.31.0"],
    ],
    live: [],
  };

  SCRIPTS["r-flaky"] = {
    hist: [
      ["prompt", "Fix the flaky webhook retry test. Fails about 1 in 20 on CI. ORB-44"],
      ["read", "internal/webhooks/retry_test.go", 96],
      ["say", "The test sleeps 50ms and assumes the second attempt has fired. Backoff has jitter, so sometimes it hasn't."],
      ["edit", "internal/webhooks/retry_test.go", ["- 	time.Sleep(50 * time.Millisecond)", "+ 	clock.Advance(backoff.Max(1))", "+ 	waitForAttempts(t, srv, 2)"], 9, 2],
      ["bash", "go test ./internal/webhooks -run Retry -count=200", ["ok  orbit-api/internal/webhooks  6.204s"]],
      ["say", "200 consecutive runs pass. The test now drives a fake clock instead of sleeping. Ready for review."],
    ],
    live: [],
  };

  SCRIPTS["r-mig"] = {
    hist: [
      ["prompt", "Migrate config loading to typed settings. Replace the string map with a struct and validate on boot."],
      ["read", "internal/config/config.go", 211],
      ["search", "cfg.Get(", 23],
      ["say", "23 call sites use cfg.Get. I'll introduce Settings, migrate packages one at a time."],
      ["edit", "internal/config/settings.go", ["+ type Settings struct {", "+ 	HTTP     HTTPSettings     `env:\"HTTP\"`", "+ 	Database DatabaseSettings `env:\"DATABASE\"`", "+ 	Queue    QueueSettings    `env:\"QUEUE\"`", "+ }"], 64, 0],
    ],
    live: [
      ["busy", "Migrating internal/http", { w: 2400 }],
      ["edit", "internal/http/server.go", ["- 	addr := cfg.Get(\"http.addr\")", "+ 	addr := s.HTTP.Addr", "- 	timeout, _ := time.ParseDuration(cfg.Get(\"http.timeout\"))", "+ 	timeout := s.HTTP.Timeout"], 2, 2, { w: 1800 }],
      ["busy", "Migrating internal/store", { w: 2400 }],
      ["edit", "internal/store/store.go", ["- 	dsn := cfg.Get(\"database.url\")", "+ 	dsn := s.Database.URL"], 1, 1, { w: 1600 }],
      ["bash", "go build ./...", [`${C.green}build ok${R}`], { w: 1500 }],
      ["busy", "Migrating internal/queue", { w: 60000 }],
    ],
  };

  SCRIPTS["r-docs"] = {
    hist: [
      ["prompt", "Document the auth flow in docs/auth.md, from key creation to request verification."],
      ["read", "internal/auth/keys.go", 134],
      ["read", "internal/auth/verify.go", 88],
      ["read", "docs/architecture.md", 41],
    ],
    live: [
      ["busy", "Writing", { w: 2600 }],
      ["edit", "docs/auth.md", ["+ ## Creating a key", "+ ", "+ Keys are created per project. The plaintext is shown once;", "+ we store an argon2id hash and an 8-character prefix for lookup."], 22, 1, { w: 2200 }],
      ["busy", "Writing", { w: 2600 }],
      ["edit", "docs/auth.md", ["+ ## Verifying a request", "+ ", "+ The middleware looks the key up by prefix, then verifies the hash", "+ in constant time. See [[architecture]] for where this sits."], 18, 0, { w: 2200 }],
      ["busy", "Checking links", { w: 60000 }],
    ],
  };

  // ---- other projects ----
  SCRIPTS["r-lum1"] = {
    hist: [["prompt", "Dark mode for the settings pages. Use the existing tokens, no new colours."], ["read", "src/styles/tokens.css", 120], ["search", "bg-white", 14]],
    live: [["busy", "Replacing hard-coded colours", { w: 3000 }], ["edit", "src/pages/settings/Profile.tsx", ["- <div className=\"bg-white text-gray-900\">", "+ <div className=\"bg-surface text-fg\">"], 1, 1, { w: 2400 }], ["edit", "src/pages/settings/Billing.tsx", ["- <section className=\"border-gray-200\">", "+ <section className=\"border-line\">"], 1, 1, { w: 2400 }], ["busy", "Running visual checks", { w: 60000 }]],
  };
  SCRIPTS["r-lum2"] = {
    hist: [["prompt", "Replace moment with date-fns across the app."], ["search", "from 'moment'", 14], ["bash", "pnpm remove moment && pnpm add date-fns", ["+ date-fns 4.1.0", "- moment 2.30.1"]], ["bash", "pnpm test", [pass("src/lib/dates.test.ts", "41 ms"), pass("src/components/Calendar.test.tsx", "212 ms")]], ["say", "Done. 14 files migrated, bundle is 61 KB smaller."]],
    live: [],
  };
  SCRIPTS["r-tide1"] = {
    hist: [["prompt", "Stream sensor readings over websockets instead of polling every 5s."], ["read", "server/routes/readings.ts"], ["read", "client/src/hooks/useReadings.ts"]],
    live: [["busy", "Working", { w: 2600 }], ["edit", "server/ws/readings.ts", ["+ wss.on(\"connection\", (socket, req) => {", "+   const station = stationFrom(req.url);", "+   const unsub = bus.subscribe(station, (r) => socket.send(JSON.stringify(r)));", "+   socket.on(\"close\", unsub);", "+ });"], 38, 0, { w: 2400 }], ["busy", "Updating the client hook", { w: 60000 }]],
  };
  SCRIPTS["r-tide2"] = {
    hist: [["prompt", "Speed up the nightly rollup query. It takes 40 minutes now."], ["bash", "psql -c 'EXPLAIN ANALYZE ...'", ["Seq Scan on readings  (cost=0.00..1843021.11)", "Execution Time: 2391044.112 ms"]]],
    live: [["busy", "Thinking", { w: 3000 }], ["say", "It's a sequential scan over 90 days. A BRIN index on recorded_at fits this append-only table."], ["edit", "db/migrations/0031_readings_brin.sql", ["+ CREATE INDEX CONCURRENTLY readings_recorded_brin", "+   ON readings USING brin (recorded_at);"], 2, 0, { w: 2400 }], ["busy", "Measuring", { w: 60000 }]],
  };
  SCRIPTS["r-led1"] = {
    hist: [["prompt", "CSV import for bank statements. Detect the delimiter and date format."], ["read", "src/import/mod.rs"], ["read", "src/model.rs"]],
    live: [["busy", "Working", { w: 2600 }], ["edit", "src/import/csv.rs", ["+ pub fn sniff_delimiter(sample: &str) -> u8 {", "+     [b',', b';', b'\\t'].into_iter()", "+         .max_by_key(|d| sample.lines().take(5).map(|l| l.bytes().filter(|b| b == d).count()).min())", "+         .unwrap_or(b',')", "+ }"], 41, 0, { w: 2400 }], ["bash", "cargo test import", ["test import::csv::sniffs_semicolons ... ok", "test import::csv::parses_iso_dates ... ok"], { w: 60000 }]],
  };

  window.__startSession = function (run, script) {
    const s = session(run);
    if (script) {
      T.history(run.id, script.hist || []);
      if (window.__GO__) T.play(run.id, script.live || []);
      else (window.__PENDING__ = window.__PENDING__ || []).push([run.id, script.live || []]);
    }
    return s;
  };

  // Live tails wait for the recorder to say go, so boot time doesn't eat them.
  window.__go = function () {
    window.__GO__ = true;
    for (const [id, live] of window.__PENDING__ || []) T.play(id, live);
    window.__PENDING__ = [];
  };

  // Boot: every seeded run gets its session.
  for (const r of S.runs) window.__startSession(r, SCRIPTS[r.id]);
})();
