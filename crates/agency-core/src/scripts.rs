use anyhow::{bail, Result};
use std::path::Path;
use std::process::Command;

/// Run a one-off lifecycle script (`sh -lc <script>`) to completion in `cwd`
/// with the given env. Returns an error if the script exits non-zero.
pub fn run_blocking(script: &str, cwd: &Path, env: &[(String, String)]) -> Result<()> {
    let mut cmd = Command::new("sh");
    cmd.arg("-lc").arg(script).current_dir(cwd);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status()?;
    if !status.success() {
        bail!("script exited with {status}");
    }
    Ok(())
}

/// Environment variables made available to every lifecycle script and to the
/// agent process. `port` is `None` until the port allocator lands (Plan 2).
pub fn script_env(
    workspace_path: &Path,
    root_path: &Path,
    workspace_name: &str,
    port: Option<u16>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("AGENCY_WORKSPACE_PATH".to_string(), workspace_path.to_string_lossy().to_string()),
        ("AGENCY_ROOT_PATH".to_string(), root_path.to_string_lossy().to_string()),
        ("AGENCY_WORKSPACE_NAME".to_string(), workspace_name.to_string()),
    ];
    if let Some(p) = port {
        env.push(("AGENCY_PORT".to_string(), p.to_string()));
    }
    env
}

/// If a non-blank `setup` script is configured, return a command that runs setup
/// first and only then `exec`s the agent (`sh -lc '<setup> && exec <agent>'`), so
/// the agent starts only when setup succeeds. Otherwise return the agent command
/// unchanged. The agent command and args are shell-quoted for safe embedding;
/// the setup string is passed through verbatim (it is the user's own shell line).
pub fn wrap_setup(setup: Option<&str>, command: &str, args: &[String]) -> (String, Vec<String>) {
    match setup {
        Some(s) if !s.trim().is_empty() => {
            let mut quoted = vec![shell_quote(command)];
            quoted.extend(args.iter().map(|a| shell_quote(a)));
            let agent = quoted.join(" ");
            let line = format!("{} && exec {}", s.trim(), agent);
            ("sh".to_string(), vec!["-lc".to_string(), line])
        }
        _ => (command.to_string(), args.to_vec()),
    }
}

/// Minimal POSIX shell single-quoting. Safe characters pass through unquoted;
/// everything else is wrapped in single quotes with embedded `'` escaped.
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn script_env_sets_workspace_vars_without_port() {
        let env = script_env(
            Path::new("/repo/.agency/worktrees/fix-login-a3k2"),
            Path::new("/repo"),
            "fix-login-a3k2",
            None,
        );
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("AGENCY_WORKSPACE_PATH").as_deref(), Some("/repo/.agency/worktrees/fix-login-a3k2"));
        assert_eq!(get("AGENCY_ROOT_PATH").as_deref(), Some("/repo"));
        assert_eq!(get("AGENCY_WORKSPACE_NAME").as_deref(), Some("fix-login-a3k2"));
        assert!(get("AGENCY_PORT").is_none());
    }

    #[test]
    fn script_env_includes_port_when_present() {
        let env = script_env(Path::new("/w"), Path::new("/r"), "x", Some(5210));
        let port = env.iter().find(|(n, _)| n == "AGENCY_PORT").map(|(_, v)| v.clone());
        assert_eq!(port.as_deref(), Some("5210"));
    }

    #[test]
    fn wrap_setup_none_returns_command_unchanged() {
        let (cmd, args) = wrap_setup(None, "claude", &["--print".to_string(), "hi there".to_string()]);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--print".to_string(), "hi there".to_string()]);
    }

    #[test]
    fn wrap_setup_blank_setup_returns_command_unchanged() {
        let (cmd, _) = wrap_setup(Some("   "), "claude", &[]);
        assert_eq!(cmd, "claude");
    }

    #[test]
    fn wrap_setup_chains_and_quotes() {
        let (cmd, args) = wrap_setup(
            Some("pnpm install"),
            "claude",
            &["--print".to_string(), "fix the bug".to_string()],
        );
        assert_eq!(cmd, "sh");
        assert_eq!(
            args,
            vec![
                "-lc".to_string(),
                "pnpm install && exec claude --print 'fix the bug'".to_string(),
            ]
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("plain"), "plain");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn run_blocking_runs_script_with_env_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        run_blocking(
            "echo $AGENCY_WORKSPACE_NAME > marker.txt",
            dir.path(),
            &[("AGENCY_WORKSPACE_NAME".to_string(), "fix-login".to_string())],
        )
        .unwrap();
        let body = std::fs::read_to_string(dir.path().join("marker.txt")).unwrap();
        assert_eq!(body.trim(), "fix-login");

        // Non-zero exit surfaces as an error.
        assert!(run_blocking("exit 3", dir.path(), &[]).is_err());
    }

    // keep PathBuf import used
    #[allow(dead_code)]
    fn _p() -> PathBuf { PathBuf::new() }
}
