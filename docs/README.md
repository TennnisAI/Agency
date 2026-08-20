# Design notes

Agency was built agent-first: nearly every feature started as a written design,
then a task-by-task plan, and the documents stayed in the repo. This directory
is that record. Dates in filenames are when the work was designed, not when it
shipped.

**Start with [`design-handoff/`](design-handoff/)** if you want to see the app:
clickable UI mockups plus the design spec (open `Agency v2.dc.html` in a
browser with `support.js` beside it).

## The record, feature by feature

- [`superpowers/specs/`](superpowers/specs/) — dated design documents: what to
  build and why, per feature. `2026-06-18-agency-design.md` is the founding
  one. (Competitor observations in these are point-in-time readings from the
  date in the filename; re-check sources before repeating them.)
- [`superpowers/plans/`](superpowers/plans/) — the matching task-by-task
  implementation plans, written to be executed by coding agents. All of these
  describe shipped work; where the code and a plan disagree, the code won.

## Assessments and standalone designs

- [`one-stop-assessment.md`](one-stop-assessment.md) and
  [`one-stop-plan.md`](one-stop-plan.md) — the reasoning and master plan behind
  the docs, issues, search, and palette features (phases 1–8, shipped).
- [`pr-review-plan.md`](pr-review-plan.md) — the in-app GitHub PR review
  surface (shipped).
- [`agentic-loops.md`](agentic-loops.md) — the design behind loop runs
  (shipped).
- [`run-teardown.md`](run-teardown.md) — what archiving and deleting an agent
  actually remove, why that is decided from the branch rather than from the
  verb, and the record a finished run leaves behind (shipped, AGE-149).
- [`beta-readiness.md`](beta-readiness.md) — the pre-beta bug hunt and fix
  pass, kept as a historical record.

## Positioning and the site

- [`competitive-landscape.md`](competitive-landscape.md) — dated, point-in-time
  readings of adjacent products, each cross-checked against source rather than
  the vendor's README. None of it goes on the site, and no competitor is named
  anywhere a customer can see.
- [`marketing-strategy.md`](marketing-strategy.md) — the layer between those
  readings and the site: which of our claims a funded competitor could copy,
  which they structurally cannot, and what follows for the copy and the
  channels. Read before changing anything positional in `webdesign/`.
- [`webdesign/`](webdesign/) — the getagency.dev design handoff. Start at its
  `README.md`; `02-messaging.md` governs all user-facing copy and is not
  optional.

## Living documents

- [`windows-compat.md`](windows-compat.md) — every macOS/Unix assumption a
  Windows port would need to touch. Kept current as shell-outs are added.
- [`tracked-issues.md`](tracked-issues.md) — open exploration: should
  `.agency/issues/` be trackable in git? No decision yet; constraints recorded.
