//! Signing in again when an HTTPS remote turns down the stored credential.
//!
//! An expired token made Sync fail with "fatal: could not read Password for
//! 'https://…': terminal prompts disabled" in the panel's error banner, which
//! names the problem and offers no way to fix it from the app. git has no TTY
//! here (`GIT_TERMINAL_PROMPT=0`, so it can't hang on a prompt nobody sees), so
//! it can't ask for the new one itself. The panel asks instead, and the answer
//! lands here.
//!
//! The credential is handed to git through a credential helper that reads it
//! from the environment, never through argv, which any process can list. The
//! helper list is reset first so ours is the only one consulted: a helper
//! further up the chain still holding the expired token (a `gh` helper whose
//! `store` is a no-op is the usual one) would otherwise answer first and fail
//! the same way again. It is kept for the life of the process, and stored with
//! the user's own helpers too, so the next launch finds it where git looks.

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// What the sign-in prompt is for: the remote's scheme and host, and the
/// username git already knows, if the remote URL carries one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthTarget {
    /// `https://github.com`: what the credential is good for, and what the
    /// prompt names.
    pub base: String,
    pub username: Option<String>,
}

#[derive(Clone)]
struct Credential {
    username: String,
    password: String,
}

/// Credentials signed in this session, keyed by [`AuthTarget::base`].
static SESSION: Mutex<Option<HashMap<String, Credential>>> = Mutex::new(None);

const USER_VAR: &str = "AGENCY_GIT_USERNAME";
const PASS_VAR: &str = "AGENCY_GIT_PASSWORD";

/// The helper git runs. `!` makes it a shell snippet; it answers `get` and
/// ignores `store` and `erase`, which are the user's helpers' business.
const HELPER: &str = "!f() { test \"$1\" = get && printf 'username=%s\\npassword=%s\\n' \
     \"$AGENCY_GIT_USERNAME\" \"$AGENCY_GIT_PASSWORD\"; }; f";

/// The scheme-and-host part of an HTTP(S) remote URL, and the username it
/// embeds. `None` for SSH and local remotes: a password prompt can't help
/// those, so their failures stay in the banner as they are.
pub fn target_of(url: &str) -> Option<AuthTarget> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (userinfo, host) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    if host.is_empty() {
        return None;
    }
    // `user:password@host` carries a secret we will not echo back; only the
    // name is a prefill.
    let username = userinfo
        .map(|u| u.split(':').next().unwrap_or(""))
        .filter(|u| !u.is_empty())
        .map(str::to_string);
    Some(AuthTarget { base: format!("{scheme}://{}", host.to_ascii_lowercase()), username })
}

/// True when git failed because the remote wants credentials it didn't get or
/// turned down the ones it did. With prompts disabled an expired token reads
/// "could not read Password for 'https://…': terminal prompts disabled".
pub fn is_auth_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("could not read username")
        || s.contains("could not read password")
        || s.contains("terminal prompts disabled")
        || s.contains("authentication failed")
        || s.contains("invalid username or password")
}

/// The environment that makes git use `username`/`password` and nothing else.
/// `GIT_CONFIG_COUNT` config is read after every file, so the empty
/// `credential.helper` clears the helpers the user's own config listed.
fn helper_env(username: &str, password: &str) -> Vec<(String, String)> {
    [
        ("GIT_CONFIG_COUNT", "2"),
        ("GIT_CONFIG_KEY_0", "credential.helper"),
        ("GIT_CONFIG_VALUE_0", ""),
        ("GIT_CONFIG_KEY_1", "credential.helper"),
        ("GIT_CONFIG_VALUE_1", HELPER),
        (USER_VAR, username),
        (PASS_VAR, password),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Reject a username or password git's credential protocol can't carry: it is
/// line-based, so a newline would end the value early, and a NUL ends it in C.
fn check_field(what: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{what} is empty");
    }
    if value.contains(['\n', '\r', '\0']) {
        bail!("{what} can't contain a line break");
    }
    Ok(())
}

/// The push URL of `origin` in `dir`, which is where Sync and Push send their
/// credentials.
fn origin_url(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "--push", "origin"])
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// What a sign-in prompt for `dir`'s `origin` should ask for, or `None` when
/// origin isn't an HTTP(S) remote.
pub fn target(dir: &Path) -> Option<AuthTarget> {
    target_of(&origin_url(dir)?)
}

/// The environment a network git command in `dir` runs with: the credential
/// signed in this session for its origin, if any, else nothing. Costs no git
/// call until someone has signed in.
pub fn env_for(dir: &Path) -> Vec<(String, String)> {
    if SESSION.lock().unwrap().as_ref().map_or(true, HashMap::is_empty) {
        return Vec::new();
    }
    let Some(target) = target(dir) else { return Vec::new() };
    let cred = SESSION.lock().unwrap().as_ref().and_then(|m| m.get(&target.base).cloned());
    cred.map(|c| helper_env(&c.username, &c.password)).unwrap_or_default()
}

