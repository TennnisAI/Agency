//! GitHub integration via the `gh` CLI.
//!
//! Agency deliberately shells out to `gh` instead of embedding a GitHub OAuth
//! flow: auth lives in gh's own keychain storage and never touches Agency,
//! GitHub Enterprise works through gh's host config, and there is no vendor
//! app identity in the middle of the user's GitHub traffic. `GhCli` is the
//! seam where another provider (glab for GitLab, a native GitHub App) could
//! present the same surface later.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// How far the user is from being able to use PR features, in the order the
/// gaps must be fixed. Each non-`Ready` state maps to a guided UI step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GhReadiness {
    /// `gh` is not on PATH.
    NotInstalled,
    /// `gh` exists but `gh auth status` fails.
    NotAuthenticated,
    /// Authenticated, but the repo has no resolvable GitHub remote.
    NoGithubRemote,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    pub title: String,
    /// OPEN | CLOSED | MERGED
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub head_ref_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckItem {
    pub name: String,
    /// gh's rollup of the check state: pass | fail | pending | skipping | cancel
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueItem {
    pub number: u64,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub url: String,
}

/// A GitHub account reference (comment/PR author). Extra fields from the API
/// are ignored.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    #[serde(default)]
    pub login: String,
}

/// Full detail for one PR, including the fields the list/status views omit:
/// `body` (description), `head_ref_oid` (the head SHA a review must anchor to),
/// `mergeable`, and `review_decision`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDetail {
    pub number: u64,
    pub url: String,
    pub title: String,
    /// OPEN | CLOSED | MERGED
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub body: String,
    /// Head commit SHA — the `commit_id` a submitted review anchors to.
    #[serde(default)]
    pub head_ref_oid: String,
    /// MERGEABLE | CONFLICTING | UNKNOWN
    #[serde(default)]
    pub mergeable: String,
    /// APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED — JSON `null` when the PR
    /// has no decision yet, so this must be an Option (a defaulted String would
    /// still fail to deserialize an explicit null).
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub author: Author,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// One file's slice of a PR diff. Same `header`/`hunks` shape the local diff
/// renderer already consumes (`crate::git::FileDiff`), plus the path metadata a
/// multi-file diff needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFileDiff {
    /// New path (post-change). For a deletion this is the old path.
    pub path: String,
    /// Old path, present only when it differs from `path` (a rename).
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub binary: bool,
    pub header: String,
    pub hunks: Vec<crate::git::Hunk>,
}

/// A resolvable review conversation on a PR. `id` is the GraphQL node id used by
/// the resolve/unresolve mutations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    /// File the thread is anchored to (thread-level, authoritative).
    #[serde(default)]
    pub path: String,
    /// Current line the thread anchors to; `null` when outdated.
    pub line: Option<u64>,
    /// RIGHT | LEFT — which side of the diff the thread sits on. Lets the UI
    /// anchor a LEFT (deletion) thread to the old-file line instead of guessing.
    #[serde(default)]
    pub diff_side: String,
    pub comments: Vec<PrReviewComment>,
}

/// One inline review comment. Named to avoid colliding with the unrelated
/// local `registry::ReviewComment` (which is typed into the agent, not GitHub).
/// `database_id` is the REST integer id a reply anchors to via `in_reply_to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewComment {
    pub id: String,
    pub database_id: u64,
    #[serde(default)]
    pub path: String,
    /// Current line in the diff; `null` once the comment goes outdated.
    pub line: Option<u64>,
    pub original_line: Option<u64>,
    #[serde(default)]
    pub diff_hunk: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub created_at: String,
    pub in_reply_to_id: Option<String>,
}

/// A new inline comment to include when submitting a review. Anchoring is
/// computed on the frontend from the selected diff rows (added/context →
/// RIGHT/newNo, deleted → LEFT/oldNo). `start_*` is set only for multi-line
/// selections and must match `side`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftComment {
    pub path: String,
    pub body: String,
    pub line: u64,
    /// RIGHT | LEFT
    pub side: String,
    #[serde(default)]
    pub start_line: Option<u64>,
    #[serde(default)]
    pub start_side: Option<String>,
}

/// The merge methods a repo permits (mirrors GitHub's repo settings), so the UI
/// offers only the buttons that will actually work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeMethods {
    pub merge: bool,
    pub squash: bool,
    pub rebase: bool,
}

/// Outcome of a PR merge. The merge itself either succeeded or returned an
/// error; this reports what happened to the head branch afterwards, so
/// best-effort cleanup can be shown as a note rather than a failed merge.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrMergeResult {
    /// The remote branch that was deleted, if any.
    pub branch_deleted: Option<String>,
    /// Set when the merge landed but branch cleanup did not.
    pub warning: Option<String>,
}

pub struct GhCli {
    bin: String,
}

impl Default for GhCli {
    fn default() -> Self {
        GhCli { bin: "gh".to_string() }
    }
}

impl GhCli {
    /// Test seam: point at a fake `gh` script.
    pub fn with_bin(bin: impl Into<String>) -> Self {
        GhCli { bin: bin.into() }
    }

    fn run(&self, repo: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
        crate::procutil::retry_etxtbsy(|| {
            Command::new(&self.bin).args(args).current_dir(repo).output()
        })
    }

