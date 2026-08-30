# Contributing to Agency

Thanks for your interest. A few honest expectations up front:

- **Agency is a solo side project.** Issues and PRs are read and answered on a
  best-effort basis, usually within a week, sometimes slower.
- **macOS on Apple Silicon is the only supported platform** right now.
  `docs/windows-compat.md` catalogs what a port would touch, but there is no
  Windows or Linux build to test against.
- **Bug reports are the most valuable contribution.** The bug template asks for
  your version and a log excerpt (both in Settings ▸ Diagnostics) — with those,
  most reports are actionable; without them, most aren't.

## Pull requests

- **Small fixes:** welcome — typos, obvious bugs, doc corrections. Just open
  the PR.
- **Anything substantial:** open an issue first and say what you're planning.
  Agency has strong opinions (terminal-first, no telemetry, no server) and a
  design history under `docs/` — a feature that fights those won't merge, and
  it's better to find that out before writing it.
- **Licensing:** contributions are accepted under the repository's
  [Apache-2.0 licence](LICENSE), and you keep the copyright in what you write.
- **You will be asked to sign a [CLA](CLA.md).** A bot comments on your first
  pull request with a sentence to reply with; that is the whole process, and it
  is once per contributor, not once per PR. It exists so the project can be
  relicensed later if it ever needs to be, which Apache-2.0 alone does not
  allow. Trivial changes are exempt, and if the bot asks when it should not
  have, say so on the PR.

## Development setup

Prerequisites are listed in the [README](README.md#prerequisites). Then:

```sh
pnpm --dir ui install   # frontend dependencies (once)
./dev.sh                # builds the agency-termd daemon, launches tauri dev
```

A dev build keeps its own data directory and daemon socket, so it never
disturbs an installed Agency.app.

## Checks to run before a PR

CI runs these on every PR (`.github/workflows/ci.yml`); running them locally
first saves a round trip:

```sh
cargo fmt --check --all         # formatting (rustfmt.toml is the config)
cargo test -p agency-core       # core library + daemon tests
cargo test -p agency-app        # Tauri shell tests (macOS only)
pnpm --dir ui exec tsc --noEmit # frontend typecheck
pnpm --dir ui test              # frontend unit tests
```

## Code style

- Rust is formatted with `rustfmt` (config in `rustfmt.toml`).
- Comments explain constraints the code can't show — see the existing code for
  the register. Match what's around you.
- User-facing strings say "agent" and "worktree", not "workspace".
