import { invoke, Channel } from "@tauri-apps/api/core";
import { b64ToBytes } from "./b64";

// NOTE: Project comes from agency_core (snake_case serde field names).
export interface Project {
  id: string;
  name: string;
  repo_path: string;
  default_agent: string | null;
  default_provider: string | null;
  color: string | null;
  // 3-letter issue key ("AGE") the project's issues are numbered under.
  issue_key: string | null;
  // null = normal repo project; "workspace" = the pinned notes/journal home.
  kind: string | null;
}

export type SessionStatus =
  | { state: "running" }
  | { state: "exited"; code: number }
  | { state: "gone" };

// Loop recipe/progress (camelCase serde, see agency_core::loops). A run with
// loopConfig set is a loop: its agent is re-invoked headless until the check
// command exits 0 or the attempt cap is spent.
export interface LoopConfig {
  checkCommand: string;
  maxAttempts: number;
  checkTimeoutSecs: number;
  // Null = off, and off is the default: a cap the user did not set never trips.
  maxWallSecs: number | null;
  maxTokens: number | null;
}

export type LoopStatus = "awaitingAgent" | "checking" | "complete" | "stalled" | "stopped";

// Why a stalled loop stalled, so the UI can say which cap to raise or what to
// fix. Null on loops stalled before reasons existed, and on driver-side stalls
// (attempt spawn failure).
export type StallReason = "attemptCap" | "crashLoop" | "wallClock" | "budget";

export interface LoopState {
  status: LoopStatus;
  attempt: number;
  consecutiveFailures: number;
  lastCheckExit: number | null;
  // Epoch seconds the loop started; the time cap measures from here.
  startedAt: number;
  // Last token total observed for the run; stays 0 for agents whose
  // transcript Agency cannot read (their token cap never trips).
  tokensUsed: number;
  stallReason: StallReason | null;
  updatedAt: number;
}

// Live activity signal derived from pane output (camelCase serde, see
// agency_app::activity). "working" = output recently; "waiting" = quiet after
// a user-driven turn (finished, or blocked on input); "idle" = quiet with no
// turn in flight (never prompted, or waited too long). A waiting run decays to
// idle after ~30m so an urgent badge does not climb forever. `since` is epoch
// ms when the current state began. Null until the backend's first 2s poll
// observes the run.
export interface RunActivity {
  state: "working" | "waiting" | "idle";
  since: number;
}

// Tokens and cost for a run, read from the agent's own transcript (see
// agency_core::usage). `cents` is null when no record carried a price we know,
// and `costComplete` is false when only some did, in which case the figure is
// a floor rather than a total and must not be labelled as one.
export interface RunUsage {
  inputTokens: number;
  outputTokens: number;
  cacheWriteTokens: number;
  cacheReadTokens: number;
  totalTokens: number;
  cents: number | null;
  costComplete: boolean;
}

export interface RunInfo {
  id: string;
  projectId: string;
  agent: string;
  prompt: string;
  title: string | null;
  branch: string;
  status: SessionStatus;
  activity: RunActivity | null;
  // Ascending order among the project's pinned runs; null = unpinned. Board
  // order only: it never changes how a run classifies above. Never null-as-in-
  // unknown the way `activity` is, because it is a stored value, not a sample.
  pinRank: number | null;
  // Null means this agent's spend is not visible to us at all, which is true
  // of every agent whose transcript format we have not read. Not the same as
  // zero, and the UI must never render it as one.
  usage: RunUsage | null;
  added: number;
  deleted: number;
  files: number;
  port: number | null;
  // Where this workspace's browser GUI is served on 127.0.0.1 (agents whose
  // interactive surface is a web app, e.g. dsh). Null when no session here
  // serves one, and while a loop is driving the run.
  guiPort: number | null;
  // Which session serves guiPort: the run's own id when its primary agent is
  // the web-served one, otherwise an extra tab's id. The focus view keys the
  // GUI pane off this, so a web agent opened as a tab gets its GUI too.
  guiSessionId: string | null;
  // Something answers on guiPort right now, so the GUI pane can load it
  // instead of a connection error. Only ever true while the session that owns
  // the port is running.
  guiLive: boolean;
  kind: "agent" | "terminal";
  // True while one of the project's run scripts is still running in this run's
  // workspace — the dot on the tile and on the Run tab.
  runScriptsLive: boolean;
  // False = the run works in the project's main checkout on its current branch
  // instead of an isolated worktree, so it has no branch of its own to merge or
  // open a PR from, and discarding it leaves the checkout untouched.
  worktree: boolean;
  // Epoch seconds; archivedAt is null for live runs.
  createdAt: number;
  archivedAt: number | null;
  raceId: string | null;
  loopConfig: LoopConfig | null;
  loopState: LoopState | null;
  // Local issue this run was dispatched from (see startIssueRun).
  issueId: string | null;
  // Model this run's agent was launched on; null = the agent's own default.
  model: string | null;
  // Messages Agency is holding for this run and has not typed into its session
  // yet. Non-zero puts the marker on the tile and the run header: a message
  // waiting behind a long turn should read as patience, not as a lost click.
  queuedMessages: number;
  // The run's own agent tab has been closed, so the tab strip stops drawing it
  // and `status` above describes whichever extra tab is standing in for it.
  // Rerunning the agent brings it back.
  primaryClosed: boolean;
  // What is left of an archived run. Only set by listArchivedRuns — answering
  // it asks git a question per run, and the live board polls every 1.5s.
  archived: ArchivedInfo | null;
}

// What an archived run still holds. The flags are the difference between "put
// away, bring it back whenever" and "finished, here is what happened".
export interface ArchivedInfo {
  // Its branch is still in the repo, so it can be restored onto it. False for
  // the normal ending of merged work, whose branch went with the archive.
  branchKept: boolean;
  // With branchKept false, the branch a restore cuts this run's branch afresh
  // from — the base its work went into, which is where a merged run's commits
  // are. Null only when that branch has gone too, the one case with nothing
  // left to restore from.
  restoreBase: string | null;
  // There is a record to read.
  hasRecord: boolean;
  // There is a conversation to read: a transcript in a format Agency parses,
  // rescued into the archive or still in the agent's own store. Runs archived
  // before records existed can have this and no record.
  hasConversation: boolean;
}

// Where a run's commits live besides its own branch, as git saw it when the
// teardown was offered. Everything is answered locally: a dialog that waits on
// the network is a dialog that hangs.
export interface BranchFacts {
  ownsBranch: boolean;
  commitsAhead: number;
  // False when `base..branch` would not resolve, in which case commitsAhead is
  // meaningless and nothing may be treated as safe.
  commitsKnown: boolean;
  merged: boolean;
  pushed: boolean;
  // The branch has been deleted or renamed outside Agency.
  gone: boolean;
  // Uncommitted changes in the worktree: on no branch and no remote, so an
  // archive commits them and a delete destroys them.
  dirty: boolean;
  // The base is in the repo, so a restore has something to cut the run's branch
  // afresh from once the archive has let that branch go.
  baseExists: boolean;
}

// Why deleting a branch loses nothing.
export type SafeBecause = "merged" | "pushed" | "empty";

// What one teardown verb would remove from one run. Decided in the backend so
// the words here and the git commands there cannot disagree.
export interface CleanupPlan {
  removesWorktree: boolean;
  deletesBranch: boolean;
  keepsBranch: boolean;
  keepsRecord: boolean;
  restorable: boolean;
  // Commits that exist nowhere but the branch about to be deleted. Zero for
  // every merged run — which is why a post-merge delete is not a red button.
  commitsAtRisk: number;
  // Uncommitted work this teardown destroys. Only a delete ever does.
  losesUncommitted: boolean;
  safeBecause: SafeBecause | null;
}

export interface RunCleanup {
  facts: BranchFacts;
  archive: CleanupPlan;
  delete: CleanupPlan;
  branch: string;
  base: string;
  // Agency knows where this agent keeps its conversation for this worktree, so
  // archiving rescues it into the archive and deleting removes it. False for an
  // agent whose transcript format Agency cannot read (most of them) and for a
  // run in the project's own checkout: in both, the agent's own session history
  // is untouched by either verb, and can still be resumed from the agent.
  managesTranscript: boolean;
}

// One message the send queue is holding for a run.
export interface QueuedMessage {
  // The session it will be typed into: the run's id, or `<run>--<n>` for an
  // extra agent tab.
  sessionId: string;
  // The feature that composed it: "review comments", "check feedback",
  // "merge conflict".
  origin: string;
  // The whole text, and the handle cancelQueuedMessage matches on.
  text: string;
}

// What became of a message the queue was holding, pushed from the notifier
// tick as a `send-queue-notice` event (see App.tsx).
//
// Both of these land long after whoever sent the text closed the window they
// sent it from, which is why they are pushed rather than polled: "dropped" is a
// message nothing was ever typed of, and "appended" is one that waited out its
// five minutes and went in after whatever was on the prompt line.
export interface QueueNotice {
  runId: string;
  kind: "dropped" | "appended";
  // The whole sentence, composed in the backend so it can be tested there.
  text: string;
}

// ── issues (the local per-project tracker) ──────────────────────────────────

export type IssueStatus = "backlog" | "todo" | "in_progress" | "in_review" | "done" | "cancelled";

