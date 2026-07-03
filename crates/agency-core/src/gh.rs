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
use std::path::Path;
use std::process::Command;

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
        Command::new(&self.bin).args(args).current_dir(repo).output()
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

    /// The open or merged PR for `branch`, if any. gh's "no pull requests
    /// found" error is a normal answer here, not a failure.
    pub fn view_pr(&self, repo: &Path, branch: &str) -> Result<Option<PrInfo>> {
        let out = self.run(
            repo,
            &[
                "pr", "view", branch, "--json",
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
            &[
                "pr", "create", "--head", branch, "--base", base, "--title", title, "--body", body,
            ],
        )?;
        self.view_pr(repo, branch)?
            .ok_or_else(|| anyhow::anyhow!("PR was created but could not be read back"))
    }

    /// CI/check rollup for the branch's PR. `gh pr checks` exits non-zero when
    /// checks are pending or failing, so the JSON on stdout is authoritative
    /// and the exit code is ignored when it parses. A PR with no checks
    /// configured reports as an empty list.
    pub fn pr_checks(&self, repo: &Path, branch: &str) -> Result<Vec<CheckItem>> {
        let out = self.run(
            repo,
            &["pr", "checks", branch, "--json", "name,bucket,link,description"],
        )?;
        if let Ok(items) = serde_json::from_slice::<Vec<CheckItem>>(&out.stdout) {
            return Ok(items);
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.to_lowercase().contains("no checks reported") {
            return Ok(Vec::new());
        }
        bail!("gh pr checks failed: {err}");
    }
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
        let bin = fake_gh(
            dir.path(),
            r#"case "$1" in --version) exit 0;; auth) exit 1;; esac"#,
        );
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
    fn pr_checks_maps_no_checks_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_gh(dir.path(), r#"echo "no checks reported on the 'agent/x' branch" >&2; exit 1"#);
        assert!(GhCli::with_bin(bin).pr_checks(dir.path(), "agent/x").unwrap().is_empty());
    }
}