    fn run_ok(&self, repo: &Path, args: &[&str]) -> Result<String> {
        let out = self.run(repo, args)?;
        if !out.status.success() {
            bail!("gh {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Probe how usable GitHub features are for this repo. Uses `gh repo view`
    /// as the remote check because it resolves remotes the way every other gh
    /// command will (git remotes + gh host config), so Enterprise hosts and
    /// non-`origin` remotes are handled for free.
    pub fn readiness(&self, repo: &Path) -> GhReadiness {
        let version = self.run(repo, &["--version"]);
        match version {
            Err(_) => return GhReadiness::NotInstalled,
            Ok(out) if !out.status.success() => return GhReadiness::NotInstalled,
            Ok(_) => {}
        }
        match self.run(repo, &["auth", "status"]) {
            Ok(out) if out.status.success() => {}
            _ => return GhReadiness::NotAuthenticated,
        }
        match self.run(repo, &["repo", "view", "--json", "nameWithOwner"]) {
            Ok(out) if out.status.success() => GhReadiness::Ready,
            _ => GhReadiness::NoGithubRemote,
        }
    }

    /// How far the user is from being able to authenticate against GitHub, with
    /// no repo in play. Same first two checks as `readiness` (install + auth) but
    /// stops there — used before a clone, when there is no local repo yet. Never
    /// returns `NoGithubRemote`; the caller only cares about NotInstalled vs
    /// NotAuthenticated vs Ready.
    pub fn auth_readiness(&self) -> GhReadiness {
        let here = Path::new(".");
        match self.run(here, &["--version"]) {
            Err(_) => return GhReadiness::NotInstalled,
            Ok(out) if !out.status.success() => return GhReadiness::NotInstalled,
            Ok(_) => {}
        }
        match self.run(here, &["auth", "status"]) {
            Ok(out) if out.status.success() => GhReadiness::Ready,
            _ => GhReadiness::NotAuthenticated,
        }
    }

    /// Clone `url` into `dest` through gh, which injects the user's stored
    /// credentials — so private repos clone without git prompting for a
    /// username/password it can't read. Assumes the caller has confirmed gh is
    /// authenticated (see `auth_readiness`). Progress is streamed to
    /// `on_progress`; `--progress` after `--` is forwarded to the underlying git.
    /// `cancel` kills the clone, so the retry is as escapable as the first try.
    pub fn clone_with_progress(
        &self,
        url: &str,
        dest: &Path,
        cancel: &crate::setup::CancelToken,
        on_progress: &mut dyn FnMut(crate::setup::CloneProgress),
    ) -> Result<()> {
        let dest_str = dest.to_string_lossy();
        let mut cmd = Command::new(&self.bin);
        cmd.arg("repo")
            .arg("clone")
            .arg(url)
            .arg(dest_str.as_ref())
            .arg("--")
            .arg("--progress")
            .current_dir(Path::new("."));
        let (ok, stderr) = crate::setup::run_clone_streaming(cmd, cancel, on_progress)?;
        if !ok {
            bail!("{}", stderr.trim());
        }
        Ok(())
    }

    /// The open or merged PR for `branch`, if any. gh's "no pull requests
    /// found" error is a normal answer here, not a failure.
    pub fn view_pr(&self, repo: &Path, branch: &str) -> Result<Option<PrInfo>> {
        let out = self.run(
            repo,
            &[
                "pr",
                "view",
                branch,
                "--json",
                "number,url,title,state,isDraft,baseRefName,headRefName",
            ],
        )?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.to_lowercase().contains("no pull requests found") {
                return Ok(None);
            }
            bail!("gh pr view failed: {err}");
        }
        Ok(Some(serde_json::from_slice(&out.stdout)?))
    }

    /// Like `view_pr`, addressed by PR number instead of branch.
    pub fn view_pr_by_number(&self, repo: &Path, number: u64) -> Result<Option<PrInfo>> {
        let num = number.to_string();
        let out = self.run(
            repo,
            &[
                "pr",
                "view",
                &num,
                "--json",
                "number,url,title,state,isDraft,baseRefName,headRefName",
            ],
        )?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.to_lowercase().contains("no pull requests found")
                || err.to_lowercase().contains("could not find")
            {
                return Ok(None);
            }
            bail!("gh pr view failed: {err}");
        }
        Ok(Some(serde_json::from_slice(&out.stdout)?))
    }

    /// Open PRs, newest first (capped at 30 for the picker).
    pub fn list_prs(&self, repo: &Path) -> Result<Vec<PrInfo>> {
        let out = self.run_ok(
            repo,
            &[
                "pr",
                "list",
                "--limit",
                "30",
                "--json",
                "number,url,title,state,isDraft,baseRefName,headRefName",
            ],
        )?;
        Ok(serde_json::from_str(&out)?)
    }

    /// Open issues, newest first (capped at 30 for the picker).
    pub fn list_issues(&self, repo: &Path) -> Result<Vec<IssueItem>> {
        let out =
            self.run_ok(repo, &["issue", "list", "--limit", "30", "--json", "number,title"])?;
        Ok(serde_json::from_str(&out)?)
    }

    pub fn view_issue(&self, repo: &Path, number: u64) -> Result<IssueDetail> {
        let num = number.to_string();
        let out = self.run_ok(repo, &["issue", "view", &num, "--json", "number,title,body,url"])?;
        Ok(serde_json::from_str(&out)?)
    }

    pub fn create_pr(
        &self,
        repo: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PrInfo> {
        self.run_ok(
            repo,
            &["pr", "create", "--head", branch, "--base", base, "--title", title, "--body", body],
        )?;
        self.view_pr(repo, branch)?
            .ok_or_else(|| anyhow::anyhow!("PR was created but could not be read back"))
    }

    /// CI/check rollup for the branch's PR. `gh pr checks` exits non-zero when
    /// checks are pending or failing, so the JSON on stdout is authoritative
    /// and the exit code is ignored when it parses. A PR with no checks
    /// configured reports as an empty list.
    pub fn pr_checks(&self, repo: &Path, branch: &str) -> Result<Vec<CheckItem>> {
        let out =
            self.run(repo, &["pr", "checks", branch, "--json", "name,bucket,link,description"])?;
        if let Ok(items) = serde_json::from_slice::<Vec<CheckItem>>(&out.stdout) {
            return Ok(items);
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.to_lowercase().contains("no checks reported") {
            return Ok(Vec::new());
        }
        bail!("gh pr checks failed: {err}");
    }

    /// Run gh with `body` piped to stdin (for `gh api --input -`, whose JSON
    /// payload can't be expressed with `-f` flags). Mirrors `git::git_stdin`.
    fn run_stdin(&self, repo: &Path, args: &[&str], body: &str) -> Result<String> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .current_dir(repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::procutil::retry_etxtbsy(|| cmd.spawn())?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("failed to open gh stdin"))?
            .write_all(body.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("gh {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// `(owner, name)` for the repo's GitHub remote. Resolved explicitly (rather
    /// than relying on gh's `{owner}`/`{repo}` placeholders) because we also need
    /// the slug as GraphQL variables, and explicit interpolation keeps the
    /// fake-gh tests deterministic.
    fn repo_slug(&self, repo: &Path) -> Result<(String, String)> {
        let out = self
            .run_ok(repo, &["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])?;
        let slug = out.trim();
        let (owner, name) = slug
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("unexpected repo slug from gh: {slug:?}"))?;
        Ok((owner.to_string(), name.to_string()))
    }

    /// Which merge methods the repo allows, so the UI only offers valid ones.
    pub fn merge_methods(&self, repo: &Path) -> Result<MergeMethods> {
        let out = self.run_ok(
            repo,
            &["repo", "view", "--json", "mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed"],
        )?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            #[serde(default)]
            merge_commit_allowed: bool,
            #[serde(default)]
            squash_merge_allowed: bool,
            #[serde(default)]
            rebase_merge_allowed: bool,
        }
        let r: Raw = serde_json::from_str(&out)?;
        Ok(MergeMethods {
            merge: r.merge_commit_allowed,
            squash: r.squash_merge_allowed,
            rebase: r.rebase_merge_allowed,
        })
    }

    /// Merge PR `number` with `method` (merge | squash | rebase), optionally
    /// deleting the remote head branch after. gh's stderr (conflicts, required
    /// checks, protected branch) is surfaced as-is on failure.
    ///
    /// Deliberately does NOT use `gh pr merge --delete-branch`: that flag also
    /// deletes the LOCAL branch, and Agency keeps every agent branch checked out
    /// in a worktree, which git refuses to delete out from under. gh reports that
    /// as a command failure even though the merge already landed, so the UI would
    /// show a merged PR as a failed merge. Branch cleanup runs separately here and
    /// downgrades to a warning; the local branch stays put and is removed with the
    /// worktree when the task is archived or discarded.
    pub fn merge_pr(
        &self,
        repo: &Path,
        number: u64,
        method: &str,
        delete_branch: bool,
    ) -> Result<PrMergeResult> {
        let num = number.to_string();
        let flag = match method {
            "squash" => "--squash",
            "rebase" => "--rebase",
            _ => "--merge",
        };
        self.run_ok(repo, &["pr", "merge", num.as_str(), flag])?;
        if !delete_branch {
            return Ok(PrMergeResult::default());
        }
        match self.delete_remote_head_branch(repo, number) {
            Ok(Some(branch)) => Ok(PrMergeResult { branch_deleted: Some(branch), warning: None }),
            Ok(None) => Ok(PrMergeResult {
                branch_deleted: None,
                warning: Some("The head branch is on a fork, so it was left in place.".into()),
            }),
            Err(e) => Ok(PrMergeResult {
                branch_deleted: None,
                warning: Some(format!("The remote branch could not be deleted: {e}")),
            }),
        }
    }

    /// Delete the PR's head branch on the remote. Returns the branch name, or
    /// `None` when the PR comes from a fork (the ref lives in another repo, so
    /// deleting it from the base repo is neither possible nor ours to do).
    fn delete_remote_head_branch(&self, repo: &Path, number: u64) -> Result<Option<String>> {
        let num = number.to_string();
        let out =
            self.run_ok(repo, &["pr", "view", &num, "--json", "headRefName,isCrossRepository"])?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Head {
            #[serde(default)]
            head_ref_name: String,
            #[serde(default)]
            is_cross_repository: bool,
        }
        let head: Head = serde_json::from_str(&out)?;
        if head.is_cross_repository {
            return Ok(None);
        }
        if head.head_ref_name.is_empty() {
            bail!("gh did not report a head branch for #{number}");
        }
        let (owner, name) = self.repo_slug(repo)?;
        let path = format!("repos/{owner}/{name}/git/refs/heads/{}", head.head_ref_name);
        self.run_ok(repo, &["api", "-X", "DELETE", &path])?;
        Ok(Some(head.head_ref_name))
    }

    /// The authenticated gh user's login. Used to detect self-authored PRs —
    /// GitHub forbids approving or requesting changes on your own PR, so the UI
    /// disables those verdicts rather than letting them fail with a 422.
    pub fn current_login(&self, repo: &Path) -> Result<String> {
        Ok(self.run_ok(repo, &["api", "user", "-q", ".login"])?.trim().to_string())
    }

    /// Full detail for PR `number` (adds body/head SHA/mergeable/reviewDecision
    /// over `view_pr`). gh's "no pull requests found" is a normal None, not a
    /// failure — same handling as `view_pr_by_number`.
    pub fn view_pr_detail(&self, repo: &Path, number: u64) -> Result<Option<PrDetail>> {
        let num = number.to_string();
        let out = self.run(
            repo,
            &[
                "pr", "view", &num, "--json",
                "number,url,title,state,isDraft,baseRefName,headRefName,body,headRefOid,mergeable,reviewDecision,author,createdAt,updatedAt",
            ],
        )?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let e = err.to_lowercase();
            if e.contains("no pull requests found")
                || e.contains("could not find")
                || e.contains("not found")
            {
                return Ok(None);
            }
            bail!("gh pr view failed: {err}");
        }
        Ok(Some(serde_json::from_slice(&out.stdout)?))
    }

    /// The PR's full multi-file diff, split into per-file hunks. `gh pr diff`
    /// captured via `.output()` is uncolored and unpaged (stdout is a pipe), so
    /// no extra flags are needed.
    pub fn pr_diff(&self, repo: &Path, number: u64) -> Result<Vec<PrFileDiff>> {
        let num = number.to_string();
        let out = self.run(repo, &["pr", "diff", &num])?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let e = err.to_lowercase();
            if e.contains("no pull requests found") || e.contains("not found") {
                return Ok(Vec::new());
            }
            bail!("gh pr diff failed: {err}");
        }
        Ok(parse_pr_diff(&String::from_utf8_lossy(&out.stdout)))
    }

    /// All review threads on the PR with their resolution state, via GraphQL —
    /// the REST `/pulls/{n}/comments` endpoint carries neither `isResolved` nor
    /// thread grouping, so GraphQL is the single source of truth here. Capped at
    /// 100 threads / 100 comments each for v1.
    pub fn pr_review_threads(&self, repo: &Path, number: u64) -> Result<Vec<ReviewThread>> {
        let (owner, name) = self.repo_slug(repo)?;
        let owner_arg = format!("owner={owner}");
        let name_arg = format!("name={name}");
        let number_arg = format!("number={number}");
        let query_arg = format!("query={THREADS_QUERY}");
        let out = self.run(
            repo,
            &[
                "api",
                "graphql",
                "-f",
                &owner_arg,
                "-f",
                &name_arg,
                "-F",
                &number_arg, // -F: typed, so GraphQL sees Int! not a string
                "-f",
                &query_arg,
            ],
        )?;
        if !out.status.success() {
            bail!(
                "gh api graphql (reviewThreads) failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let resp: GqlThreadsResp = serde_json::from_slice(&out.stdout)?;
        let nodes = resp
            .data
            .repository
            .and_then(|r| r.pull_request)
            .map(|p| p.review_threads.nodes)
            .unwrap_or_default();
        Ok(nodes
            .into_iter()
            .map(|t| ReviewThread {
                id: t.id,
                is_resolved: t.is_resolved,
                is_outdated: t.is_outdated,
                path: t.path.unwrap_or_default(),
                line: t.line,
                diff_side: t.diff_side.unwrap_or_else(|| "RIGHT".to_string()),
                comments: t
                    .comments
                    .nodes
                    .into_iter()
                    .map(|c| PrReviewComment {
                        id: c.id,
                        database_id: c.database_id,
                        path: c.path.unwrap_or_default(),
                        line: c.line,
                        original_line: c.original_line,
                        diff_hunk: c.diff_hunk.unwrap_or_default(),
                        body: c.body.unwrap_or_default(),
                        author: c.author.map(|a| a.login).unwrap_or_default(),
                        created_at: c.created_at.unwrap_or_default(),
                        in_reply_to_id: c.reply_to.map(|r| r.id),
                    })
                    .collect(),
            })
            .collect())
    }

    /// Submit a review as one atomic POST: an event verdict plus optional inline
    /// comments and a summary body. `commit_id` must be the live head SHA (the
    /// reviews API anchors comments by line in the *current* diff), so callers
    /// pass the SHA they just read from `view_pr_detail`. The body is nested +
    /// snake_case — unlike the rest of this module — so it's built with
    /// `serde_json` and piped via `--input -`.
    pub fn submit_pr_review(
        &self,
        repo: &Path,
        number: u64,
        commit_id: &str,
        event: &str,
        body: Option<&str>,
        comments: &[DraftComment],
    ) -> Result<()> {
        let has_body = body.map(|b| !b.trim().is_empty()).unwrap_or(false);
        if (event == "REQUEST_CHANGES" || event == "COMMENT") && !has_body && comments.is_empty() {
            bail!("a {event} review needs a summary or at least one inline comment");
        }
        let comment_json: Vec<serde_json::Value> = comments
            .iter()
            .map(|c| {
                let mut m = serde_json::json!({
                    "path": c.path,
                    "body": c.body,
                    "line": c.line,
                    "side": c.side,
                });
                // Only a genuine multi-line span carries start_*; GitHub rejects
                // start_line == line.
                if let (Some(sl), Some(ss)) = (c.start_line, c.start_side.as_ref()) {
                    if sl != c.line {
                        m["start_line"] = serde_json::json!(sl);
                        m["start_side"] = serde_json::json!(ss);
                    }
                }
                m
            })
            .collect();
        let mut payload = serde_json::json!({
            "commit_id": commit_id,
            "event": event,
            "comments": comment_json,
        });
        if has_body {
            payload["body"] = serde_json::json!(body.unwrap());
        }
        let (owner, name) = self.repo_slug(repo)?;
        let path = format!("repos/{owner}/{name}/pulls/{number}/reviews");
        let body_str = serde_json::to_string(&payload)?;
        self.run_stdin(repo, &["api", "--method", "POST", &path, "--input", "-"], &body_str)?;
        Ok(())
    }

    /// Post a reply into an existing review thread. `in_reply_to` is a comment's
    /// `database_id` (the REST integer id), not the GraphQL node id. The FE
    /// refetches threads after, so the created comment isn't returned.
    pub fn reply_review_comment(
        &self,
        repo: &Path,
        number: u64,
        in_reply_to: u64,
        body: &str,
    ) -> Result<()> {
        let (owner, name) = self.repo_slug(repo)?;
        let path = format!("repos/{owner}/{name}/pulls/{number}/comments");
        let body_arg = format!("body={body}");
        let reply_arg = format!("in_reply_to={in_reply_to}");
        self.run_ok(repo, &["api", "--method", "POST", &path, "-f", &body_arg, "-F", &reply_arg])?;
        Ok(())
    }

    /// Mark a review thread resolved. `thread_id` is the GraphQL node id
    /// (`ReviewThread.id`), not a database id.
    pub fn resolve_review_thread(&self, repo: &Path, thread_id: &str) -> Result<()> {
        self.set_thread_resolved(repo, thread_id, true)
    }

    /// Reopen a resolved review thread.
    pub fn unresolve_review_thread(&self, repo: &Path, thread_id: &str) -> Result<()> {
        self.set_thread_resolved(repo, thread_id, false)
    }

    fn set_thread_resolved(&self, repo: &Path, thread_id: &str, resolve: bool) -> Result<()> {
        let mutation = if resolve { "resolveReviewThread" } else { "unresolveReviewThread" };
        let query = format!(
            "mutation($threadId:ID!){{ {mutation}(input:{{threadId:$threadId}}){{ thread{{ id isResolved }} }} }}"
        );
        let thread_arg = format!("threadId={thread_id}");
        let query_arg = format!("query={query}");
        let out = self.run(repo, &["api", "graphql", "-f", &thread_arg, "-f", &query_arg])?;
        if !out.status.success() {
            bail!("gh api graphql ({mutation}) failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(())
    }
}

/// GraphQL query for a PR's review threads. `$number` is bound with `-F` so it
/// arrives typed as `Int!`.
const THREADS_QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      reviewThreads(first:100){
        nodes{
          id isResolved isOutdated path line diffSide
          comments(first:100){
            nodes{
              id databaseId path line originalLine diffHunk body createdAt
              author{login}
              replyTo{id}
            }
          }
        }
      }
    }
  }
}"#;

// Private envelope for the reviewThreads GraphQL response. GraphQL emits
// camelCase keys, so `rename_all = "camelCase"` maps them onto snake_case Rust
// fields; these get flattened into the public `ReviewThread`/`PrReviewComment`.
#[derive(Deserialize)]
struct GqlThreadsResp {
    data: GqlThreadsData,
}
#[derive(Deserialize)]
struct GqlThreadsData {
    repository: Option<GqlRepository>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRepository {
    pull_request: Option<GqlPullRequest>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPullRequest {
    review_threads: GqlThreadNodes,
}
#[derive(Deserialize)]
struct GqlThreadNodes {
    nodes: Vec<GqlThread>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlThread {
    id: String,
    is_resolved: bool,
    #[serde(default)]
    is_outdated: bool,
    path: Option<String>,
    line: Option<u64>,
    diff_side: Option<String>,
    comments: GqlCommentNodes,
}
#[derive(Deserialize)]
struct GqlCommentNodes {
    nodes: Vec<GqlComment>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlComment {
    id: String,
    database_id: u64,
    path: Option<String>,
    line: Option<u64>,
    original_line: Option<u64>,
    diff_hunk: Option<String>,
    body: Option<String>,
    created_at: Option<String>,
    author: Option<Author>,
    reply_to: Option<GqlNodeRef>,
}
#[derive(Deserialize)]
struct GqlNodeRef {
    id: String,
}

/// Split `gh pr diff` output into per-file diffs. A `diff --git a/X b/Y` line
/// starts each file; the remainder of the section is parsed by the same routine
/// the local diff renderer uses (`crate::git::parse_diff`), so each hunk keeps
/// its literal `@@` header for the frontend to derive line numbers from. Pure —
/// unit-tested without gh.
pub fn parse_pr_diff(diff: &str) -> Vec<PrFileDiff> {
    let mut files = Vec::new();
    let mut section: Vec<&str> = Vec::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") && !section.is_empty() {
            if let Some(f) = build_pr_file(&section) {
                files.push(f);
            }
            section.clear();
        }
        section.push(line);
    }
    if let Some(f) = build_pr_file(&section) {
        files.push(f);
    }
    files
}

fn build_pr_file(section: &[&str]) -> Option<PrFileDiff> {
    if section.is_empty() {
        return None;
    }
    let text = section.join("\n");
    let fd = crate::git::parse_diff(&text);

    let mut new_path: Option<String> = None;
    let mut old_path: Option<String> = None;
    let mut binary = false;
    for line in section {
        if let Some(p) = line.strip_prefix("+++ ") {
            new_path = normalize_diff_path(p);
        } else if let Some(p) = line.strip_prefix("--- ") {
            old_path = normalize_diff_path(p);
        } else if line.starts_with("Binary files ") {
            binary = true;
        }
    }
    // Renames and binary files have no ---/+++ body lines; fall back to the
    // `diff --git a/X b/Y` line for the paths.
    if new_path.is_none() || old_path.is_none() {
        if let Some((a, b)) = section.first().and_then(|l| parse_diff_git_line(l)) {
            old_path = old_path.or(Some(a));
            new_path = new_path.or(Some(b));
        }
    }

    let path = new_path.clone().or_else(|| old_path.clone()).unwrap_or_default();
    // old_path is only meaningful when it differs from the new path (a rename);
    // for a plain add/modify it's noise.
    let old = match old_path {
        Some(o) if Some(&o) != new_path.as_ref() => Some(o),
        _ => None,
    };
    Some(PrFileDiff { path, old_path: old, binary, header: fd.header, hunks: fd.hunks })
}

/// Strip a diff path's `a/`/`b/` prefix; `/dev/null` (add or delete) → None.
fn normalize_diff_path(p: &str) -> Option<String> {
    let p = p.trim();
    if p == "/dev/null" {
        return None;
    }
    let p = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(p);
    Some(p.to_string())
}

/// Parse the paths out of a `diff --git a/X b/Y` line. Splits on the ` b/`
/// separator; good enough for the common (unquoted, spaceless) case.
fn parse_diff_git_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    let idx = rest.find(" b/")?;
    let a = rest[..idx].strip_prefix("a/").unwrap_or(&rest[..idx]).to_string();
    let b = rest[idx + 1..].strip_prefix("b/").unwrap_or(&rest[idx + 1..]).to_string();
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable fake `gh` whose behavior is the given shell body.
    fn fake_gh(dir: &Path, body: &str) -> String {
        let path = dir.join("gh");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh\n{body}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn readiness_not_installed_when_binary_missing() {
        let gh = GhCli::with_bin("/nonexistent/gh-definitely-missing");
        assert_eq!(gh.readiness(Path::new("/tmp")), GhReadiness::NotInstalled);
    }

    #[test]
    fn readiness_not_authenticated_when_auth_status_fails() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(dir.path(), r#"case "$1" in --version) exit 0;; auth) exit 1;; esac"#);
        assert_eq!(GhCli::with_bin(bin).readiness(dir.path()), GhReadiness::NotAuthenticated);
    }

    #[test]
    fn readiness_ready_when_all_probes_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(dir.path(), "exit 0");
        assert_eq!(GhCli::with_bin(bin).readiness(dir.path()), GhReadiness::Ready);
    }

    #[test]
    fn view_pr_parses_json_and_maps_no_pr_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"
if [ "$3" = "agent/has-pr" ]; then
  echo '{"number":7,"url":"https://github.com/o/r/pull/7","title":"T","state":"OPEN","isDraft":false,"baseRefName":"main","headRefName":"agent/has-pr"}'
  exit 0
fi
echo "no pull requests found for branch" >&2
exit 1
"#,
        );
        let gh = GhCli::with_bin(bin);
        let pr = gh.view_pr(dir.path(), "agent/has-pr").unwrap().unwrap();
        assert_eq!(pr.number, 7);
        assert_eq!(pr.base_ref_name, "main");
        assert!(gh.view_pr(dir.path(), "agent/none").unwrap().is_none());
    }

    #[test]
    fn pr_checks_parses_stdout_despite_failing_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"
echo '[{"name":"build","bucket":"fail","link":"https://ci/1","description":"boom"},{"name":"lint","bucket":"pass","link":"","description":""}]'
exit 1
"#,
        );
        let checks = GhCli::with_bin(bin).pr_checks(dir.path(), "agent/x").unwrap();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].bucket, "fail");
        assert_eq!(checks[1].name, "lint");
    }

    #[test]
    fn issue_and_pr_lists_parse() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"