export interface Issue {
  id: string;
  projectId: string;
  // Per-project number, displayed with the project's issue key: AGE-14.
  seq: number;
  title: string;
  body: string;
  status: IssueStatus;
  // 0 none · 1 low · 2 medium · 3 high · 4 urgent.
  priority: number;
  // Civil dates, "YYYY-MM-DD" — lexicographic order is date order.
  due: string | null;
  scheduled: string | null;
  // Manual board order within a status group, ascending.
  rank: number | null;
  // Keys of the issues this one is linked to ("AGE-12"), any project's. Links
  // are undirected — the app writes both sides — and a key with no issue
  // behind it is kept (see lib/issueLinks).
  links: string[];
  // The discussion, oldest first. Stored in the issue file below the body, so
  // an agent dispatched on the issue is handed the thread with it.
  comments: IssueComment[];
  createdAt: number;
  updatedAt: number;
}

export interface IssueComment {
  // The repo's git user for comments written here; whatever an agent signs its
  // own with otherwise.
  author: string;
  // Epoch seconds, and the comment's id within its issue.
  createdAt: number;
  body: string;
}

// Partial update: omitted fields keep their values. For the nullable fields
// an explicit null clears the value (absent still means untouched).
export interface IssuePatch {
  title?: string;
  body?: string;
  status?: IssueStatus;
  priority?: number;
  due?: string | null;
  scheduled?: string | null;
  rank?: number | null;
  // The whole link set, replaced wholesale: add and remove are the same call.
  links?: string[];
}

export const listProjects = () => invoke<Project[]>("list_projects");

// `color` must be a palette accent name (see PROJECT_COLOR_NAMES); the backend
// rejects anything else.
export const setProjectColor = (id: string, color: string) =>
  invoke<void>("set_project_color", { id, color });

export const listIssues = (projectId: string) => invoke<Issue[]>("list_issues", { projectId });
export const createIssue = (projectId: string, title: string, body: string, status: IssueStatus) =>
  invoke<Issue>("create_issue", { projectId, title, body, status });
export const updateIssue = (id: string, patch: IssuePatch) =>
  invoke<Issue>("update_issue", { id, patch });
export const deleteIssue = (id: string) => invoke<void>("delete_issue", { id });

// Backlog sharing. `sync` is read from the tracked agency.toml merged under the
// local one, but only ever *written* to the local one: writing the tracked file
// would leave the checkout dirty, which is what untracking the issues avoided.
// `fromRepo` says the tracked file asked for it, so the UI can explain that
// switching off here is a local override rather than a change everyone sees.
export interface IssueSyncConfig {
  sync: boolean;
  /** Sync on the board's own schedule as well as from the Sync button. */
  auto: boolean;
  remote: string;
  remotes: string[];
  fromRepo: boolean;
  /** This project's issue key prefix (`AGE` in `AGE-14`). */
  issueKey: string;
  /**
   * Names of other projects already on this key. Allowed on purpose — two
   * checkouts of one repo sharing one backlog need the same key — but a
   * `[[AGE-14]]` wikilink can then only resolve to one of them.
   */
  keySharedWith: string[];
}
// Whole-file last-writer-wins is not what happens; the merge is field by field,
// and a conflict is one field of one issue that both sides changed.
export interface IssueSyncConflict {
  key: string;
  field: string;
  detail: string;
}
export interface IssueSyncOutcome {
  written: number;
  deleted: number;
  assetsFetched: number;
  assetsDeleted: number;
  conflicts: IssueSyncConflict[];
  skipped: [string, string][];
  committed: boolean;
  pushed: boolean;
}
// "merge" in the steady state. Two already-populated machines syncing for the
// first time share no history, so merge fails there and the caller re-runs with
// "publish" (this machine seeds the shared tracker) or "adopt" (the reverse).
export type IssueSyncMode = "merge" | "publish" | "adopt";

export const getIssueSyncConfig = (projectId: string) =>
  invoke<IssueSyncConfig>("get_issue_sync_config", { projectId });
export const saveIssueSyncConfig = (
  projectId: string,
  sync: boolean,
  auto: boolean,
  remote: string,
) => invoke<void>("save_issue_sync_config", { projectId, sync, auto, remote });
/** One file's move under a key rename: `DEM-7` to `AGE-7`, or to `AGE-9`
 * when `AGE-7` was already someone else's. */
export interface KeyMove {
  from: string;
  to: string;
}
/**
 * What a key rename does to the issue files. `renumbered` is the subset that
 * also took a new number because the old one was taken under the new key; an
 * issue's identity is its uid, and every link to it is rewritten in the same
 * pass, so nothing is lost, but the user should hear the new numbers.
 */
export interface RekeyPlan {
  moves: KeyMove[];
  renumbered: KeyMove[];
}
/** What `setProjectIssueKey` would do, for the dialog that asks first. */
export interface IssueKeyPreview {
  /** The key as it would be stored: trimmed and upper-cased. */
  key: string;
  current: string;
  plan: RekeyPlan;
  /** Other projects already on `key`. Allowed, and said. */
  sharedWith: string[];
}
export const previewIssueKey = (projectId: string, key: string) =>
  invoke<IssueKeyPreview>("preview_issue_key", { projectId, key });
/** Renames the project's issue files onto `key`; resolves with what moved where. */
export const setProjectIssueKey = (projectId: string, key: string) =>
  invoke<RekeyPlan>("set_project_issue_key", { projectId, key });
// "Needs seeding" is an outcome, not an error: two already-populated machines
// with no history in common is a question for the user (which side seeds the
// other), and the UI has to tell it apart from a broken remote without reading
// an error string.
export type IssueSyncResult =
  | { kind: "done"; outcome: IssueSyncOutcome; keyMismatch: KeyMismatch | null }
  | { kind: "needsSeeding"; local: number; remote: number };

/**
 * The pass merged issue files whose key prefix this project does not use, so
 * they are on disk and off the board. `adopted` means the project had no
 * issues of its own and has been moved onto the shared tracker's key; false
 * means both keys hold real issues and only the user can say which this
 * project is.
 */
export interface KeyMismatch {
  ours: string;
  theirs: string;
  count: number;
  adopted: boolean;
}

// Streamed, like the git commands: a pass fetches, reads one blob per issue per
// tree, then pushes, so it is seconds on a real backlog and the board shows a
// phase readout rather than nothing.
export function syncIssues(
  projectId: string,
  mode: IssueSyncMode,
  onProgress?: (p: CloneProgress) => void,
): Promise<IssueSyncResult> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<IssueSyncResult>("sync_issues", { projectId, mode, onProgress: onProgressChannel });
}

// Comments are their own calls rather than a field of the patch: the thread
// lives in the issue file and an agent may be appending to it at the same
// time, so each call re-reads the file and applies only its own change.
// `createdAt` addresses a comment within its issue. All three return the issue
// as it now stands.
export const addIssueComment = (issueId: string, body: string) =>
  invoke<Issue>("add_issue_comment", { issueId, body });
export const updateIssueComment = (issueId: string, createdAt: number, body: string) =>
  invoke<Issue>("update_issue_comment", { issueId, createdAt, body });
export const deleteIssueComment = (issueId: string, createdAt: number) =>
  invoke<Issue>("delete_issue_comment", { issueId, createdAt });
export const startIssueRun = (issueId: string, agent: string, model?: string | null, base?: string | null, mergeTarget?: string | null) =>
  invoke<RunInfo>("start_issue_run", { issueId, agent, model: model ?? null, base: base ?? null, mergeTarget: mergeTarget ?? null });
export const startIssueRace = (issueId: string, attempts: RaceAttempt[], base?: string | null, mergeTarget?: string | null) =>
  invoke<RunInfo[]>("start_issue_race", { issueId, attempts, base: base ?? null, mergeTarget: mergeTarget ?? null });
export const startIssueLoop = (
  issueId: string,
  agent: string,
  model: string | null,
  checkCommand: string,
  maxAttempts: number,
  maxWallSecs: number | null,
  maxTokens: number | null,
  base?: string | null,
  mergeTarget?: string | null,
) =>
  invoke<RunInfo>("start_issue_loop", {
    issueId, agent, model: model ?? null, checkCommand, maxAttempts, maxWallSecs, maxTokens,
    base: base ?? null, mergeTarget: mergeTarget ?? null,
  });

export const addProject = (name: string, repoPath: string) =>
  invoke<Project>("add_project", { name, repoPath });

// ── workspace (the pinned notes/journal project) ────────────────────────────

export const getWorkspace = () => invoke<Project | null>("get_workspace");
export const defaultWorkspaceLocation = () => invoke<string>("default_workspace_location");
// Creates (or adopts) the workspace folder. With `useGit` it is initialized and
// given an initial commit so agents can run in it; without, it's just a folder.
export const createWorkspace = (path: string, useGit: boolean) =>
  invoke<Project>("create_workspace", { path, useGit });
// Moves the workspace folder on disk and repoints the project at it.
export const moveWorkspace = (newPath: string) =>
  invoke<Project>("move_workspace", { newPath });
// Re-seed the workspace's Welcome guide if deleted; returns its note path.
export const ensureWorkspaceGuide = (projectId: string) =>
  invoke<string>("ensure_workspace_guide", { projectId });

