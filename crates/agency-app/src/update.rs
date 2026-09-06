//! Passive update check: ask GitHub what the latest release is, compare it to
//! the running version, and let the UI offer a download link. Agency never
//! downloads or installs anything itself — the user goes to the releases page
//! and installs the DMG when it suits them. That keeps a background update from
//! ever restarting the app out from under running agents.
//!
//! This is the only outbound network call Agency's own code makes. It sends no
//! identifiers: a plain GET, a User-Agent naming the app, and nothing else. It
//! runs once per launch and can be turned off in Settings, which suppresses the
//! automatic check only; the manual button in Diagnostics still works.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// Repo the release check reads. Kept next to the "Report an issue" URL in the
/// UI — both have to move together if the repo does.
const RELEASES_API: &str = "https://api.github.com/repos/TennnisAI/Agency/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/TennnisAI/Agency/releases/latest";

/// Wall-clock cap on the whole request. The check is never on a path the user is
/// waiting for, so a slow network should give up quietly rather than hang.
const TIMEOUT_SECS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// Version of the running app, as the bundle reports it.
    pub current: String,
    /// Latest published release, with the leading `v` stripped. `None` when the
    /// check could not reach GitHub or found no releases.
    pub latest: Option<String>,
    /// True only when `latest` is a strictly newer release than `current`.
    pub update_available: bool,
    /// Where to send the user to get it.
    pub url: String,
    /// Why the check came back empty, for the UI to show verbatim. `None` on a
    /// successful check.
    pub error: Option<String>,
}

impl UpdateCheck {
    fn failed(current: &str, error: String) -> Self {
        UpdateCheck {
            current: current.to_string(),
            latest: None,
            update_available: false,
            url: RELEASES_PAGE.to_string(),
            error: Some(error),
        }
    }
}

/// Fetch the latest release tag and compare it to `current`.
///
/// Never returns `Err` for an unreachable network — being offline is an
/// expected state, not a failure worth surfacing as an error dialog, so it
/// comes back as an `UpdateCheck` with `error` set for the UI to show quietly.
pub fn check(current: &str) -> UpdateCheck {
    match fetch_latest_tag() {
        Ok(tag) => {
            let latest = tag.trim_start_matches('v').to_string();
            UpdateCheck {
                current: current.to_string(),
                update_available: agency_core::version::is_newer(&tag, current),
                latest: Some(latest),
                url: RELEASES_PAGE.to_string(),
                error: None,
            }
        }
        Err(e) => {
            log::info!("update check did not complete: {e:#}");
            UpdateCheck::failed(current, format!("{e}"))
        }
    }
}

/// GET the latest-release JSON and pull out `tag_name`.
///
/// Shells out to `curl` rather than linking an HTTP client: macOS always ships
/// one, this is the only request Agency ever makes, and it keeps the app free of
/// a TLS stack and the dependency surface that comes with it.
fn fetch_latest_tag() -> Result<String> {
    let out = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-time",
            &TIMEOUT_SECS.to_string(),
            // GitHub rejects API requests without a User-Agent.
            "--user-agent",
            "Agency-update-check",
            "--header",
            "Accept: application/vnd.github+json",
            RELEASES_API,
        ])
        .output()
        .context("running curl for the update check")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr.trim();
        // curl exit 22 is the --fail path: a non-2xx response, which for this
        // endpoint most often means the repo is private or has no releases yet.
        return Err(anyhow!(
            "could not reach the releases feed{}",
            if detail.is_empty() { String::new() } else { format!(": {detail}") }
        ));
    }

    let body: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing the release feed")?;
    body.get("tag_name")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .ok_or_else(|| anyhow!("the release feed had no tag_name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_check_reports_no_update_and_keeps_the_download_link() {
        let c = UpdateCheck::failed("0.1.0", "offline".to_string());
        assert_eq!(c.current, "0.1.0");
        assert_eq!(c.latest, None);
        assert!(!c.update_available, "an unreachable feed must never claim an update");
        assert_eq!(c.error.as_deref(), Some("offline"));
        assert_eq!(c.url, RELEASES_PAGE);
    }

    #[test]
    fn release_page_and_api_point_at_the_same_repo() {
        // The two URLs are written out separately; this catches one being
        // repointed at a new repo without the other.
        let repo = "TennnisAI/Agency";
        assert!(RELEASES_API.contains(repo));
        assert!(RELEASES_PAGE.contains(repo));
    }
}
