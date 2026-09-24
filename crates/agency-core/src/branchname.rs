//! Pure validation for the branch name a run works on.
//!
//! A run's branch is derived from its prompt at dispatch (`agent/<task id>`),
//! and a merge writes that name into the base branch's history for good:
//! `Merge branch 'agent/work-on-issue-age-139-…-https-github-com-…'`.
//! A merged PR's head branch cannot be renamed afterwards either. So the name
//! has to be editable while the run is still local, and every name the user
//! types has to be one git will actually accept as a ref, checked before
//! anything is renamed rather than after `git branch -m` has half-done it.
//!
//! The rules below are `git check-ref-format`'s, applied here rather than by
//! shelling out to it: this is the one place the answer is needed, the rules
//! do not change, and a pure function is testable without a repo.

use anyhow::{bail, Result};

/// Longest accepted branch name. Git itself only stops at the filesystem's
/// path limit; this is about the name staying readable in a merge commit, a PR
/// title and a `git branch` listing. The derived names are ~50.
pub const MAX_LEN: usize = 100;

/// Characters git rejects outright in a ref name, plus the ones a shell or a
/// pathspec would eat.
const FORBIDDEN: &[char] = &['~', '^', ':', '?', '*', '[', '\\', ' ', '\t'];

/// Validate a user-typed branch name and return it trimmed, exactly as typed
/// otherwise.
///
/// AGE-238: this used to prepend `agent/` to anything that lacked it, so
/// renaming to `users/nick/branch1` produced `agent/users/nick/branch1`, which
/// a remote enforcing a `users/<name>/…` naming rule then rejected on push. The
/// user's name is the user's: nothing reads the prefix to recognize a run's
/// branch (the registry row does that), so there is no reason to impose it.
pub fn normalize(input: &str) -> Result<String> {
    let name = input.trim();
    if name.is_empty() {
        bail!("a branch needs a name");
    }
    if let Some(bad) = name.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        let shown = if bad == ' ' {
            "a space".to_string()
        } else if bad.is_control() {
            "a control character".to_string()
        } else {
            format!("'{bad}'")
        };
        bail!("git does not allow {shown} in a branch name");
    }
    if name.contains("..") {
        bail!("git does not allow '..' in a branch name");
    }
    if name.contains("@{") {
        bail!("git does not allow '@{{' in a branch name");
    }
    if name == "@" {
        bail!("git does not allow '@' on its own as a branch name");
    }
    // `git branch` refuses both, and a leading '-' would reach `git branch -m`
    // as an option rather than a name.
    if name.starts_with('-') {
        bail!("a branch name can't start with '-'");
    }
    if name == "HEAD" {
        bail!("git does not allow 'HEAD' as a branch name");
    }
    if name.ends_with('/') || name.ends_with('.') {
        bail!("a branch name can't end with '{}'", if name.ends_with('/') { '/' } else { '.' });
    }
    for part in name.split('/') {
        if part.is_empty() {
            bail!("a branch name can't have an empty part (two slashes in a row)");
        }
        if part.starts_with('.') {
            bail!("no part of a branch name can start with '.'");
        }
        if part.ends_with(".lock") {
            bail!("no part of a branch name can end with '.lock'");
        }
    }
    if name.chars().count() > MAX_LEN {
        bail!("that branch name is {} characters; keep it to {MAX_LEN}", name.chars().count());
    }
    Ok(name.to_string())
}

/// The four-character disambiguator a run id ends with (`fix-login-a3k2`),
/// which a branch Agency cut for the run keeps through a first-prompt rename
/// (`agent/<leaf>-a3k2`). One definition for the app that writes the suffix
/// and the registry that reads a branch's shape by it: two copies of the rule
/// would drift, and the registry's would then delete by a stale one.
pub fn id_suffix(run_id: &str) -> Option<&str> {
    let (_, suf) = run_id.rsplit_once('-')?;
    if suf.len() == 4 && suf.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(suf)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_suffix_reads_the_four_char_disambiguator() {
        assert_eq!(id_suffix("add-a-login-page-a3k2"), Some("a3k2"));
        assert_eq!(id_suffix("agent-36a2"), Some("36a2"));
        // No hyphen at all, and a trailing segment that is not the 4-char shape.
        assert_eq!(id_suffix("nope"), None);
        assert_eq!(id_suffix("add-a-login-page-toolong"), None);
    }

    fn err(input: &str) -> String {
        normalize(input).unwrap_err().to_string()
    }

    /// AGE-238: a name outside `agent/` is kept as typed, not prefixed.
    #[test]
    fn keeps_the_name_as_typed_without_adding_a_prefix() {
        assert_eq!(normalize("users/nick/branch1").unwrap(), "users/nick/branch1");
        assert_eq!(normalize("tidy-branch").unwrap(), "tidy-branch");
        assert_eq!(normalize("agent/tidy-branch").unwrap(), "agent/tidy-branch");
        assert_eq!(normalize("  agent/tidy-branch \n").unwrap(), "agent/tidy-branch");
    }

    #[test]
    fn keeps_a_nested_name() {
        assert_eq!(normalize("agent/nick/fix-login").unwrap(), "agent/nick/fix-login");
    }

    #[test]
    fn rejects_an_empty_name() {
        assert!(err("").contains("needs a name"));
        assert!(err("   ").contains("needs a name"));
    }

    #[test]
    fn rejects_what_git_rejects() {
        // One case per check-ref-format rule, so a lost rule fails a test.
        assert!(err("a b").contains("a space"));
        assert!(err("a~b").contains("'~'"));
        assert!(err("a^b").contains("'^'"));
        assert!(err("a:b").contains("':'"));
        assert!(err("a?b").contains("'?'"));
        assert!(err("a*b").contains("'*'"));
        assert!(err("a[b").contains("'['"));
        assert!(err("a\\b").contains('\\'));
        assert!(err("a\u{7}b").contains("control character"));
        assert!(err("a..b").contains(".."));
        assert!(err("a@{b").contains("@{"));
        assert!(err("@").contains("on its own"));
        assert!(err("-oops").contains("start with '-'"));
        assert!(err("HEAD").contains("HEAD"));
        assert!(err("trailing/").contains("end with"));
        assert!(err("trailing.").contains("end with"));
        assert!(err("two//slashes").contains("empty part"));
        assert!(err("/leading").contains("empty part"));
        assert!(err(".hidden").contains("start with"));
        assert!(err("nested/.hidden").contains("start with"));
        assert!(err("name.lock").contains(".lock"));
        assert!(err("nested/name.lock/x").contains(".lock"));
    }

    #[test]
    fn rejects_an_overlong_name() {
        let long = "a".repeat(MAX_LEN + 1);
        assert!(err(&long).contains(&MAX_LEN.to_string()));
        let at_cap = "b".repeat(MAX_LEN);
        assert_eq!(normalize(&at_cap).unwrap().chars().count(), MAX_LEN);
    }

    #[test]
    fn accepts_the_shape_agency_derives() {
        assert_eq!(
            normalize("agent/work-on-issue-age-148-let-the-branch-nam-39iz").unwrap(),
            "agent/work-on-issue-age-148-let-the-branch-nam-39iz",
        );
    }
}