export type RepoReadiness = {
  state: "notARepo" | "noCommits" | "ready";
  stageable: boolean;
  dirty: boolean;
};

export const inspectRepo = (repoPath: string) =>
  invoke<RepoReadiness>("inspect_repo", { repoPath });
export const initRepo = (repoPath: string) =>
  invoke<void>("init_repo", { repoPath });
// A progress update streamed from `git clone --progress` during a clone.
export type CloneProgress = { phase: string; percent: number | null; detail: string };

// Clones `url` into a new folder under `parentDir`; resolves to the clone's path.
// `onProgress`, if given, is called as git reports download progress.
export function cloneRepo(
  url: string,
  parentDir: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<string> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<string>("clone_repo", { url, parentDir, onProgress: onProgressChannel });
}
// Stages everything in `repoPath` and makes the initial commit. `onProgress`, if
// given, is called as files are staged — a folder with a big tree takes a while.
// `ignorePaths` are added to .gitignore instead of being committed (the large
// files the user opted out of); folders end with a "/".
export function commitRepo(
  repoPath: string,
  addGitignore: boolean,
  ignorePaths: string[],
  onProgress?: (p: CloneProgress) => void,
): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("commit_repo", {
    repoPath,
    addGitignore,
    ignorePaths,
    onProgress: onProgressChannel,
  });
}
// Stops an in-flight cloneRepo for this url/destination pair by killing the git
// it is waiting on, and deletes the half-downloaded folder. Pass the same two
// arguments the clone was started with: the backend derives the folder name, so
// the frontend never has to match it. cloneRepo then rejects with "cancelled".
export const cancelClone = (url: string, parentDir: string) =>
  invoke<void>("cancel_clone", { url, parentDir });

// Stops an in-flight commitRepo for this folder by killing the git it is
// waiting on. A no-op when nothing is running there.
// commitRepo then rejects with "cancelled", which is the user's own doing and
// not a failure to report.
export const cancelRepoSetup = (repoPath: string) =>
  invoke<void>("cancel_repo_setup", { repoPath });

// What a backend job rejects with when the user stopped it: their own doing, so
// callers clear it rather than showing it as a failure. Matched exactly, since
// git's own output can mention the word.
export const CANCELLED = "cancelled";

// One file large enough to be worth warning about before it's committed.
export type LargeFile = { path: string; bytes: number };
// What a folder holds that would make its first commit slow: `files` is the
// biggest few (largest first), `count`/`bytes` cover them all, and
// `ignorePaths` is the shortest set of .gitignore entries that excludes them.
// `truncated` means the walk hit its budget, so `count` is a floor and
// `ignorePaths` may not reach every large file.
export type LargeFileScan = {
  files: LargeFile[];
  count: number;
  bytes: number;
  truncated: boolean;
  ignorePaths: string[];
  // The size that got a file counted. Comes from the backend so the warning
  // can't quote a threshold the scan didn't use.
  thresholdBytes: number;
};
export const scanLargeFiles = (repoPath: string) =>
  invoke<LargeFileScan>("scan_large_files", { repoPath });

// Both project teardowns stop every agent in the project, and deleting also
// hands git each worktree to unlink — seconds to tens of seconds on a busy
// project, so `onProgress`, if given, is called with the step they are on.
export function closeProject(id: string, onProgress?: (p: CloneProgress) => void): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("close_project", { id, onProgress: onProgressChannel });
}
export function deleteProject(id: string, onProgress?: (p: CloneProgress) => void): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("delete_project", { id, onProgress: onProgressChannel });
}

// Creates an agent workspace (git worktree + first session). `onProgress`, if
// given, is called as the worktree is checked out and essentials copied — a
// large repo takes a while, so the UI shows movement instead of freezing.
// `worktree: false` starts the agent in the project's own checkout, on the
// branch already there; `base`/`mergeTarget` are then ignored.
export function createRun(
  projectId: string,
  prompt: string,
  agent: string,
  model: string | null,
  base: string,
  mergeTarget?: string | null,
  onProgress?: (p: CloneProgress) => void,
  worktree = true,
): Promise<RunInfo> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<RunInfo>("create_run", {
    projectId, prompt, agent, model, base, mergeTarget: mergeTarget ?? null, worktree,
    onProgress: onProgressChannel,
  });
}
export const createLoop = (
  projectId: string,
  prompt: string,
  agent: string,
  model: string | null,
  base: string,
  mergeTarget: string | null,
  checkCommand: string,
  maxAttempts: number,
  maxWallSecs: number | null,
  maxTokens: number | null,
) =>
  invoke<RunInfo>("create_loop", {
    projectId, prompt, agent, model, base, mergeTarget, checkCommand, maxAttempts,
    maxWallSecs, maxTokens,
  });
export const stopLoop = (id: string) => invoke<void>("stop_loop", { id });
export const createTerminal = (projectId: string) =>
  invoke<RunInfo>("create_terminal", { projectId });
export const agentInstalled = (agent: string) =>
  invoke<boolean>("agent_installed", { agent });
// How the binary on PATH was installed, read off its canonicalized path by the
// backend; package/formula names come out of that same path, not a table.
export type InstallMethod =
  | { method: "npm"; package: string }
  | { method: "homebrew"; formula: string }
  | { method: "vendor" }
  | { method: "unknown" };
// One Settings diagnostics row (AGE-146): local facts only, no staleness verdict.
export interface AgentCliInfo {
  agent: string;
  path: string | null;
  version: string | null;
  install: InstallMethod;
}
export const agentCliInfo = () => invoke<AgentCliInfo[]>("agent_cli_info");
export const createInstallTerminal = (projectId: string, agent: string, command: string) =>
  invoke<RunInfo>("create_install_terminal", { projectId, agent, command });
export const confirmQuit = () => invoke<void>("confirm_quit");
// Fills an empty title from the first prompt, and when the branch is still the
// empty-prompt `agent/<id>` fallback, renames it to match (AGE-183).
export const setRunTitle = (id: string, firstPrompt: string) =>
  invoke<void>("set_run_title", { id, firstPrompt });
// Rename a run: overwrites the display title (empty clears it → falls back to prompt/branch).
export const renameRun = (id: string, title: string) =>
  invoke<void>("rename_run", { id, title });

// Renames the run's branch, in git and in the registry together, and resolves
// to the name actually applied: the `agent/` prefix is kept whether or not it
// was typed. Refused for a branch that already exists, one git will not take as
// a ref, one mid-merge, and one already pushed (renaming that here would leave
// the published copy, and any PR from it, behind).
export const renameRunBranch = (id: string, branch: string) =>
  invoke<string>("rename_run_branch", { id, branch });
// Pin a run to the end of its project's pinned runs, or unpin it. Pinned order
// is the order they were pinned in; unpin and pin again to move one to the end.
export const pinRun = (id: string, pinned: boolean) =>
  invoke<void>("pin_run", { id, pinned });
export const listRuns = (projectId: string) => invoke<RunInfo[]>("list_runs", { projectId });
export const runPreview = (id: string, lines: number) =>
  invoke<string>("run_preview", { id, lines });
export const detachRun = (id: string) => invoke<void>("detach_run", { id });
export const runInput = (id: string, data: string) => invoke<void>("run_input", { id, data });
export const resizeRun = (id: string, cols: number, rows: number) =>
  invoke<void>("resize_run", { id, cols, rows });
export const runStatus = (id: string) => invoke<SessionStatus>("run_status", { id });
// Tearing an agent down stops its session and hands git a whole worktree to
// unlink, which on a big repo takes long enough to look like a hang. All three
// report their step through the same progress channel clone and push use, so
// the confirm dialog can say what it is waiting on.
export function discardRun(id: string, onProgress?: (p: CloneProgress) => void): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("discard_run", { id, onProgress: onProgressChannel });
}
export const stopRun = (id: string) => invoke<void>("stop_run", { id });
export const rerun = (id: string) => invoke<RunInfo>("rerun", { id });
export const ensureRunActive = (id: string) => invoke<void>("ensure_run_active", { id });
export function archiveRun(id: string, onProgress?: (p: CloneProgress) => void): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("archive_run", { id, onProgress: onProgressChannel });
}
export const restoreRun = (id: string) => invoke<RunInfo>("restore_run", { id });
/**
 * What archiving or deleting this run would actually remove, asked of git now.
 * Read before either dialog is shown so the wording is about this branch
 * rather than about the verb.
 */
export const runCleanup = (id: string) => invoke<RunCleanup>("run_cleanup", { id });
/** The archived run's record as markdown; null for a run archived before records. */
export const readRunRecord = (id: string) => invoke<string | null>("read_run_record", { id });
/** One rendered line of a run's conversation. Tool turns are one-line markers. */
export type ConversationTurn = { role: "user" | "assistant" | "tool"; text: string };
/** One session file of a run's transcript, oldest first in the list. */
export type ConversationSession = {
  title: string | null;
  started: string | null;
  turns: ConversationTurn[];
};
/**
 * A run's conversation, parsed from its agent's own transcript (the copy the
 * archive rescued, or the live session directory). `supported` false means we
 * cannot read this agent's format at all, which must never be presented as
 * "the agent said nothing".
 */
export type RunConversation = { supported: boolean; sessions: ConversationSession[] };
export const readRunConversation = (id: string) =>
  invoke<RunConversation>("read_run_conversation", { id });
