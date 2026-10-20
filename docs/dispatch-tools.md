# Dispatch-side tools: an agent starting runs and waiting on them

AGE-209. Shipped.

The `agency-preview` MCP server used to point one way: it served a dispatched
agent tools about its own run (the preview, the user's open file, its status
line). This adds a group that points the other way. An agent can ask Agency to
start another run, block until that run is done or stuck, and read what it did.

The blocking wait is the point. Without it an orchestrating agent spends its
turns sleeping and re-reading output. Agency can offer more than a bare
multiplexer here because it already has what a child run needs: an issue
tracker to dispatch from, a worktree per run so children cannot collide, and a
merge flow to land what they produce.

## The tools

All three live on the existing `agency-preview` server, after the other groups
so that switching them changes nothing listed before them.

- `run_dispatch(issue | prompt) -> run_id`. Exactly one of an issue key in this
  project's tracker (`AGE-14`) or a self-contained prompt. It goes through the
  same code as the user's Dispatch: `start_issue_run` for an issue (which also
  moves the issue to in progress), `create_run_spec` for a prompt, cut from the
  project's base branch. The answer names the run id, branch and worktree path.
- `run_wait(run_id, timeout_secs?)`. Blocks until the child is done, blocked,
  idle, exited, archived or discarded, or until the timeout (default 300s, at
  most 1800s) passes and it answers "still working". For blocked, a guessed
  "quiet" and an exit, the answer carries the bottom of the child's screen.
- `run_read(run_id, what?)`. `transcript` (the default), `diff` (from the commit
  the branch was cut at to the worktree as it stands, uncommitted and untracked
  files included) or `screen`. Cut to 24,000 characters; the answer names the
  worktree, where the rest is.

## What `run_wait` trusts

It reads the same state the board shows (`AppState::read_activity`), polled
once a second, and the verdict is `dispatch::verdict`:

| Board says                   | `run_wait` answers |
| ---------------------------- | ------------------ |
| not observed yet             | keeps waiting      |
| working                      | keeps waiting      |
| blocked (reported)           | blocked            |
| done, reported by hooks      | done               |
| done, guessed from the pane  | quiet, flagged as a guess, with the screen |
| idle                         | idle               |
| session exited / gone        | exited / gone      |

A reported `done` comes from the agent's own lifecycle hooks (AGE-206), which
is what makes the wait trustworthy. An agent without hooks gets the pane's
guess, and the answer says it is one: "quiet" may be a finished turn or a
prompt, and the screen is attached so the parent can tell which.

"Not observed yet" is never idle. A run the notifier has not looked at is
every run in the first two seconds after a dispatch.

## Constraints, and how each is held

**Off by default, per project, held by the user.** `[dispatch] agent_tools`,
read only from `.agency/agency.local.toml`. `config::strip_machine_only` drops
`[dispatch]` from the tracked `agency.toml` before the merge, so a cloned repo
cannot switch it on for whoever clones it. The switch is in Settings, under
Project, "Agent dispatch". Switching it off refuses the next call at once:
`dispatch_child` reads the config afresh rather than through the server's
five-second cache.

**No escalation by proxy.** A child runs its parent's own profile and model,
by name, read from the registry as the user's Dispatch reads them. The issue
sketched an optional `profile` argument; it is deliberately absent. Comparing
two profiles' permissions is not possible with free-form CLI arguments:
dropping `--permission-mode plan` is fewer arguments and a wider gate, so any
rule short of "the same profile" can be got wrong. A user-held allowlist of
profiles a parent may pick would be the safe way to add the choice back.

**The parent cannot answer for the user.** There is no tool to type into a
child. A child stopped on a permission prompt is `blocked` on the board, with
the same badge and notification as any run, and `run_wait` tells the parent it
cannot answer it. A `send` tool would let a parent approve a child's dialog by
typing `1`, so it should not be added without a design that rules that out.

**Never more than the parent had.** A dispatched run is never offered the run
tools itself: the per-request caps check `run_parents`, and the link is in
memory before the child's server starts, so the first `tools/list` the child's
agent caches already leaves them out. Trees are one level deep, and a parent
may have at most four children not yet archived or discarded
(`dispatch::MAX_LIVE_CHILDREN`).

**Only the run's own agent is the caller.** The port says nothing about who
is calling: any local process can reach it. `preview::request_caps` drops the
run tools from any request that fails one of three checks, cheapest first, so
they are neither listed nor answered:

1. **Not a browser.** A script on the previewed page shares the server's
   loopback origin, so it passes the origin check every request gets, and
   could otherwise have started an agent run with the user's credentials.
   Browsers put `Origin` on every POST and `Sec-Fetch-Site` on every fetch; an
   agent's MCP client sends neither. `state_report` refuses browsers the same
   way.
2. **The run's token.** Each run has a random token (`Registry::run_token`,
   kept across restarts because the agent read it at launch), written only
   into that run's emitted MCP config as the `x-agency-run-token` header. This
   stops a process that only found the port, and keeps the expensive check
   below off every request that has no business with the run tools.
3. **The run's own process tree.** A token in a file proves only that the
   caller read the file, and a dispatched child can read its parent's MCP
   config. So the server names the process on the client end of the
   connection (`agency_core::peer::tcp_peer_pid`) and admits it only if it
   descends from the process the terminal daemon spawned for one of the run's
   own sessions (`AppState::is_run_caller`). A process cannot choose its
   parent, and a child's processes descend from the child's session, so a
   copied token gets the child nowhere.

The daemon reports a session's process as an optional `pid` on its `Status`
reply, which needs no protocol bump: a bump kills every live session. A
daemon from before this release omits it, and the check then fails closed.
Such a daemon survives an app update for as long as it hosts agents, so the
run tools stay off until it is replaced, when it is next idle or Agency quits.
The log says so once per launch.

What none of this can stop is a process of the user's that sets out to act
through Agency's own machinery: one that types into the parent's terminal
through the daemon's socket, or edits Agency's database or
`agency.local.toml`. Any process the user runs can do that to any run today,
with or without these tools, so it is not a hole this feature opens. Closing it
is a sandbox's job, the agent CLI's own, not a check on a loopback port.

**Default-deny on reach.** `run_wait` and `run_read` reach only the runs the
asker dispatched (`dispatch::may_reach`). Every other run is another agent's
work and was another decision of the user's.

## Where the pieces are

- `agency-core/src/preview/runs.rs`: the protocol. Parsing, bounds, clipping.
- `agency-core/src/preview/rpc.rs`: the tool schemas and the switched-off text.
- `agency-core/src/config.rs`: `DispatchConfig`, `save_dispatch`.
- `agency-core/src/registry.rs`: the `run_parents` table.
- `agency-app/src/dispatch.rs`: the decisions. Admission, reach, the wait
  verdict, the answers' text. Pure and unit-tested.
- `agency-app/src/state.rs`: `run_tool` and its helpers, the side effects.
- `agency-app/src/lib.rs`: installs the executor, since a server's connection
  thread holds no `AppState` and reaches it through the app handle.

## Not done

- The skills kit (`skills.rs`) does not mention the run tools. The MCP server's
  instructions and the tools' own descriptions do, and both are in the agent's
  context; the kit is written before a child is known to be one, so it could
  not tell a child it lacks them.
- A run already going when the switch goes on gets the tools only if its agent
  was launched with the `agency-preview` server already in its MCP config.
- Nothing in the UI draws the parent-child link yet. Children are ordinary runs
  on the board, titled from their issue or prompt.
