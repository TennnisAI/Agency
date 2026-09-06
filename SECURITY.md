# Security policy

Agency spawns shells, runs coding-agent CLIs with your credentials in their
environment, and manages git worktrees on your machine. If you find a
vulnerability, please report it privately rather than in a public issue.

## Reporting

Use **GitHub's private vulnerability reporting**: on the repository page, go to
the **Security** tab → **Report a vulnerability**. Reports go only to the
maintainer.

If you would rather not use GitHub, or you cannot, email
**nick@tennnis.no** instead.

Please include the app version (Settings ▸ Diagnostics), what you found, and
how to reproduce it. You'll get an acknowledgement as soon as the report is
read — this is a solo project, so triage is best-effort, but security reports
jump the queue.

## Scope worth knowing about

- Agency's own code makes exactly one network request: the launch-time update
  check against GitHub's public releases API, which can be turned off in
  Settings ▸ Diagnostics. Anything else talking to the network is the agent
  CLI or git you invoked.
- Agency injects no cloud credentials. Agent CLIs manage their own
  authentication; Agency inherits your shell environment when spawning them.
  Agent profiles can carry user-provided environment variables, which are
  stored unencrypted in Agency's local database — treat anything you put
  there as readable by any process running as your user.
