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

/// The prefix every Agency-cut branch carries. `state.rs` and the PR
/// association read it to tell our own branches from the user's, so a rename
/// keeps it whether or not the user typed it.
pub const AGENT_PREFIX: &str = "agent/";

/// Longest accepted branch name, prefix included. Git itself only stops at the
/// filesystem's path limit; this is about the name staying readable in a merge
/// commit, a PR title and a `git branch` listing. The derived names are ~50.
pub const MAX_LEN: usize = 100;

/// Characters git rejects outright in a ref name, plus the ones a shell or a
/// pathspec would eat.
const FORBIDDEN: &[char] = &['~', '^', ':', '?', '*', '[', '\\', ' ', '\t'];

/// Normalize a user-typed branch name to the full `agent/<leaf>` form,
/// rejecting anything git would not accept.
///
/// The `agent/` prefix is optional in the input: the rename dialog prefills the
/// whole current name, so the user usually edits the tail and leaves the prefix
/// in place, but typing just the tail means the same thing.
pub fn normalize(input: &str) -> Result<String> {
    let trimmed = input.trim();
    let leaf = trimmed.strip_prefix(AGENT_PREFIX).unwrap_or(trimmed);
    if leaf.is_empty() {
        bail!("a branch needs a name after '{AGENT_PREFIX}'");
    }
    if let Some(bad) = leaf.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        let shown = if bad == ' ' {
            "a space".to_string()
        } else if bad.is_control() {
            "a control character".to_string()
        } else {
            format!("'{bad}'")
        };
        bail!("git does not allow {shown} in a branch name");
    }
    if leaf.contains("..") {
        bail!("git does not allow '..' in a branch name");
    }
    if leaf.contains("@{") {
        bail!("git does not allow '@{{' in a branch name");
    }
    if leaf == "@" {
        bail!("git does not allow '@' on its own as a branch name");
    }
    if leaf.ends_with('/') || leaf.ends_with('.') {
        bail!("a branch name can't end with '{}'", if leaf.ends_with('/') { '/' } else { '.' });
    }
    for part in leaf.split('/') {
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
    let full = format!("{AGENT_PREFIX}{leaf}");
    if full.chars().count() > MAX_LEN {
        bail!("that branch name is {} characters; keep it to {MAX_LEN}", full.chars().count());
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(input: &str) -> String {
        normalize(input).unwrap_err().to_string()
    }

    #[test]
    fn adds_the_prefix_when_it_is_missing_and_keeps_it_when_it_is_there() {
        assert_eq!(normalize("tidy-branch").unwrap(), "agent/tidy-branch");
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
        assert!(err("agent/").contains("needs a name"));
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
        assert!(err("trailing/").contains("end with"));
        assert!(err("trailing.").contains("end with"));
        assert!(err("two//slashes").contains("empty part"));
        assert!(err(".hidden").contains("start with"));
        assert!(err("nested/.hidden").contains("start with"));
        assert!(err("name.lock").contains(".lock"));
        assert!(err("nested/name.lock/x").contains(".lock"));
    }

    #[test]
    fn rejects_an_overlong_name() {
        let long = "a".repeat(MAX_LEN);
        assert!(err(&long).contains(&MAX_LEN.to_string()));
        // Exactly at the cap, prefix included, is fine.
        let at_cap = "b".repeat(MAX_LEN - AGENT_PREFIX.len());
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