case "$1 $2" in
  "issue list") echo '[{"number":12,"title":"Fix login"}]';;
  "issue view") echo '{"number":12,"title":"Fix login","body":"It breaks","url":"https://github.com/o/r/issues/12"}';;
  "pr list") echo '[{"number":3,"url":"u","title":"T","state":"OPEN","isDraft":false,"baseRefName":"main","headRefName":"feat/x"}]';;
esac
"#,
        );
        let gh = GhCli::with_bin(bin);
        let issues = gh.list_issues(dir.path()).unwrap();
        assert_eq!(issues[0].number, 12);
        let detail = gh.view_issue(dir.path(), 12).unwrap();
        assert_eq!(detail.body, "It breaks");
        let prs = gh.list_prs(dir.path()).unwrap();
        assert_eq!(prs[0].head_ref_name, "feat/x");
    }

    #[test]
    fn pr_checks_maps_no_checks_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let bin =
            fake_gh(dir.path(), r#"echo "no checks reported on the 'agent/x' branch" >&2; exit 1"#);
        assert!(GhCli::with_bin(bin).pr_checks(dir.path(), "agent/x").unwrap().is_empty());
    }

    #[test]
    fn parse_pr_diff_splits_files_and_marks_binary() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,3 @@
 ctx
-old
+new
+add
diff --git a/newf.txt b/newf.txt
new file mode 100644
index 000..333
--- /dev/null
+++ b/newf.txt
@@ -0,0 +1 @@
+hello
diff --git a/img.png b/img.png
index 444..555 100644
Binary files a/img.png and b/img.png differ
";
        let files = parse_pr_diff(diff);
        assert_eq!(files.len(), 3);

        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].old_path, None); // modify: old == new
        assert!(!files[0].binary);
        assert_eq!(files[0].hunks.len(), 1);
        assert!(files[0].header.contains("diff --git a/src/a.rs"));
        assert!(files[0].hunks[0].header.starts_with("@@ -1,2 +1,3 @@"));

        assert_eq!(files[1].path, "newf.txt");
        assert_eq!(files[1].old_path, None); // addition: --- /dev/null
        assert_eq!(files[1].hunks.len(), 1);

        assert_eq!(files[2].path, "img.png");
        assert!(files[2].binary);
        assert!(files[2].hunks.is_empty());
    }

    #[test]
    fn parse_pr_diff_records_rename_old_path() {
        let diff = "\
diff --git a/old/name.rs b/new/name.rs
similarity index 100%
rename from old/name.rs
rename to new/name.rs
";
        let files = parse_pr_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new/name.rs");
        assert_eq!(files[0].old_path.as_deref(), Some("old/name.rs"));
    }

    #[test]
    fn view_pr_detail_parses_body_sha_and_null_review_decision() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"echo '{"number":7,"url":"u","title":"T","state":"OPEN","isDraft":false,"baseRefName":"main","headRefName":"feat/x","body":"desc","headRefOid":"abc123","mergeable":"MERGEABLE","reviewDecision":null,"author":{"login":"bob"},"createdAt":"t0","updatedAt":"t1"}'"#,
        );
        let pr = GhCli::with_bin(bin).view_pr_detail(dir.path(), 7).unwrap().unwrap();
        assert_eq!(pr.head_ref_oid, "abc123");
        assert_eq!(pr.body, "desc");
        assert_eq!(pr.author.login, "bob");
        assert!(pr.review_decision.is_none());
    }

    #[test]
    fn pr_review_threads_flattens_graphql_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"
