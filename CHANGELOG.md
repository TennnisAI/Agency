# Changelog

Notable changes in each release. The GitHub release for a version carries the
same notes alongside the downloads; this file is so the history is readable without
leaving the repository.

## 0.2.0 (2026-09-11)

Agency runs on Linux. The rest is first-run setup, which now installs what a
fresh machine is missing instead of stopping at it, and a handful of fixes for
workspaces with more than one agent in them.

macOS 11 or later, Apple Silicon. Linux with glibc 2.34 or later, x86_64 or
arm64.

### Linux

- Agency runs on Linux. Each release carries a `.deb` for Debian and Ubuntu, an
  `.rpm` for Fedora, and an AppImage for Arch and everything else, each for
  x86_64 and arm64. They need glibc 2.34 or later: Ubuntu 22.04 and Debian 12
  onwards, and current Fedora and Arch.
- Two things are weaker than on macOS. The tray shows the running-agent count
  in its tooltip rather than beside the icon. And Linux does not tell Agency
  when you click a notification, so the click does not open the run it was
  about.
- The window is one bar high, not three. Linux showed the window manager's
  title bar, then a menu strip, then Agency's own title row. The Linux window
  is now undecorated and Agency's title bar carries the menus and the
  minimize, maximize and close buttons, the way the macOS one carries the
  traffic lights.
- Shortcut hints say Ctrl where the key is Ctrl, not ⌘.

### First run

- Onboarding checks for git, Node.js and npm, and the GitHub CLI, and offers
  to install whichever is missing: through Homebrew or the Command Line
  Tools on macOS, through the distribution's package manager on Linux (the
  desktop asks for your password). Without either it shows the line to copy.
- Installing an agent from onboarding runs the install in the background and
  ticks the agent when it lands, rather than handing over a command to paste
  into a terminal that the first run has no way to open. An agent that needs
  npm on a machine without it installs Node.js first.
- An npm install on a machine whose Node came from a system package no longer
  fails with a permissions error on the root-owned global prefix; the agent
  goes under `~/.local` instead.
- Installing more than one dependency at once no longer makes them fail on each
  other. System package installs share the machine's one package lock, so they
  run in turn instead of colliding with "Could not get lock"; a tool waiting
  its turn shows "Waiting…".
- A first commit on a freshly installed git, which has no name or email set,
  no longer just fails with "Author identity unknown". Agency asks for a name
  and email, saves them for the repository, and makes the commit. The same
  form appears wherever a commit hits that error, including Source Control.

### Projects

- A project added without git can be given a repository later: from its
  right-click menu, from File in the menu bar, and from the command palette.
- "Couldn't start claude: could not run git…" now says "git is not
  installed" when that is what happened, and offers to install it. The same
  for "Initialize repository" in the folder setup dialog, which used to fail
  with a bare `No such file or directory (os error 2)`.
- The DeepSeek Harness tile in onboarding no longer runs its install button
  out past the tile's border.
- Add project and Clone a repository on the welcome screen no longer drop you
  back to the overview the moment the project is added.

### Agents

- A worktree with several agents in its tab strip shows every one of them. The
  focus rail, the sidebar tree and the tiles named a run after its first agent
  alone, so three agents in one worktree read as one. A count beside the run
  expands into its tabs, each opening straight onto that agent.
- The agents side panel in Docs and Files reaches every agent tab in a
  workspace, and shows the one the focus view last had on screen. It used to
  attach the run's first agent only, and stayed on that session after its tab
  was closed.

### Editor

- An open file tab follows an agent's edits while you watch. It used to keep
  whatever it first read until the file was closed and reopened. The change
  lands without moving the lines above it or your cursor, and unsaved edits in
  the tab still win.

### Terminal

- Switching to another agent and back no longer paints cursor-agent's input
  box and status bar over the middle of the transcript, with what you type going
  into the copy that is not live.
- Opening and closing the terminal inside an agent's pane no longer leaves the
  pane parked above the agent's input box until you type.
- A session being resumed no longer draws its placeholder as a staircase while
  it attaches.

## 0.1.1 (2026-09-08)

The first update since launch. Most of it is the merge hand-off, the conflict
view, and agent resuming: the paths where Agency was leaving an agent waiting,
or writing the wrong thing to a file.