export const listArchivedRuns = (projectId: string) =>
  invoke<RunInfo[]>("list_archived_runs", { projectId });
/** Result of a bulk discard: some runs can fail while the rest still go. */
export type DiscardSummary = { discarded: number; failed: string[] };
export function discardArchivedRuns(
  projectId: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<DiscardSummary> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<DiscardSummary>("discard_archived_runs", { projectId, onProgress: onProgressChannel });
}

export function attachRun(id: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run", { id, cols, rows, onChunk });
}

// One candidate run command detected in the project (package.json script,
// Makefile target, …), offered when adding a script.
export type RunSuggestion = {
  label: string;
  name: string;
  command: string;
  detail: string;
  web: boolean;
};

// One entry in the project's run list.
export type RunScript = {
  name: string;
  command: string;
  // Serves a web app on $AGENCY_PORT, so it gets a URL, the preview pane and
  // "Open in browser". A build or a test run doesn't.
  web: boolean;
  // Starting this one stops every other run script in the project.
  nonconcurrent: boolean;
};

// The project's run scripts as the Run tab sees them. Empty `scripts` is what
// the setup card renders for.
export type RunScriptConfig = {
  scripts: RunScript[];
  shared: boolean;
  suggestions: RunSuggestion[];
  workspace: string;
  port: number | null;
  // Set when this workspace's preview MCP server is listening: the preview
  // iframe loads this port's instrumented proxy instead of the dev server
  // directly, so the dispatched agent can see and drive the pane (AGE-143).
  // Null for the project checkout and for runs without the server.
  previewPort: number | null;
  // `[preview] agent_tools` for this project: whether marking a script "web"
  // gives dispatched agents preview tools at all. Drives the editor's
  // disclosure copy.
  previewToolsEnabled: boolean;
};

export type RunScriptStatus = { name: string; status: SessionStatus };

// Which workspace a run script runs in: an agent's (its run id) or the
// project's own checkout. The same token shape Source Control takes.
export const projectTarget = (projectId: string) => `project:${projectId}`;

export const runScriptConfig = (target: string) =>
  invoke<RunScriptConfig>("run_script_config", { target });
export const saveRunScripts = (target: string, scripts: RunScript[]) =>
  invoke<void>("save_run_scripts", { target, scripts });
export const startRunScript = (target: string, script: string) =>
  invoke<void>("start_run_script", { target, script });
export const stopRunScript = (target: string, script: string) =>
  invoke<void>("stop_run_script", { target, script });
export const runScriptsStatus = (target: string) =>
  invoke<RunScriptStatus[]>("run_scripts_status", { target });
// Just the "something is running here" bit, for the dot on the project's Run
// tab. Agents carry the same flag on their RunInfo (`runScriptsLive`), so the
// board never asks per agent.
export const runScriptsLive = (target: string) =>
  invoke<boolean>("run_scripts_live", { target });

// One run whose preview MCP server is up. `url` is the instrumented preview;
// `active` whether anything serves on the run's app port, i.e. whether a
// hidden preview host is worth mounting (see PreviewKeeper).
export type PreviewTarget = { runId: string; url: string; active: boolean };
export const previewTargets = () => invoke<PreviewTarget[]>("preview_targets");

// Where a run's visible preview pane sits in the window (CSS px), or null when
// it leaves the screen. Feeds the native crop behind the agent's
// preview_screenshot tool.
export type PreviewRect = { x: number; y: number; width: number; height: number };
export const setPreviewRect = (runId: string, rect: PreviewRect | null) =>
  invoke<void>("set_preview_rect", { runId, rect });

// A terminal pane carries a single id, but a run script is addressed by
// workspace *and* script name. These pack the pair into one key for the
// FocusTerminal stream and unpack it again at the IPC boundary. Neither a run
// id nor `project:<id>` can contain "#", so the first one always splits them.
export const runScriptKey = (target: string, script: string) => `${target}#${script}`;
function splitRunScriptKey(key: string): { target: string; script: string } {
  const at = key.indexOf("#");
  return at < 0
    ? { target: key, script: "" }
    : { target: key.slice(0, at), script: key.slice(at + 1) };
}

export const runScriptPreview = (key: string, lines: number) =>
  invoke<string>("run_script_preview", { ...splitRunScriptKey(key), lines });
export const detachRunScript = (key: string) =>
  invoke<void>("detach_run_script", splitRunScriptKey(key));
export const runScriptInput = (key: string, data: string) =>
  invoke<void>("run_script_input", { ...splitRunScriptKey(key), data });
export const resizeRunScript = (key: string, cols: number, rows: number) =>
  invoke<void>("resize_run_script", { ...splitRunScriptKey(key), cols, rows });

export function attachRunScript(key: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run_script", { ...splitRunScriptKey(key), cols, rows, onChunk });
}

// Companion shell: a per-run interactive terminal sharing the run's worktree,
// independent of the agent and run-script sessions.
export const startShell = (id: string) => invoke<void>("start_shell", { id });
export const stopShell = (id: string) => invoke<void>("stop_shell", { id });
export const shellStatus = (id: string) => invoke<SessionStatus>("shell_status", { id });
export const shellPreview = (id: string, lines: number) =>
  invoke<string>("shell_preview", { id, lines });
export const detachShell = (id: string) => invoke<void>("detach_shell", { id });
export const shellInput = (id: string, data: string) =>
  invoke<void>("shell_input", { id, data });
export const resizeShell = (id: string, cols: number, rows: number) =>
  invoke<void>("resize_shell", { id, cols, rows });

export function attachShell(id: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_shell", { id, cols, rows, onChunk });
}

// Extra agent sessions: additional agent tabs sharing a run's worktree. Ids
// are composite (`<runId>--<n>`) and are accepted by the ordinary run-terminal
// calls (attachRun / runInput / resizeRun / runPreview / ensureRunActive).
export interface RunSessionInfo {
  id: string;
  runId: string;
  agent: string;
  status: SessionStatus;
}

export const startRunSession = (runId: string, agent?: string) =>
  invoke<RunSessionInfo>("start_run_session", { runId, agent: agent ?? null });
export const listRunSessions = (runId: string) =>
  invoke<RunSessionInfo[]>("list_run_sessions", { runId });
export const closeRunSession = (id: string) => invoke<void>("close_run_session", { id });
// Put back a run's own agent tab after it was closed, resuming the
// conversation it had.
export const reopenRunAgent = (id: string) => invoke<void>("reopen_run_agent", { id });

export interface FileChange {
  path: string;
  index: string; // staged status code
  worktree: string; // unstaged status code
}

export const gitStatus = (taskId: string) =>
  invoke<FileChange[]>("git_status", { taskId });

export const gitDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<string>("git_diff", { taskId, path, staged });

export const gitStage = (taskId: string, path: string) =>
  invoke<void>("git_stage", { taskId, path });

export const gitUnstage = (taskId: string, path: string) =>
  invoke<void>("git_unstage", { taskId, path });
export const gitStageAll = (taskId: string) => invoke<void>("git_stage_all", { taskId });
export const gitUnstageAll = (taskId: string) => invoke<void>("git_unstage_all", { taskId });
export const gitDiscard = (taskId: string, path: string, untracked: boolean) =>
  invoke<void>("git_discard", { taskId, path, untracked });
export const gitDiscardAll = (taskId: string) => invoke<void>("git_discard_all", { taskId });

export const gitCommit = (taskId: string, message: string) =>
  invoke<void>("git_commit", { taskId, message });

// Pushes the current branch to origin, streaming git's progress to `onProgress`
// (same shape as clone) so the UI can show a determinate bar for large pushes.
export function gitPush(
  taskId: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("git_push", { taskId, onProgress: onProgressChannel });
}
// Stops an in-flight gitPush (or the push half of a gitSync) for this run by
// killing the git it is waiting on; that call then rejects with CANCELLED. A
// no-op when nothing is pushing, so it is safe to fire from a Cancel that is
// also live during steps with nothing to kill. Nothing is cleaned up: a killed
// push leaves origin unchanged and the local branch is the user's own.
export const cancelPush = (taskId: string) => invoke<void>("cancel_push", { taskId });

export const gitFetch = (taskId: string) => invoke<void>("git_fetch", { taskId });
// Fetch this repo's origin *if* it hasn't been fetched recently — the backend
// throttles and backs off per project, so callers fire it at moments the user
// is about to read ahead/behind (panel open, window focus) without thinking
// about cadence. Resolves to whether a fetch actually ran; never rejects for a
// remote that is down, since nobody asked for this one.
export const gitAutoFetch = (taskId: string) => invoke<boolean>("git_auto_fetch", { taskId });
export const gitPull = (taskId: string) => invoke<void>("git_pull", { taskId });

// VS Code-style "Sync Changes": fetch, fast-forward in incoming commits, then
// push local ones (streaming push progress, same shape as `gitPush`). Resolves
// to "diverged" when the branch can't fast-forward — nothing was changed and
// the caller should offer to rebase.
export type SyncOutcome = "synced" | "diverged";
export function gitSync(
  taskId: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<SyncOutcome> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<SyncOutcome>("git_sync", { taskId, onProgress: onProgressChannel });
}

export const gitSetRemote = (taskId: string, url: string) =>
  invoke<void>("git_set_remote", { taskId, url });