case "$1 $2" in
  "repo view") echo "o/r";;
  "api graphql") echo '{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[{"id":"T1","isResolved":false,"isOutdated":true,"path":"a.rs","line":42,"diffSide":"LEFT","comments":{"nodes":[{"id":"C1","databaseId":555,"path":"a.rs","line":42,"originalLine":40,"diffHunk":"@@ -1 +1 @@","body":"hi","createdAt":"t","author":{"login":"bob"},"replyTo":null}]}}]}}}}}';;
esac
"#,
        );
        let threads = GhCli::with_bin(bin).pr_review_threads(dir.path(), 7).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "T1");
        assert!(!threads[0].is_resolved);
        assert!(threads[0].is_outdated);
        assert_eq!(threads[0].path, "a.rs");
        assert_eq!(threads[0].line, Some(42));
        assert_eq!(threads[0].diff_side, "LEFT");
        let c = &threads[0].comments[0];
        assert_eq!(c.database_id, 555);
        assert_eq!(c.author, "bob");
        assert_eq!(c.line, Some(42));
        assert!(c.in_reply_to_id.is_none());
    }

    #[test]
    fn submit_pr_review_pipes_expected_body() {
        let dir = tempfile::tempdir().unwrap();
        let cap = dir.path().join("review.json");
        // repo_slug → "o/r"; the reviews POST captures its stdin to a file so we
        // can assert the serialized payload.
        let body = format!(
            r#"
case "$1 $2" in
  "repo view") echo "o/r";;
  "api --method") cat > "{cap}"; echo '{{}}';;