macOS 11 or later, Apple Silicon.

### Conflicts

- A conflicted file under Source Control opens a conflict view instead of the
  diff viewer. `git diff` answers for an unmerged path with a combined diff
  whose lines are not the file's lines, so staging a side out of the old view
  could write `<<<<<<< HEAD` and the branch name into your file.
- Hand a conflict to any agent in the worktree, or to a new one. Handing it to
  a tab other than the run's lead used to queue the prompt behind an agent that
  was already sitting idle, for the full five-minute timeout.
- The pane no longer keeps one file's text on screen while the next file loads,
  so "Keep current" cannot write the previous file's contents to the path you
  selected.
- It re-reads from disk before applying. Resolving a conflict with an agent
  while the pane is open on it no longer puts the conflicted version back.
- Conflicts parse in a CRLF checkout. Every conflict in such a repo used to
  land in "markers Agency can't read", unresolvable in the pane.
- A line of seven pipes inside a side is refused rather than silently dropping
  the rest of that side from the file it writes.
- Add and delete conflicts each explain themselves and offer a way out, instead
  of quietly meaning "keep this side".

### Merging into main

- Agency offers to delete the remote branch when a merge lands, the way the PR
  merge dialog already did. Every locally merged agent branch that had been
  pushed used to stay on the remote for good, one per run.
- The box unticks itself when the branch has an open PR, and says why: GitHub
  closes a PR whose head branch is deleted rather than marking it merged.
- The merge waits for that PR check, so pressing Enter can no longer beat it.
- A branch published to more than one remote has every copy deleted, and all of
  them are checked before any is deleted.
- Deleting a branch cannot hang on an SSH key passphrase prompt.
- The teardown buttons are disabled while a remote delete is in flight.

### Committing

- Pressing Commit with nothing staged showed a git error with nothing after the
  colon. It now says what is wrong and offers Stage All or Stage All & Commit,
  with a "Don't ask again" that skips the prompt from then on. A clean tree and
  an unresolved conflict explain themselves in the panel.

### Agents

- Cursor resumes its conversation when the profile points at `agent` rather
  than `cursor-agent`. Both names are the same binary, and `--help` calls
  itself `agent`, so it is the natural profile to write by hand. Runs on such a
  profile came back on a brand new chat every time.
- A half-typed draft at cursor-agent's prompt no longer holds a queued message
  for five minutes and then types over it.
- The check that decides whether a prompt line is empty reads a narrower window
  and refuses more, so a dim line of an agent's own output cannot be mistaken
  for an empty prompt.
- When a run opens a tab that will ask its own first-run question, the note says
  so rather than telling you to close the window and watch. Cursor and Copilot
  both gate on trusting the folder before they read what they were launched
  with.
- Copy a queued prompt out of its popover. The row clamps to three lines, which
  is enough to recognise a merge-conflict prompt and not enough to read one.

### Terminal

- Typing into crush's command palette no longer tears the pane apart. The pane
  returned the carriage on a bare line feed, so every erase after the first
  landed at column 0 instead of inside the dialog.
- Pane captures no longer carry cell styles to the callers that do not read
  them, roughly halving the payload of the app's busiest IPC.

### Projects

- A project folder that is moved, renamed, deleted or ejected is detected and
  marked in the sidebar. You get one screen saying what happened and where it
  was, instead of every tab failing on its own timer in its own words.
  Reconnecting repoints the project at the folder's new home and keeps its
  agents, issues and notes, and mends the worktrees underneath.
- The workspace opens on Docs rather than on an agents grid that is empty for
  most of its life.
- Pinned runs show the pin in the projects tree and the agents rail, not only
  on the tile, and the pin now sits in the same place on every tile.

### Editor

- Drag a file or a note out of the sidebar onto an agent to type its path at
  that prompt, the way a drop from Finder already did.
- An agent can ask which file you have open, so "fix this file" resolves. It is
  behind a switch in Settings, Editor, off until you turn it on, and nothing
  about the open file is recorded while it is down.
- A deep path in the editor toolbar truncates instead of pushing Revert and
  Save out of the pane.

### Site

- getagency.dev links the repository from the header and footer.
- The requirements no longer list Xcode. Agency needs git, which macOS offers
  to install if it is missing.

## 0.1.0 (2026-09-06)

First public release.