export interface RunBranches {
  branch: string;
  base: string;
}
export const runBranches = (taskId: string) =>
  invoke<RunBranches>("run_branches", { taskId });

// NOTE: field names mirror agency_core's AgentProfile (snake_case serde).
export interface AgentProfile {
  name: string;
  command: string;
  args: string[];
  env: [string, string][];
  resume_args: string[] | null;
  loop_args: string[] | null;
}

export interface ProviderSettings {
  lmStudioBaseUrl: string;
  // Agent id "New Agent" spawns; null = auto (project's last-used agent).
  defaultAgent: string | null;
  // Whether the add-agent menu starts with "Own worktree" ticked. A default
  // only: the menu still offers both on every spawn.
  defaultWorktree: boolean;
}

export type McpTransport = "stdio" | "http" | "sse";

// One MCP server definition: either stdio (command/args/env) or remote (url).
// `transport` is inferred when null; `headers` carry auth for remote servers.
// `userScopeAgents` lists the agent CLIs the server is registered with directly
// (Authenticate flow) — Agency stops emitting it into those agents' per-worktree
// config, since their own user config already holds the signed-in session, and
// keeps emitting it for every other agent.
export interface McpServer {
  name: string;
  command: string | null;
  args: string[];
  env: Record<string, string>;
  url: string | null;
  transport: McpTransport | null;
  headers: Record<string, string>;
  userScopeAgents: string[];
}

// Result of importing an mcp.json: the full list after merge, plus how many
// servers the file contributed (entries with neither command nor url are skipped).
export interface McpImportResult {
  servers: McpServer[];
  imported: number;
}

export const listMcpServers = () => invoke<McpServer[]>("list_mcp_servers");
export const saveMcpServers = (servers: McpServer[]) =>
  invoke<void>("save_mcp_servers", { servers });
export const importMcpJson = (text: string) =>
  invoke<McpImportResult>("import_mcp_json", { text });
export const authenticateMcpServer = (projectId: string, agent: string, name: string) =>
  invoke<RunInfo>("authenticate_mcp_server", { projectId, agent, name });
export const deauthenticateMcpServer = (agent: string, name: string) =>
  invoke<void>("deauthenticate_mcp_server", { agent, name });

// Per-project graphify knowledge-graph config (persisted to the project's
// gitignored .agency/agency.local.toml). *_command are null when unset, in
// which case *_default is what actually runs; *_installed reports PATH presence.
export interface KnowledgeConfig {
  graph: boolean;
  // Rebuild the graph after every clean merge. The one thing here that spends
  // a model budget without the user pressing anything, so it is a switch.
  rebuild_on_merge: boolean;
  serve_command: string | null;
  build_command: string | null;
  serve_default: string;
  build_default: string;
  serve_installed: boolean;
  build_installed: boolean;
  // What the build can run on, this machine, best first. `note` is the line
  // about cost and destination shown before anything runs; `default_model` is
  // the model placeholder, empty where the user has to name one.
  backends: { id: string; label: string; note: string; default_model: string }[];
  // Which backend the effective build command names ("custom" for a
  // hand-written one), and the model it names.
  build_backend: string;
  build_model: string;
  // The graph file the serve command reads, and whether it exists yet. No
  // graph means no MCP server is handed to agents.
  graph_path: string;
  graph_built: boolean;
  // A build is running; poll this config until it clears.
  building: boolean;
  // How long the running build has been going, and how long the last finished
  // one took. A graph build takes minutes, so the elapsed time is what tells a
  // working build apart from a stuck panel.
  build_elapsed_secs: number | null;
  last_build_secs: number | null;
  // The tail of the running (or last) build's output, oldest first.
  build_log: string[];
  last_build_error: string | null;
  // The last build ended because the user stopped it, which is not a failure.
  last_build_stopped: boolean;
  install_command: string;
}

export const getKnowledgeConfig = (projectId: string) =>
  invoke<KnowledgeConfig>("get_knowledge_config", { projectId });
// Pick which model the build runs on. Writes the build command; nothing runs
// until Build graph is pressed.
export const setKnowledgeBackend = (projectId: string, backend: string, model: string) =>
  invoke<void>("set_knowledge_backend", { projectId, backend, model });
export const saveKnowledgeConfig = (
  projectId: string,
  graph: boolean,
  rebuildOnMerge: boolean,
  serveCommand: string | null,
  buildCommand: string | null,
) =>
  invoke<void>("save_knowledge_config", {
    projectId,
    graph,
    rebuildOnMerge,
    serveCommand,
    buildCommand,
  });
export const buildKnowledgeGraph = (projectId: string) =>
  invoke<void>("build_knowledge_graph", { projectId });
// Stop the running build. Signals the whole process group, so the agent CLI
// graphify spawns per document stops with it.
export const stopKnowledgeBuild = (projectId: string) =>
  invoke<void>("stop_knowledge_build", { projectId });

// The Map's drill-down view of the knowledge graph: the directory tree with
// per-file symbols, file-level dependency edges (category counts), and
// symbol-level edges for the detail panel. Computed backend-side from the
// primary repo's graphify-out/graph.json. Resolves to null when no graph has
// been built yet, which is the Map's empty state; it rejects only when a graph
// exists but could not be read, and then the Map shows that error.
export interface MapSymbol {
  id: string;
  label: string;
  line: number | null;
  callable: boolean;
  class: boolean;
  community: string;
}
export interface MapFile {
  name: string;
  path: string;
  symbols: MapSymbol[];
}
export interface MapDir {
  name: string;
  path: string;
  dirs: MapDir[];
  files: MapFile[];
}
export interface MapFileEdge {
  source: string;
  target: string;
  calls: number;
  imports: number;
  refs: number;
  other: number;
}
export interface KnowledgeGraphView {
  root: MapDir;
  file_edges: MapFileEdge[];
  /** [source symbol id, target symbol id, relation] */
  symbol_edges: [string, string, string][];
  stats: { files: number; symbols: number; edges: number; communities: number };
}
export const knowledgeGraphView = (projectId: string) =>
  invoke<KnowledgeGraphView | null>("knowledge_graph_view", { projectId });
// Opens a terminal running `install_command`; returns it so the caller can jump in.
export const installKnowledgeTooling = (projectId: string) =>
  invoke<RunInfo>("install_knowledge_tooling", { projectId });

// Per-project list of files copied into every new agent worktree (persisted to
// the project's gitignored .agency/agency.local.toml). `detectedEnv` is the set
// of untracked root .env* files copied automatically on top of `copy`.
export interface FilesConfig {
  copy: string[];
  detectedEnv: string[];
}

export const getFilesConfig = (projectId: string) =>
  invoke<FilesConfig>("get_files_config", { projectId });
export const saveFilesConfig = (projectId: string, copy: string[]) =>
  invoke<void>("save_files_config", { projectId, copy });

export const listProfiles = () => invoke<AgentProfile[]>("list_profiles");
export const saveProfile = (profile: AgentProfile) =>
  invoke<void>("save_profile", { profile });
export const deleteProfile = (name: string) => invoke<void>("delete_profile", { name });

/** One built-in agent from the catalog (may or may not be enabled yet). */
export interface CatalogEntry {
  id: string;
  command: string;
  resumeArgs: string[] | null;
  loopArgs: string[] | null;
  enabled: boolean;
  installed: boolean;
  /** Whether Agency emits per-workspace MCP config for this agent. */
  supportsMcp: boolean;
  /** Whether Agency can register an OAuth MCP server with this agent's own CLI. */
  supportsMcpAuth: boolean;
  /**
   * Whether this CLI can be handed an opening prompt when the run is created.
   * False means a dispatched issue or review starts the agent promptless.
   */
  acceptsPrompt: boolean;
  /**
   * This agent's interactive surface is a browser GUI served from the
   * workspace; Agency renders it beside the server's log pane.
   */
  servesWebUi: boolean;
}

/**
 * What the model picker offers for one agent. `supported: false` means that
 * CLI takes no model flag at all, so no picker is shown for it and its runs use
 * whatever it is configured to use.
 */
export interface AgentModelInfo {
  agent: string;
  supported: boolean;
  /** Stable vendor aliases worth one click; often empty by design. */
  suggested: string[];
  /** Models used before with this agent, newest first. */
  recent: string[];
  /** Chosen last time; null = the agent's own default. */
  selected: string | null;
  /**
   * The agent's own command for listing what it can run ("opencode models"),
   * or null where its CLI has no such command. Both the hint under the picker's
   * field and what `probeAgentModels` runs.
   */
  listCommand: string | null;
}

/**
 * One attempt in a race: an agent, and the model it runs on (null = the agent's
 * own default). The attempt is the unit rather than the agent, so the same
 * agent can race itself on two models.
 */
export interface RaceAttempt {
  agent: string;
  model: string | null;
}

export const listAgentModels = () => invoke<AgentModelInfo[]>("list_agent_models");

/**
 * Run `agent`'s own listing command and return the model ids it reports.
 *
 * Only for agents whose `listCommand` is set, and only from an open picker:
 * this starts the agent's CLI and waits on a network round trip (about a
 * second). Cached for the app session on the Rust side, so reopening a picker
 * does not run the CLI again. Rejects with the CLI's own last words when it
 * fails or lists nothing, which is why the typed field never goes away.
 *
 * `projectId` is the project the picker is open in. opencode reads an
 * `opencode.json` beside the code, so its listing command has to run there or
 * the project's own providers are missing from the answer (AGE-135); agents
 * configured once for the user ignore it, and are cached once for all projects.
 */