/// Sign in to `dir`'s origin. The credential is tried against the remote first
/// (`git ls-remote`), so a mistyped token is reported here, in the prompt,
/// rather than as the same banner the user was trying to clear. Once it works
/// it is kept for this session and handed to the user's own credential helpers
/// (`git credential approve`), which is what git does after any successful
/// prompt; Agency writes no config to make that happen.
pub fn sign_in(dir: &Path, username: &str, password: &str) -> Result<()> {
    check_field("The username", username)?;
    check_field("The password", password)?;
    let Some(target) = target(dir) else {
        bail!("origin isn't an HTTPS remote, so there's no password to update");
    };
    let out = std::process::Command::new("git")
        .args(["ls-remote", "--heads", "origin"])
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(helper_env(username, password))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if is_auth_failure(&stderr) {
            bail!("{} didn't accept that username and password", target.base);
        }
        bail!("couldn't reach origin: {}", stderr.trim());
    }
    SESSION.lock().unwrap().get_or_insert_with(HashMap::new).insert(
        target.base.clone(),
        Credential { username: username.to_string(), password: password.to_string() },
    );
    // Best effort: with no helper configured there is nowhere to keep it, and
    // the session copy above still covers every retry until Agency quits.
    let input = format!("url={}\nusername={username}\npassword={password}\n\n", target.base);
    let _ = crate::git::git_with(dir, &["credential", "approve"], &[], Some(input.as_bytes()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_remotes_name_their_host_and_user() {
        assert_eq!(
            target_of("https://github.com/acme/app.git"),
            Some(AuthTarget { base: "https://github.com".into(), username: None })
        );
        assert_eq!(
            target_of("https://nic@GitHub.com/acme/app.git"),
            Some(AuthTarget { base: "https://github.com".into(), username: Some("nic".into()) })
        );
        assert_eq!(
            target_of("http://git.local:8080/x"),
            Some(AuthTarget { base: "http://git.local:8080".into(), username: None })
        );
    }

    #[test]
    fn an_embedded_password_is_never_the_prefill() {
        let t = target_of("https://nic:ghp_secret@github.com/acme/app.git").unwrap();
        assert_eq!(t.username.as_deref(), Some("nic"));
        assert!(!t.base.contains("secret"));
    }

    #[test]
    fn ssh_and_local_remotes_get_no_prompt() {
        assert_eq!(target_of("git@github.com:acme/app.git"), None);
        assert_eq!(target_of("ssh://git@github.com/acme/app.git"), None);
        assert_eq!(target_of("/srv/git/app.git"), None);
        assert_eq!(target_of("file:///srv/git/app.git"), None);
        assert_eq!(target_of("https:///nohost"), None);
    }

    #[test]
    fn recognises_an_expired_token() {
        assert!(is_auth_failure(
            "git push failed:\nfatal: could not read Password for 'https://nic@github.com': \
             terminal prompts disabled"
        ));
        assert!(is_auth_failure("fatal: Authentication failed for 'https://github.com/a/b.git/'"));
        assert!(!is_auth_failure("! [rejected] main -> main (fetch first)"));
        assert!(!is_auth_failure("fatal: unable to access: Could not resolve host: github.com"));
    }

    #[test]
    fn the_secret_travels_in_the_environment_only() {
        let env = helper_env("nic", "tok");
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        // The helper list is cleared before ours is added.
        assert_eq!(get("GIT_CONFIG_VALUE_0"), Some(""));
        assert!(!get("GIT_CONFIG_VALUE_1").unwrap().contains("tok"));
        assert_eq!(get(PASS_VAR), Some("tok"));
    }

    #[test]
    fn git_takes_ours_over_a_stale_helper_in_the_users_config() {
        // The case this exists for: a helper in the user's config still answers
        // with the expired token, and git would use it first.
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("gitconfig");
        std::fs::write(
            &global,
            "[credential]\n\thelper = \"!f() { echo username=old; echo password=expired; }; f\"\n",
        )
        .unwrap();
        let mut child = std::process::Command::new("git")
            .args(["credential", "fill"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .envs(helper_env("nic", "fresh token"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(b"url=https://github.com\n\n").unwrap();
        let out = child.wait_with_output().unwrap();
        let out = String::from_utf8_lossy(&out.stdout);
        assert!(out.contains("username=nic\n"), "{out}");
        assert!(out.contains("password=fresh token\n"), "{out}");
    }

    #[test]
    fn rejects_values_the_credential_protocol_cannot_carry() {
        assert!(check_field("x", "").is_err());
        assert!(check_field("x", "a\nb").is_err());
        assert!(check_field("x", "ghp_ok").is_ok());
    }
}
