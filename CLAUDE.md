# Working in this repo

Agency is a Tauri 2 desktop app: a Rust backend in `crates/` and a React + Vite
frontend in `ui/`. See `README.md` for the layout and build instructions, and
`docs/` for the design record.

## Git hygiene

These two rules exist because both have already been broken here, and both are
expensive to fix after the fact.

- **No AI attribution, ever.** No `Co-Authored-By: Claude` trailer, no
  "Generated with" line, no session URL, in a commit message or a PR body. Write
  the message and stop. Nick is the sole copyright holder and the history is the
  evidence for that; a co-author trailer is a claim against it.
- **No competitor product name in a commit message, a branch name, or a PR
  title.** The design record discusses competitors by name and that is fine, but
  git metadata is a different artifact with a different audience, and a merged
  PR's branch name cannot be edited afterwards. Describe what the commit does,
  not who it is about: "Add a competitor reading to the competitive landscape",
  never the product's name.

`.githooks/commit-msg` enforces both, and `./dev.sh` points `core.hooksPath` at
it. The hook only knows the distinctive names. Competitors whose names are
ordinary English words — Conductor and Crystal today — are deliberately left out
of it, because "crystal-clear" would trip the guard and a guard that cries wolf
gets bypassed into uselessness. Those are your judgment call, not the hook's.

Commit subjects are imperative and lower-case after the first word. Branch names
describe the work.

## Build and test

```sh
cargo fmt                                  # rustfmt.toml: max_width 100, small_heuristics Max
cargo test -p agency-core -p agency-app
./dev.sh                                   # builds the termd sidecar, then tauri dev
```

Frontend, from the repo root:

```sh
pnpm --dir ui install
ui/node_modules/.bin/tsc --noEmit
ui/node_modules/.bin/vite build
```

- **Use `pnpm`, never `npm`.** The lockfile is pnpm's, and npm churns it.
- **Invoke the frontend binaries directly**, as above. pnpm 11's
  `exec`/`test`/`build` wrappers exit 1 on an esbuild ignored-build even when the
  underlying command succeeded, so a green build looks red.
- **`./dev.sh`, not `tauri dev`.** Tauri's `beforeDevCommand` starts Vite only;
  it does not build the `agency-termd` daemon, and `AppState::new()` fails
  without it. A *stale* `target/debug/agency-termd` is worse than a missing one:
  it runs old daemon code silently. Rebuild it when you touch anything under
  `crates/agency-core/src/term/`.
- Tests leak orphan `agency-termd` processes. Never kill the one under
  Application Support; that is the user's live daemon.

## User-facing copy

- **No emoji anywhere in the UI.** Icons are monochrome text glyphs (`◈ ⇋ ≳ ∥`).
  If a glyph has an emoji presentation, pin it with U+FE0E.
- **No em dashes in UI copy.** Split the sentence, or use a comma, semicolon,
  colon or parentheses. Docs are exempt; strings the user reads are not.
- Sentence case, second person, present tense.
- Say "no first-party data collection", never "nothing leaves your machine".
  Wherever the claim appears, say in the same breath that the agents are third
  party and talk to their own providers; the app's whole job is launching them,
  so a claim that omits them is the one a reader catches. On the site, where the
  claim is made in full, the GitHub update check appears with it too. In the app
  it does not have to: it is one toggle in Settings, and a starter note is not a
  privacy page. See `docs/webdesign/02-messaging.md`, which governs this and is
  not optional.

## Code conventions

The house style is a pure, heavily unit-tested transition function with the side
effects hoisted to the caller: `looper.rs`, `notifier.rs`, `activity.rs` and
`mcp.rs` are the models to copy. If new logic needs a repo, a daemon or a running
app to test, it is probably in the wrong layer.

- **Name the observed failure in the comment above the fix.** Not "handle the
  edge case" but what actually happened, with the number if there is one: "2,417
  duplicate rows observed", "claude does not exit on resume-failure in a PTY, so
  the daemon's early-exit fallback cannot recover it". The code then reads as a
  changelog of what really broke, and the next person can tell a load-bearing
  workaround from a stale one. `resume_probe.rs` and `mcp.rs::launch_flag` are
  the standard.
- **Verify, do not trust, any operation where a model rewrites a user's file.**
  Back up, transform, verify structurally, then swap atomically. A pass that
  fails verification is a pure no-op. Applies to condensing, migrating or
  reformatting anything the user owns.
- **Push live state into an agent's context; do not tell it to go read a file.**
  "Instructed to read" is not "knows". Anything we inject at dispatch is injected
  with its facts already in it.
- **Keep injected context prompt-cache-stable.** No timestamps, UUIDs or
  counters in the stable prefix. Volatile facts go through a per-turn channel.
- **Default-deny allowlists for anything imported or shared.** If we ever accept
  a shared agent profile, run config or deep link, allowlist exact values. A
  denylist leaks, and it leaks quietly.
- **Never silently escalate an agent's permissions.** Any autonomy feature keeps
  the gate visible and chosen. This is a positioning commitment, not a
  preference.
- Anything Agency writes into a user's worktree must be excluded from git, or it
  turns up in every diff and every PR. See `worktree.rs::ensure_agency_excludes`.
- Agency writes inside the workspace only. Never mutate a user's global config.

## Testing the app

Drive the UI by hand. Never use `osascript`, CGEvent or any other synthetic
desktop input to click through the app.