export const probeAgentModels = (agent: string, projectId: string | null) =>
  invoke<string[]>("probe_agent_models", { agent, projectId });

/**
 * Choose the model `agent`'s next run starts on; null is the agent's own
 * default. The same setting a launch writes, which is why the Settings row and
 * the pickers can't drift apart: there is one model the next run starts on.
 */
export const setAgentModel = (agent: string, model: string | null) =>
  invoke<void>("set_agent_model", { agent, model });

/**
 * The model `agent` last ran on — what a spawn with no picker in front of it
 * should repeat, so a run started from a shortcut doesn't quietly drop back to
 * the default model. Null on any failure: the agent's own default is the safe
 * answer, and it is what every run got before models could be chosen.
 */
export const rememberedModel = (agent: string) =>
  listAgentModels()
    .then((ms) => ms.find((m) => m.agent === agent)?.selected ?? null)
    .catch(() => null);

export const agentOnboardingNeeded = () => invoke<boolean>("agent_onboarding_needed");
export const listAgentCatalog = () => invoke<CatalogEntry[]>("list_agent_catalog");
export const enableAgentProfiles = (ids: string[]) =>
  invoke<void>("enable_agent_profiles", { ids });
export const completeAgentOnboarding = (ids: string[]) =>
  invoke<void>("complete_agent_onboarding", { ids });

export const getSettings = () => invoke<ProviderSettings>("get_settings");
export const saveSettings = (settings: ProviderSettings) =>
  invoke<void>("save_settings", { settings });

export type MergeOutcome =
  | { kind: "clean"; commit: string }
  | { kind: "conflicts"; files: string[] };

export interface MergePreview {
  base: string;
  branch: string;
  commitsAhead: number;
  commitsBehind: number;
  worktreeDirty: boolean;
  dirtyFiles: string[];
  /** The branch's remote-tracking ref (`origin/agent/foo`), or null when the
   *  branch was never pushed and so has no remote copy to offer deleting. */
  remoteBranch: string | null;
}

// How far the user is from usable PR features; each non-ready state maps to a
// guided setup step (install gh → gh auth login → add a GitHub remote).
export type GhReadiness = "notInstalled" | "notAuthenticated" | "noGithubRemote" | "ready";

export interface PrInfo {
  number: number;
  url: string;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  isDraft: boolean;
  baseRefName: string;
  headRefName: string;
}

export interface CheckItem {
  name: string;
  bucket: "pass" | "fail" | "pending" | "skipping" | "cancel" | "";
  link: string;
  description: string;
}

export interface PrStatus {
  pr: PrInfo | null;
  checks: CheckItem[];
}

export interface IssueItem {
  number: number;
  title: string;
}

// ── In-app PR review ────────────────────────────────────────────────────────

export interface Author {
  login: string;
}

export interface PrDetail {
  number: number;
  url: string;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  isDraft: boolean;
  baseRefName: string;
  headRefName: string;
  body: string;
  headRefOid: string;
  mergeable: string; // MERGEABLE | CONFLICTING | UNKNOWN
  reviewDecision: string | null; // APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED | null
  author: Author;
  createdAt: string;
  updatedAt: string;
}

// One file of a PR diff: the local FileDiff shape plus path metadata.
export interface PrFileDiff extends FileDiff {
  path: string;
  oldPath: string | null;
  binary: boolean;
}

export interface PrReviewComment {
  id: string;
  databaseId: number;
  path: string;
  line: number | null;
  originalLine: number | null;
  diffHunk: string;
  body: string;
  author: string;
  createdAt: string;
  inReplyToId: string | null;
}

export interface ReviewThread {
  id: string;
  isResolved: boolean;
  isOutdated: boolean;
  path: string;
  line: number | null;
  diffSide: string; // RIGHT | LEFT
  comments: PrReviewComment[];
}

export type ReviewEvent = "APPROVE" | "REQUEST_CHANGES" | "COMMENT";

// A new inline comment to include when submitting a review. Anchoring (line +
// side, and start_* for multi-line) is computed on the frontend from the
// selected diff rows.
export interface DraftComment {
  path: string;
  body: string;
  line: number;
  side: "RIGHT" | "LEFT";
  startLine?: number | null;
  startSide?: "RIGHT" | "LEFT" | null;
}

export const createRace = (
  projectId: string,
  prompt: string,
  attempts: RaceAttempt[],
  base: string,
  mergeTarget?: string | null,
) => invoke<RunInfo[]>("create_race", { projectId, prompt, attempts, base, mergeTarget: mergeTarget ?? null });
export const listGhIssues = (projectId: string) =>
  invoke<IssueItem[]>("list_gh_issues", { projectId });
export const listGhPrs = (projectId: string) => invoke<PrInfo[]>("list_gh_prs", { projectId });
export const createRunFromIssue = (projectId: string, number: number, agent: string, model?: string | null) =>
  invoke<RunInfo>("create_run_from_issue", { projectId, number, agent, model: model ?? null });
export const createRunFromPr = (projectId: string, number: number, agent: string, model?: string | null) =>
  invoke<RunInfo>("create_run_from_pr", { projectId, number, agent, model: model ?? null });

// Where an agent PR review landed. `sessionId` is set when the review had to run
// as an extra tab inside an existing run (the PR's branch was already checked
// out there) — focus that tab rather than the run's primary agent.
export interface PrAgentRun {
  run: RunInfo;
  sessionId: string | null;
}
// Start an agent that reviews a PR and then stays around to fix what it found.
// `postComments` publishes the findings to the PR on GitHub.
export const createPrReviewRun = (
  projectId: string,
  number: number,
  agent: string,
  model: string | null,
  postComments: boolean,
) => invoke<PrAgentRun>("create_pr_review_run", { projectId, number, agent, model, postComments });

// Start an agent that clears a PR's merge conflicts: merges the base branch
// into the PR's branch, resolves, and pushes.
export const createPrConflictRun = (
  projectId: string,
  number: number,
  agent: string,
  model: string | null,
) => invoke<PrAgentRun>("create_pr_conflict_run", { projectId, number, agent, model });

export const ghReadiness = (projectId: string) =>
  invoke<GhReadiness>("gh_readiness", { projectId });
// gh install + auth state without a repo — for the clone dialog's sign-in guidance.
export const ghAuthReadiness = () => invoke<GhReadiness>("gh_auth_readiness");
export const createPr = (taskId: string) => invoke<PrInfo>("create_pr", { taskId });
export const prStatus = (taskId: string) => invoke<PrStatus>("pr_status", { taskId });
// Resolves true if the text is in the agent's session already, false if it is
// queued behind the turn the agent is in the middle of.
export const sendCheckFeedback = (taskId: string) =>
  invoke<boolean>("send_check_feedback", { taskId });

// In-app PR review, keyed by (projectId, prNumber).
export const prDetail = (projectId: string, number: number) =>
  invoke<PrDetail | null>("pr_detail", { projectId, number });
export const ghCurrentLogin = (projectId: string) =>
  invoke<string>("gh_current_login", { projectId });
export interface MergeMethods {
  merge: boolean;
  squash: boolean;
  rebase: boolean;
}
export type MergeMethod = "merge" | "squash" | "rebase";
/** What happened to the head branch after a successful PR merge. */
export interface PrMergeResult {
  branchDeleted: string | null;
  // Set when the merge landed but branch cleanup didn't; a note, not a failure.
  warning: string | null;
}
export const prMergeMethods = (projectId: string) =>
  invoke<MergeMethods>("pr_merge_methods", { projectId });
export const mergePr = (projectId: string, number: number, method: MergeMethod, deleteBranch: boolean) =>
  invoke<PrMergeResult>("merge_pr", { projectId, number, method, deleteBranch });
/** Where a conflicted PR's conflicts are. GitHub only reports *that* a PR
 * conflicts, so the files come from a local probe: `probed` false means that
 * probe couldn't run, and `files` is empty for want of an answer rather than
 * because the merge is clean. */
export interface PrConflicts {
  base: string;
  head: string;
  files: string[];
  probed: boolean;
}
export const prConflicts = (projectId: string, number: number) =>
  invoke<PrConflicts>("pr_conflicts", { projectId, number });
export const prDiff = (projectId: string, number: number) =>
  invoke<PrFileDiff[]>("pr_diff", { projectId, number });
export const prReviewThreads = (projectId: string, number: number) =>
  invoke<ReviewThread[]>("pr_review_threads", { projectId, number });
export const submitPrReview = (
  projectId: string,
  number: number,
  event: ReviewEvent,
  body: string | null,
  comments: DraftComment[],
) => invoke<void>("submit_pr_review", { projectId, number, event, body, comments });
export const replyPrComment = (projectId: string, number: number, inReplyTo: number, body: string) =>
  invoke<void>("reply_pr_comment", { projectId, number, inReplyTo, body });
// Rewrite the PR's title and/or description. A null field is left untouched, so
// a title-only edit never sends a description back.
export const editPr = (projectId: string, number: number, title: string | null, body: string | null) =>
  invoke<void>("edit_pr", { projectId, number, title, body });