esac
"#,
            cap = cap.display()
        );
        let bin = fake_gh(dir.path(), &body);
        let comments = vec![
            DraftComment {
                path: "a.rs".into(),
                body: "single".into(),
                line: 42,
                side: "RIGHT".into(),
                start_line: None,
                start_side: None,
            },
            DraftComment {
                path: "b.rs".into(),
                body: "multi".into(),
                line: 14,
                side: "RIGHT".into(),
                start_line: Some(10),
                start_side: Some("RIGHT".into()),
            },
        ];
        GhCli::with_bin(bin)
            .submit_pr_review(
                dir.path(),
                7,
                "deadbeef",
                "REQUEST_CHANGES",
                Some("please fix"),
                &comments,
            )
            .unwrap();
        let sent = std::fs::read_to_string(&cap).unwrap();
        assert!(sent.contains(r#""commit_id":"deadbeef""#), "sent: {sent}");
        assert!(sent.contains(r#""event":"REQUEST_CHANGES""#));
        assert!(sent.contains(r#""body":"please fix""#));
        assert!(sent.contains(r#""line":42"#));
        assert!(sent.contains(r#""start_line":10"#));
        assert!(sent.contains(r#""start_side":"RIGHT""#));
        // The single-line comment must not carry a start_line.
        assert!(!sent.contains(r#""start_line":42"#));
    }

    #[test]
    fn submit_pr_review_rejects_empty_request_changes() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(dir.path(), r#"echo "o/r""#);
        let err = GhCli::with_bin(bin)
            .submit_pr_review(dir.path(), 7, "sha", "REQUEST_CHANGES", None, &[])
            .unwrap_err();
        assert!(err.to_string().contains("needs a summary"));
    }

    #[test]
    fn merge_methods_parses_repo_settings() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"echo '{"mergeCommitAllowed":true,"squashMergeAllowed":true,"rebaseMergeAllowed":false}'"#,
        );
        let m = GhCli::with_bin(bin).merge_methods(dir.path()).unwrap();
        assert!(m.merge && m.squash && !m.rebase);
    }

    #[test]
    fn merge_pr_builds_expected_args() {
        let dir = tempfile::tempdir().unwrap();
        let cap = dir.path().join("args.txt");
        // Log every gh invocation, and answer the two queries the cleanup makes.
        let bin = fake_gh(
            dir.path(),
            &format!(
                r#"echo "$@" >> "{}"
case "$1 $2" in
  "pr view") echo '{{"headRefName":"agent/agent-zek6","isCrossRepository":false}}' ;;
  "repo view") echo "o/r" ;;
esac"#,
                cap.display()
            ),
        );
        let res = GhCli::with_bin(bin).merge_pr(dir.path(), 3, "squash", true).unwrap();
        assert_eq!(res.branch_deleted.as_deref(), Some("agent/agent-zek6"));
        assert_eq!(res.warning, None);
        let args = std::fs::read_to_string(&cap).unwrap();
        let lines: Vec<&str> = args.lines().collect();
        // No --delete-branch: it would also delete the local branch, which is
        // checked out in the task's worktree.
        assert_eq!(lines[0], "pr merge 3 --squash");
        assert_eq!(
            lines.last().copied(),
            Some("api -X DELETE repos/o/r/git/refs/heads/agent/agent-zek6")
        );
    }

    #[test]
    fn merge_pr_without_delete_branch_only_merges() {
        let dir = tempfile::tempdir().unwrap();
        let cap = dir.path().join("args.txt");
        let bin = fake_gh(dir.path(), &format!(r#"echo "$@" >> "{}""#, cap.display()));
        let res = GhCli::with_bin(bin).merge_pr(dir.path(), 3, "merge", false).unwrap();
        assert_eq!(res, PrMergeResult::default());
        assert_eq!(std::fs::read_to_string(&cap).unwrap().trim(), "pr merge 3 --merge");
    }

    #[test]
    fn merge_pr_reports_failed_branch_cleanup_as_warning() {
        let dir = tempfile::tempdir().unwrap();
        // The merge succeeds; every follow-up call fails. The merge must still
        // come back Ok so a landed PR is never shown as a failed merge.
        let bin = fake_gh(
            dir.path(),
            r#"if [ "$1 $2" = "pr merge" ]; then exit 0; fi
echo "boom" >&2; exit 1"#,
        );
        let res = GhCli::with_bin(bin).merge_pr(dir.path(), 3, "merge", true).unwrap();
        assert_eq!(res.branch_deleted, None);
        assert!(res.warning.unwrap().contains("could not be deleted"));
    }

    #[test]
    fn merge_pr_leaves_fork_branches_alone() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"case "$1 $2" in
  "pr view") echo '{"headRefName":"feature","isCrossRepository":true}' ;;
  "api -X") echo "should not delete a fork ref" >&2; exit 1 ;;
esac"#,
        );
        let res = GhCli::with_bin(bin).merge_pr(dir.path(), 3, "merge", true).unwrap();
        assert_eq!(res.branch_deleted, None);
        assert!(res.warning.unwrap().contains("fork"));
    }

    #[test]
    fn merge_pr_surfaces_merge_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(dir.path(), r#"echo "Pull request is not mergeable" >&2; exit 1"#);
        let err = GhCli::with_bin(bin).merge_pr(dir.path(), 3, "merge", true).unwrap_err();
        assert!(err.to_string().contains("not mergeable"));
    }

    #[test]
    fn resolve_review_thread_ok() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(
            dir.path(),
            r#"echo '{"data":{"resolveReviewThread":{"thread":{"id":"T1","isResolved":true}}}}'"#,
        );
        GhCli::with_bin(bin).resolve_review_thread(dir.path(), "T1").unwrap();
    }
}
