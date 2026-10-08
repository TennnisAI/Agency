// Git state for the demo runs: changed files with real-looking hunks, a
// branch history, and the merge flow.
(function () {
  const S = window.__MOCK_STATE__;
  const H = window.__MOCK_HANDLERS__;
  const NOW = Math.floor(Date.now() / 1000);
  const hunk = (header, lines) => ({ header, lines });
  const add = (s) => s.replace(/\t/g, "    ").split("\n").map((l) => "+" + l);
  const ctx = (s) => s.replace(/\t/g, "    ").split("\n").map((l) => " " + l);

  const BUCKET = `package ratelimit

import (
	"sync"
	"time"
)

// Bucket is a token bucket for one API key. Capacity is the burst a key may
// spend at once; refill is how many tokens come back per second.
type Bucket struct {
	mu       sync.Mutex
	tokens   float64
	capacity float64
	refill   float64
	last     time.Time
}

func NewBucket(capacity, refill float64, now time.Time) *Bucket {
	return &Bucket{tokens: capacity, capacity: capacity, refill: refill, last: now}
}

// Allow spends one token if there is one. When there is not, it says how long
// until there will be, which becomes the Retry-After header.
func (b *Bucket) Allow(now time.Time) (bool, time.Duration) {
	b.mu.Lock()
	defer b.mu.Unlock()
	elapsed := now.Sub(b.last).Seconds()
	b.tokens = min(b.capacity, b.tokens+elapsed*b.refill)
	b.last = now
	if b.tokens >= 1 {
		b.tokens--
		return true, 0
	}
	wait := (1 - b.tokens) / b.refill
	return false, time.Duration(wait * float64(time.Second))
}`;

  S.git = {
    "r-rate": {
      files: [
        { path: "internal/http/middleware.go", index: " ", worktree: "M", hunks: [
          hunk("@@ -88,6 +88,27 @@ func Recover(log *slog.Logger) Middleware {", [
            ...ctx("\t}\n}\n"),
            ...add("// RateLimit spends one token from the caller's bucket per request and\n// answers 429 with Retry-After when the bucket is empty.\nfunc RateLimit(store *ratelimit.Store) Middleware {\n\treturn func(next http.Handler) http.Handler {\n\t\treturn http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {\n\t\t\tkey := auth.KeyFrom(r.Context())\n\t\t\tok, wait := store.Allow(key, time.Now())\n\t\t\tif !ok {\n\t\t\t\tw.Header().Set(\"Retry-After\", strconv.Itoa(int(wait.Seconds())+1))\n\t\t\t\tproblem.Write(w, http.StatusTooManyRequests, \"rate_limited\")\n\t\t\t\treturn\n\t\t\t}\n\t\t\tnext.ServeHTTP(w, r)\n\t\t})\n\t}\n}\n"),
            ...ctx("// Timeout bounds every handler.\nfunc Timeout(d time.Duration) Middleware {"),
          ]),
        ] },
        { path: "internal/http/router.go", index: " ", worktree: "M", hunks: [
          hunk("@@ -41,9 +41,11 @@ func NewRouter(deps Deps) http.Handler {", [
            ...ctx("\tr := chi.NewRouter()\n\tr.Use(Recover(deps.Log))\n\tr.Use(RequestID)"),
            "-    r.Use(auth.Middleware(deps.Keys))",
            "+    r.Use(auth.Middleware(deps.Keys))",
            "+    r.Use(RateLimit(deps.Limits))",
            ...ctx("\tr.Use(Timeout(30 * time.Second))\n\n\tr.Route(\"/v2\", func(r chi.Router) {"),
          ]),
        ] },
        { path: "internal/ratelimit/bucket.go", index: " ", worktree: "?", hunks: [hunk("@@ -0,0 +1,38 @@", add(BUCKET))] },
        { path: "internal/ratelimit/store.go", index: " ", worktree: "?", hunks: [hunk("@@ -0,0 +1,12 @@", add("package ratelimit\n\n// Store holds one bucket per key, sized from the key's plan.\ntype Store struct {\n\tmu      sync.Mutex\n\tbuckets map[string]*Bucket\n\tplans   PlanLimits\n}"))] },
        { path: "internal/ratelimit/bucket_test.go", index: " ", worktree: "?", hunks: [hunk("@@ -0,0 +1,9 @@", add("func TestBucketRefills(t *testing.T) {\n\tnow := time.Unix(0, 0)\n\tb := NewBucket(2, 1, now)\n\tassertAllow(t, b, now, true)\n\tassertAllow(t, b, now, true)\n\tassertAllow(t, b, now, false)\n\tassertAllow(t, b, now.Add(time.Second), true)\n}"))] },
        { path: "config/plans.yaml", index: " ", worktree: "M", hunks: [hunk("@@ -1,6 +1,15 @@", [...ctx("plans:\n  free:\n    seats: 1"), "+    rate_limit: { per_minute: 60, burst: 120 }", ...ctx("  team:\n    seats: 10"), "+    rate_limit: { per_minute: 600, burst: 1200 }"])] },
      ],
      log: [
        ["c41f2a9", "add token bucket and per-key store", 600],
        ["9be0d14", "load plan limits from config", 1100],
      ],
    },
    "r-flaky": {
      files: [
        { path: "internal/webhooks/retry_test.go", index: "M", worktree: " ", hunks: [hunk("@@ -52,10 +52,10 @@ func TestRetryBacksOff(t *testing.T) {", [
          ...ctx("\tsrv := failingServer(t, 1)\n\td := NewDispatcher(srv.URL, WithClock(clock))\n\td.Send(ctx, evt)"),
          "-    time.Sleep(50 * time.Millisecond)", "-    if got := srv.Attempts(); got != 2 {",
          "+    clock.Advance(backoff.Max(1))", "+    if got := waitForAttempts(t, srv, 2); got != 2 {",
          ...ctx("\t\tt.Fatalf(\"attempts = %d, want 2\", got)\n\t}"),
        ])] },
        { path: "internal/webhooks/helpers_test.go", index: "M", worktree: " ", hunks: [hunk("@@ -12,3 +12,12 @@", [...ctx("}"), ...add("\n// waitForAttempts polls the fake server instead of sleeping a fixed time.\nfunc waitForAttempts(t *testing.T, srv *fakeServer, n int) int {\n\tt.Helper()\n\tdeadline := time.Now().Add(2 * time.Second)\n\tfor srv.Attempts() < n && time.Now().Before(deadline) {\n\t\truntime.Gosched()\n\t}\n\treturn srv.Attempts()\n}")])] },
      ],
      log: [["7d2e0b1", "drive the retry test from a fake clock", 300]],
    },
  };
  // Deterministic, realistic-looking 40-hex ids from a 7-char prefix.
  const full = (h) => { let x = parseInt(h, 16) || 7; let out = h; while (out.length < 40) { x = (x * 1103515245 + 12345) >>> 0; out += (x >>> 8).toString(16).padStart(6, "0"); } return out.slice(0, 40); };
  S.fullHash = full;
  const gitOf = (id) => S.git[String(id).split("--")[0]];

  function unified(f) {
    let out = `diff --git a/${f.path} b/${f.path}\n--- a/${f.path}\n+++ b/${f.path}\n`;
    for (const h of f.hunks) out += h.header + "\n" + h.lines.join("\n") + "\n";
    return out;
  }
  const runOf = (id) => S.runs.find((r) => r.id === String(id).split("--")[0]);

  H.git_status = ({ taskId }) => { const g = gitOf(taskId); return g && !g.committed ? g.files.map(({ path, index, worktree }) => ({ path, index, worktree })) : []; };
  H.git_parse_diff = ({ taskId, path }) => { const f = gitOf(taskId)?.files.find((x) => x.path === path); return f ? { header: `diff --git a/${path} b/${path}`, hunks: f.hunks } : { header: "", hunks: [] }; };
  H.git_diff = ({ taskId, path }) => { const f = gitOf(taskId)?.files.find((x) => x.path === path); return f ? unified(f) : ""; };
  H.git_commit_diff = ({ taskId, path }) => { const f = gitOf(taskId)?.files.find((x) => x.path === path); return f ? unified(f) : ""; };
  H.git_commit_files = ({ taskId }) => (gitOf(taskId)?.files || []).slice(0, 3).map((f) => ({ path: f.path, status: f.worktree === "?" ? "A" : "M" }));
  H.git_log_graph = ({ taskId }) => {
    const base = [
      ["e19a3c7", "Merge branch 'agent/structured-logging'", 86400, ["main", "origin/main"]],
      ["a4b8f02", "structured logging for request handlers", 90000],
      ["5c0d9e3", "upgrade CI to Postgres 17", 172800],
      ["0f7e6a1", "seed script for local development", 259200],
      ["b2c91de", "return Problem Details from the auth middleware", 345600],
      ["8e44f10", "move webhooks onto the job queue", 432000],
    ];
    const g = gitOf(taskId);
    const r = runOf(taskId);
    const own = g ? g.log.map(([h, s, ago], i) => [h, s, ago, i === 0 && r ? [r.branch] : []]) : [];
    const all = [...own, ...base];
    return all.map(([hash, subject, ago, refs], i) => ({
      hash: full(hash), parents: i < all.length - 1 ? [full(all[i + 1][0])] : [],
      author: i < own.length ? "Sam Rivera" : ["Sam Rivera", "Ada Okafor", "Jun Park"][i % 3], email: "sam@example.com",
      date: NOW - ago, subject, refs: refs || [],
    }));
  };
  H.git_branch_info = ({ taskId }) => {
    const r = runOf(taskId);
    return { branch: r ? r.branch : "main", upstream: null, ahead: r ? (gitOf(taskId)?.log.length || 2) : 0, behind: 0, base: r ? "main" : null, hasRemote: true, merging: false };
  };
  H.merge_preview = ({ taskId }) => ({ base: "main", branch: runOf(taskId)?.branch || "", commitsAhead: (gitOf(taskId)?.log.length || 2) + 1, commitsBehind: 0, worktreeDirty: !!(gitOf(taskId) && !gitOf(taskId).committed), dirtyFiles: (gitOf(taskId)?.files || []).map((f) => f.path), remoteBranches: [] });
  H.git_stage_all = () => null;
  H.git_commit = ({ taskId }) => { const g = gitOf(taskId); if (g) g.committed = true; return null; };
  H.merge_task = ({ taskId, onProgress }) => new Promise((res) => {
    const steps = ["Checking out main", "Merging " + (runOf(taskId)?.branch || ""), "Done"];
    steps.forEach((p, i) => setTimeout(() => window.__mock.sendChannel(onProgress, { phase: p, percent: null, detail: "" }), 250 * i));
    setTimeout(() => {
      const r = runOf(taskId);
      if (r) { r.merged = true; r.activity = { state: "done", since: Date.now(), reported: true }; }
      const i = S.issues.find((x) => x.id === r?.issueId);
      if (i) i.status = "done";
      res({ kind: "clean", commit: full("3f9a2c1") });
    }, 900);
  });
  H.merge_status = ({ taskId }) => ({ merging: false, unresolved: [], merged: !!runOf(taskId)?.merged, blockedBy: null });
  H.run_cleanup = ({ id }) => ({
    facts: { ownsBranch: true, cutBranch: true, commitsAhead: 0, commitsKnown: true, merged: true, pushed: false, gone: false, dirty: false, baseExists: true },
    archive: { removesWorktree: true, deletesBranch: true, keepsBranch: false, leavesBranch: false, keepsRecord: true, restorable: true, commitsAtRisk: 0, losesUncommitted: false, safeBecause: "merged" },
    delete: { removesWorktree: true, deletesBranch: true, keepsBranch: false, leavesBranch: false, keepsRecord: false, restorable: false, commitsAtRisk: 0, losesUncommitted: false, safeBecause: "merged" },
    branch: runOf(id)?.branch || "", base: "main", managesTranscript: true,
  });
  H.archive_run = ({ id, onProgress }) => new Promise((res) => {
    window.__mock.sendChannel(onProgress, { phase: "Removing worktree", percent: null, detail: "" });
    setTimeout(() => { const r = runOf(id); if (r) r.archivedAt = Math.floor(Date.now() / 1000); res(null); }, 500);
  });
  H.pr_merge_methods = () => ({ merge: true, squash: true, rebase: true });
})();