// Rewrite one of your own review comments. `commentId` is a comment's
// `databaseId` (the REST integer id), not the GraphQL node id.
export const editPrComment = (projectId: string, commentId: number, body: string) =>
  invoke<void>("edit_pr_comment", { projectId, commentId, body });
export const resolvePrThread = (projectId: string, threadId: string) =>
  invoke<void>("resolve_pr_thread", { projectId, threadId });
export const unresolvePrThread = (projectId: string, threadId: string) =>
  invoke<void>("unresolve_pr_thread", { projectId, threadId });
export const prNumberForRun = (taskId: string) =>
  invoke<number | null>("pr_number_for_run", { taskId });
export const createPrFromBranch = (
  projectId: string,
  head: string,
  base: string | null,
  title: string | null,
  body: string | null,
) => invoke<PrInfo>("create_pr_from_branch", { projectId, head, base, title, body });

export const mergePreview = (taskId: string) =>
  invoke<MergePreview>("merge_preview", { taskId });
/** Delete a merged run's branch from its remote. Resolves to the ref that went,
 *  or null when the remote no longer had it. Rejects when the remote branch
 *  carries commits the base doesn't have. */
export const deleteRunRemoteBranch = (taskId: string) =>
  invoke<string | null>("delete_run_remote_branch", { taskId });
// The merge family streams progress like the teardowns do: it runs `git merge`
// in the project's shared checkout, which is a branch checkout plus a merge, and
// on a large repo that is seconds with nothing else to show.
export const mergeTask = (taskId: string, onProgress?: (p: CloneProgress) => void) => {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<MergeOutcome>("merge_task", { taskId, onProgress: onProgressChannel });
};
/** Where a conflicted merge stands, asked of git rather than re-run. */
export interface MergeState {
  merging: boolean;
  unresolved: string[];
  merged: boolean;
  // Set when the project's checkout is mid-merge of another run's branch. All
  // runs merge in the same checkout, so one unfinished merge blocks the rest.
  blockedBy: string | null;
}
export const mergeStatus = (taskId: string) =>
  invoke<MergeState>("merge_status", { taskId });
// Commit a resolved merge (or accept one the resolver committed itself) and
// put the checkout back on the branch it was on.
export const finishMergeTask = (taskId: string, onProgress?: (p: CloneProgress) => void) => {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<MergeOutcome>("finish_merge_task", { taskId, onProgress: onProgressChannel });
};
export const abortMergeTask = (taskId: string, onProgress?: (p: CloneProgress) => void) => {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("abort_merge_task", { taskId, onProgress: onProgressChannel });
};
// "Fix with agent": types the conflict, with git's own status output, into this
// run's live agent session. Resolves true if it went in now, false if it is
// queued behind the agent's current turn.
export const sendMergeConflict = (taskId: string, sessionId?: string) =>
  invoke<boolean>("send_merge_conflict", { taskId, sessionId: sessionId ?? null });

export interface Hunk {
  header: string;
  lines: string[];
}
export interface FileDiff {
  header: string;
  hunks: Hunk[];
}

export const gitParseDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<FileDiff>("git_parse_diff", { taskId, path, staged });

/** One side of a file's change, as bytes. Empty with `tooLarge` when the file is
 *  past the preview cap. */
export interface BlobSide {
  b64: string;
  size: number;
  tooLarge: boolean;
}
/** Before/after bytes of a file git can't diff as text. A side is null when the
 *  file doesn't exist there: an add has no `old`, a delete no `new`. */
export interface BlobSides {
  mime: string;
  old: BlobSide | null;
  new: BlobSide | null;
}

// Pass `hash` for a commit (compared against its first parent); otherwise
// `staged` picks HEAD-vs-index or index-vs-working-tree, as in gitParseDiff.
export const gitBlobSides = (taskId: string, path: string, staged: boolean, hash: string | null) =>
  invoke<BlobSides>("git_blob_sides", { taskId, path, staged, hash });
export const gitStageHunk = (taskId: string, path: string, hunkIndex: number) =>
  invoke<void>("git_stage_hunk", { taskId, path, hunkIndex });
export const gitUnstageHunk = (taskId: string, path: string, hunkIndex: number) =>
  invoke<void>("git_unstage_hunk", { taskId, path, hunkIndex });
export interface HistoryItem {
  hash: string;
  parents: string[];
  author: string;
  email: string;
  date: number;
  subject: string;
  refs: string[];
}

export interface BranchInfo {
  branch: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  base: string | null;
  hasRemote: boolean;
}

export const gitLogGraph = (taskId: string, limit: number) =>
  invoke<HistoryItem[]>("git_log_graph", { taskId, limit });
export const gitBranchInfo = (taskId: string) =>
  invoke<BranchInfo>("git_branch_info", { taskId });

export interface ProjectBranches {
  current: string;
  branches: string[];
  /** Branches that exist on origin but not locally, without the `origin/` prefix. */
  remote: string[];
}

export const listProjectBranches = (projectId: string) =>
  invoke<ProjectBranches>("list_project_branches", { projectId });

export interface CommitFile {
  path: string;
  status: string;
}

export const gitCommitFiles = (taskId: string, hash: string) =>
  invoke<CommitFile[]>("git_commit_files", { taskId, hash });
export const gitCommitDiff = (taskId: string, hash: string, path: string) =>
  invoke<string>("git_commit_diff", { taskId, hash, path });
export const gitCommitAmend = (taskId: string, message: string) =>
  invoke<void>("git_commit_amend", { taskId, message });

export const gitCheckoutBranch = (taskId: string, name: string) =>
  invoke<void>("git_checkout_branch", { taskId, name });
export const gitCreateBranch = (taskId: string, name: string, from: string | null, checkout: boolean) =>
  invoke<void>("git_create_branch", { taskId, name, from, checkout });
export const gitDeleteBranch = (taskId: string, name: string, force: boolean) =>
  invoke<void>("git_delete_branch", { taskId, name, force });
export const gitListBranches = (taskId: string) =>
  invoke<ProjectBranches>("git_list_branches", { taskId });
export const gitPullRebase = (taskId: string) => invoke<void>("git_pull_rebase", { taskId });
// Force-pushes the current branch (with lease), streaming git's progress like
// gitPush: a force push after a rebase re-uploads the whole branch. cancelPush
// stops this one too.
export function gitPushForce(
  taskId: string,
  onProgress?: (p: CloneProgress) => void,
): Promise<void> {
  const onProgressChannel = new Channel<CloneProgress>();
  if (onProgress) onProgressChannel.onmessage = onProgress;
  return invoke<void>("git_push_force", { taskId, onProgress: onProgressChannel });
}
/** Resolves to the undone commit's message so it can be restored into the input. */
export const gitUndoLastCommit = (taskId: string) =>
  invoke<string>("git_undo_last_commit", { taskId });
export const gitResetTo = (taskId: string, hash: string, mode: "soft" | "mixed" | "hard") =>
  invoke<void>("git_reset_to", { taskId, hash, mode });
export const gitRevertCommit = (taskId: string, hash: string) =>
  invoke<void>("git_revert_commit", { taskId, hash });
export const gitCherryPick = (taskId: string, hash: string) =>
  invoke<void>("git_cherry_pick", { taskId, hash });

export interface StashEntry {
  index: number;
  message: string;
}

export const gitStashList = (taskId: string) =>
  invoke<StashEntry[]>("git_stash_list", { taskId });
export const gitStashPush = (taskId: string, message: string | null, includeUntracked: boolean) =>
  invoke<void>("git_stash_push", { taskId, message, includeUntracked });
export const gitStashApply = (taskId: string, index: number) =>
  invoke<void>("git_stash_apply", { taskId, index });
export const gitStashPop = (taskId: string, index: number) =>
  invoke<void>("git_stash_pop", { taskId, index });
export const gitStashDrop = (taskId: string, index: number) =>
  invoke<void>("git_stash_drop", { taskId, index });

export const gitStageLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_stage_lines", { taskId, path, hunkIndex, lines });
export const gitUnstageLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_unstage_lines", { taskId, path, hunkIndex, lines });
export const gitRevertLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_revert_lines", { taskId, path, hunkIndex, lines });

export interface NotifSettings {
  agentFinished: boolean;
  agentIdle: boolean;
  runCrashed: boolean;
  mergeAttention: boolean;
  loopEvents: boolean;
  /** Skip notifications for the run that's open in a focused window; every
   *  other run still notifies while you work in the app. */
  onlyWhenWatching: boolean;
  idleSecs: number;
}

export const setUiState = (focused: boolean, activeRun: string | null) =>
  invoke<void>("set_ui_state", { focused, activeRun });
// Toggle the native menu's context-dependent items: project-gated (New
// Agent/Terminal, Source) and agent-gated (the Agent menu).
export const setMenuContext = (project: boolean, focusedAgent: boolean) =>
  invoke<void>("set_menu_context", { project, focusedAgent });
export const getNotifSettings = () => invoke<NotifSettings>("get_notif_settings");
export const saveNotifSettings = (settings: NotifSettings) =>
  invoke<void>("save_notif_settings", { settings });

