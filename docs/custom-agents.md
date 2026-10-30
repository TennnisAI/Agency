# Custom agents

A custom agent is who the agent is: a name, a description and a set of
instructions. It is separate from an agent profile, which says which CLI to
launch. You write the definition once, in Settings or by hand, and start it in
any harness that can take one.

## The file

One markdown file per agent, named for its id (lower-case letters, digits and
hyphens):

```markdown
---
name: Reviewer
description: "Reviews a diff for correctness"
---
You review diffs. Lead with what is wrong, and say why.
```

- `.agency/agents/<id>.md` is one project's. The directory stays trackable, so
  commit it to share the agent with your team.
- `custom-agents/<id>.md` in the app's data directory is available in every
  project. Both are offered even under one id, and a project's is labelled as
  such in the picker (`Project: Reviewer`), so a `git pull` can't change what picking yours starts.

Only `name`, `description` and the body are read. Anything else in the
frontmatter is dropped, on purpose: a project's agents arrive through `git pull`,
and a native format's `permissionMode`, `tools`, `hooks` or `mcpServers` must not
change what an agent may do because someone picked it from a menu. A model is
left out because its ids belong to one CLI; the spawn has its own picker.

## How each harness is told

`DELIVERIES` in `crates/agency-core/src/custom_agents.rs` is the table, with what
was probed against which version.

| Harness | Delivery |
| --- | --- |
| Claude Code | `.claude/agents/agency-<id>-<tag>.md`, `--agent` |
| Copilot | `.github/agents/agency-<id>-<tag>.agent.md`, `--agent` |
| OpenCode | `.opencode/agents/agency-<id>-<tag>.md`, `--agent` |
| Codex | `-c developer_instructions=...` |
| pi | `--append-system-prompt` |
| Cursor | `.cursor/rules/agency-<id>-<tag>.mdc`, always applied |

`<tag>` is random and set per run. A name a repo could predict is a name a file
it commits could answer to, and `--agent` would launch that file instead. A path
that leaves the worktree through a symlink is refused, and the spawn with it.

Generated files are excluded from git. Any other harness refuses at spawn rather
than starting a plain agent that looks like the one you asked for. So does Cursor
without its own worktree, because its rules file would apply to everything else
running in your checkout.

## What a run keeps

A run stores a copy of the definition when it starts. A resume, a rerun or a
loop's next attempt comes back as the agent the work started under, even if the
file has since been edited or deleted. A new tab of the same harness in that
workspace runs as the same agent; a tab of another harness does not.