export interface UpdateCheck {
  current: string;
  /** Latest published release, or null when the check couldn't complete. */
  latest: string | null;
  updateAvailable: boolean;
  /** Releases page to send the user to. */
  url: string;
  /** Why the check came back empty; null on success. */
  error: string | null;
}

/** Asks GitHub for the latest release. Resolves (never rejects) when offline —
 *  inspect `error`. Agency downloads nothing; the user installs the DMG. */
export const checkForUpdate = () => invoke<UpdateCheck>("check_for_update");
export const getUpdateCheckEnabled = () => invoke<boolean>("get_update_check_enabled");
export const setUpdateCheckEnabled = (enabled: boolean) =>
  invoke<void>("set_update_check_enabled", { enabled });

export interface ReviewComment {
  id: string;
  runId: string;
  path: string;
  lineStart: number;
  lineEnd: number;
  body: string;
  sent: boolean;
  createdAt: number;
}

export const addReviewComment = (
  runId: string, path: string, lineStart: number, lineEnd: number, body: string,
) => invoke<ReviewComment>("add_review_comment", { runId, path, lineStart, lineEnd, body });
export const listReviewComments = (runId: string) =>
  invoke<ReviewComment[]>("list_review_comments", { runId });
export const deleteReviewComment = (id: string) =>
  invoke<void>("delete_review_comment", { id });
// Resolves true if the comments are in the agent's session already, false if
// they are queued behind the turn it is in the middle of.
export const sendReviewComments = (runId: string) =>
  invoke<boolean>("send_review_comments", { runId });

// What the run is still owed, oldest first. The count is on RunInfo; this is
// the detail behind the marker.
export const listQueuedMessages = (runId: string) =>
  invoke<QueuedMessage[]>("list_queued_messages", { runId });
// Resolves false if the message went in (or was discarded) before the click
// landed, so there was nothing left to drop.
export const cancelQueuedMessage = (sessionId: string, text: string) =>
  invoke<boolean>("cancel_queued_message", { sessionId, text });

export type FileRoot = { kind: "run"; id: string } | { kind: "project"; id: string };

/**
 * The file root behind a git panel's task token, mirroring the backend's
 * `git_root`: a "project:<id>" token means the project's main checkout, any
 * other token is a run's worktree.
 */
export const fileRootOf = (taskId: string): FileRoot =>
  taskId.startsWith("project:")
    ? { kind: "project", id: taskId.slice("project:".length) }
    : { kind: "run", id: taskId };

export interface DirEntry {
  name: string;
  isDir: boolean;
  // For directories: whether they hold at least one entry (drives the expand arrow).
  hasChildren: boolean;
}

export interface FileContents {
  text: string;
  binary: boolean;
  tooLarge: boolean;
}

export const listDir = (root: FileRoot, relPath: string) =>
  invoke<DirEntry[]>("list_dir", { root, relPath });

export const readFile = (root: FileRoot, relPath: string) =>
  invoke<FileContents>("read_file", { root, relPath });

export interface BinaryContents {
  b64: string;
  mime: string;
  tooLarge: boolean;
}

export const readFileBase64 = (root: FileRoot, relPath: string) =>
  invoke<BinaryContents>("read_file_base64", { root, relPath });

export const writeFile = (root: FileRoot, relPath: string, contents: string) =>
  invoke<void>("write_file", { root, relPath, contents });

export const createFile = (root: FileRoot, relPath: string) =>
  invoke<void>("create_file", { root, relPath });
export const createDir = (root: FileRoot, relPath: string) =>
  invoke<void>("create_dir", { root, relPath });
export const renamePath = (root: FileRoot, from: string, to: string) =>
  invoke<void>("rename_path", { root, from, to });
// Copy a file or folder to another path in the same root (folders recurse).
// Refuses an existing destination, so the caller picks a free name first.
export const copyPath = (root: FileRoot, from: string, to: string) =>
  invoke<void>("copy_path", { root, from, to });
export const trashPath = (root: FileRoot, relPath: string) =>
  invoke<void>("trash_path", { root, relPath });
// Appends the path to the root's .gitignore. Resolves to true if a new entry was
// written, false if the path was already ignored.
export const addToGitignore = (root: FileRoot, relPath: string) =>
  invoke<boolean>("add_to_gitignore", { root, relPath });
export const absPath = (root: FileRoot, relPath: string) =>
  invoke<string>("abs_path", { root, relPath });
export const revealPath = (root: FileRoot, relPath: string) =>
  invoke<void>("reveal_path", { root, relPath });

/** A path printed in terminal output, resolved on disk (see lib/termLinks). */
export interface LinkedPath {
  absPath: string;
  /** Relative to the root, or null when the path lives outside it. */
  relPath: string | null;
  isDir: boolean;
}

// Which of the path-shaped words on a hovered terminal line are real paths.
// Answers positionally, null where nothing resolved.
export const resolveTermPaths = (root: FileRoot, paths: string[]) =>
  invoke<(LinkedPath | null)[]>("resolve_term_paths", { root, paths });

// Hand a clicked path to the OS — a directory to the file manager, a file
// outside the root to its default app.
export const openTermPath = (root: FileRoot, path: string) =>
  invoke<void>("open_term_path", { root, path });

// Actual on-disk name of the project's top-level docs folder (case-insensitive
// match), or null when the project has none.
export const detectDocsDir = (projectId: string) =>
  invoke<string | null>("detect_docs_dir", { projectId });

export interface DocFile {
  // Path relative to the docs dir, "/"-separated.
  path: string;
  text: string;
  tooLarge: boolean;
}

// Every markdown file under the docs dir in one call — feeds the docs index.
export const readDocsCorpus = (root: FileRoot, docsDir: string) =>
  invoke<DocFile[]>("read_docs_corpus", { root, docsDir });

// Stat-only corpus pass: change signatures without body reads. Steady-state
// docs polls diff these and re-read only what changed.
export interface DocStat {
  path: string;
  mtimeMs: number;
  size: number;
}
export interface DocsScan {
  files: DocStat[];
  // Every folder under the docs dir, sorted. The tree's folders otherwise come
  // from note paths alone, which hides a folder until it has a note in it.
  dirs: string[];
  // Every non-markdown file under the docs dir, sorted (dotfiles excluded).
  // Names only — the tree draws a row per attachment and the Files viewer
  // loads whichever one is opened.
  attachments: string[];
}
export const docsCorpusStats = (root: FileRoot, docsDir: string) =>
  invoke<DocsScan>("docs_corpus_stats", { root, docsDir });
// Read a named subset of the corpus (the poll's "these changed" list).
export const readDocsFiles = (root: FileRoot, docsDir: string, paths: string[]) =>
  invoke<DocFile[]>("read_docs_files", { root, docsDir, paths });

// ── checkbox tasks (one-stop Phase 8) ───────────────────────────────────────

export interface TaskHit {
  path: string; // relative to the docs dir, "/"-separated
  line: number; // 0-based
  checked: boolean;
  text: string; // marker stripped, trimmed
}

// Every checkbox task in a root's docs corpus (same file set as the index).
export const scanTasks = (root: FileRoot, docsDir: string) =>
  invoke<TaskHit[]>("scan_tasks", { root, docsDir });

// Flip one task's checkbox in place. False = the line no longer holds the
// expected task (external edit); the caller refreshes instead of writing.
export const toggleTask = (root: FileRoot, docsDir: string, relPath: string, line: number, checked: boolean) =>
  invoke<boolean>("toggle_task", { root, docsDir, relPath, line, checked });

// ── content search (one-stop Phase 2 primitive) ─────────────────────────────

export interface SearchQuery {
  query: string;
  // Treat query as a regex; default literal.
  regex?: boolean;
  // Case-sensitive when true; default insensitive.
  case?: boolean;
  // Gitignore-style globs ("*.md" matches at any depth); empty = all files.
  globs?: string[];
  maxHits?: number;
}

export interface BackendSearchHit {
  path: string; // relative to the searched dir
  line: number; // 1-based
  col: number; // 1-based
  text: string; // the matching line, length-capped
}

// Content search under `dir` within a root. Bounded (hits/bytes/time) and
// best-effort: hitting a cap returns what was collected.
export const searchFiles = (root: FileRoot, dir: string, query: SearchQuery) =>
  invoke<BackendSearchHit[]>("search_files", { root, dir, query });

// Sorted relative file paths under `dir` — quick-open's name list. Same file
// set as the search fallback (gitignore respected in repos), capped backend-side.
export const listFiles = (root: FileRoot, dir: string, maxFiles: number) =>
  invoke<string[]>("list_files", { root, dir, maxFiles });

// Write base64 bytes to a NEW file (fails on an existing path). For image paste.
export const writeFileBase64 = (root: FileRoot, relPath: string, b64: string) =>
  invoke<void>("write_file_base64", { root, relPath, b64 });

// Copy a file from anywhere on disk into a NEW path inside the root (fails on
// an existing path). For attaching files dropped from Finder or picked in the
// dialog, where the UI has a path but not the bytes.
export const importFile = (root: FileRoot, srcPath: string, relPath: string) =>
  invoke<void>("import_file", { root, srcPath, relPath });

// The whole system clipboard as text (every pasteboard item, newline-joined).
// The webview's own copy of a paste is not trustworthy — see lib/clipboard.ts.
export const readClipboardText = () => invoke<string | null>("read_clipboard_text");
