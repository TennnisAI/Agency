//! The markdown a run leaves behind when its workspace goes.
//!
//! Archiving used to keep a database row and a branch. The row is one line of
//! prompt text in a collapsed list, and once the branch is deleted too — which
//! is now the normal ending for merged work — an archived agent would be a
//! name and nothing else. So the archive keeps a file instead: what the agent
//! was asked, what it committed, how the work ended, what it cost.
//!
//! It is markdown in the project's own `.agency/records/`, for the same reason
//! the issue tracker is: a record the user can grep, read in any editor and
//! keep after Agency is gone is worth more than a row only Agency can render.
//!
//! Pure: a struct in, a string out. The gathering (git log, diffstat, usage)
//! and the writing live in the app.

use crate::issuefs::epoch_to_rfc3339;
use std::path::{Path, PathBuf};

/// Where a project keeps the records of its finished runs. Beside the issue
/// tracker, and git-excluded by the same list (`worktree::ensure_agency_excludes`).
pub fn dir(repo: &Path) -> PathBuf {
    repo.join(".agency").join("records")
}

/// The record file for one run.
pub fn path(repo: &Path, run_id: &str) -> PathBuf {
    dir(repo).join(format!("{run_id}.md"))
}

/// How a run's work ended, as git could see it at teardown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every commit is on the base branch.
    Merged { base: String },
    /// Every commit is on a remote, but not on the base.
    Pushed,
    /// Commits exist only on the run's own branch, which was kept.
    Kept { commits: usize },
    /// Commits existed only on the run's own branch, and it was deleted.
    Dropped { commits: usize },
    /// The agent committed nothing.
    Empty,
    /// The run worked in the project's own checkout, so it had no worktree and
    /// no branch of its own and nothing of its was removed.
    NoWorkspace { branch: String },
}

/// One commit, as `git log` gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub short: String,
    pub subject: String,
}

/// Everything known about a finished run at the moment its workspace goes.
/// Optional fields are the ones we genuinely may not know; each one prints as
/// nothing rather than as a zero, on the same principle as `usage.rs` — "we
/// cannot see this" and "this was none" are different claims.
#[derive(Debug, Clone, PartialEq)]
pub struct RunRecord {
    pub id: String,
    /// The run's title if it earned one, else its branch.
    pub heading: String,
    pub agent: String,
    pub model: Option<String>,
    /// The prompt it was dispatched with. Empty for a run started blank.
    pub prompt: String,
    pub branch: String,
    pub base: String,
    /// `AGE-149 — I want to tidy up our workflow`, when it came from an issue.
    pub issue: Option<String>,
    pub created_at: i64,
    pub archived_at: i64,
    pub outcome: Outcome,
    /// Newest first, as `git log` orders them. Empty when the range could not
    /// be resolved, which reads differently from a run that committed nothing.
    pub commits: Vec<Commit>,
    /// Whether the commit range was resolvable at all. False means the list is
    /// missing, not that the agent wrote nothing.
    pub commits_known: bool,
    pub diffstat: Option<crate::git::DiffStat>,
    /// Preformatted by the caller, which already prices tokens for the UI.
    pub usage: Option<String>,
    /// Where the agent's own transcript for this workspace is, when it exists
    /// and we know the shape of it. Agency does not own that directory, so the
    /// record points at it rather than claiming to have kept it.
    pub transcript: Option<String>,
}

/// Render the record. Deterministic; the only clock is in the caller.
pub fn render(r: &RunRecord) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("run: {}\n", r.id));
    out.push_str(&format!("agent: {}\n", r.agent));
    if let Some(m) = &r.model {
        out.push_str(&format!("model: {m}\n"));
    }
    out.push_str(&format!("branch: {}\n", r.branch));
    out.push_str(&format!("base: {}\n", r.base));
    out.push_str(&format!("outcome: {}\n", outcome_key(&r.outcome)));
    out.push_str(&format!("started: {}\n", epoch_to_rfc3339(r.created_at)));
    out.push_str(&format!("archived: {}\n", epoch_to_rfc3339(r.archived_at)));
    out.push_str("---\n\n");

    out.push_str(&format!("# {}\n\n", r.heading));
    if let Some(issue) = &r.issue {
        out.push_str(&format!("Dispatched from [[{issue}]].\n\n"));
    }
    out.push_str(&format!("{}\n\n", outcome_sentence(&r.outcome, &r.branch)));

    if !r.prompt.trim().is_empty() {
        out.push_str("## Prompt\n\n");
        for line in r.prompt.trim_end().lines() {
            out.push_str(&format!("> {line}\n"));
        }
        out.push('\n');
    }

    // A run that worked in the project's checkout has no branch of its own and
    // so no range to list. Printing "not recoverable" there would report a
    // failure where there was never anything to read. Everything below this
    // section still applies to such a run: it spent tokens and it has a
    // transcript like any other.
    if !matches!(r.outcome, Outcome::NoWorkspace { .. }) {
        out.push_str("## Commits\n\n");
        if !r.commits_known {
            out.push_str(
                "Not recoverable: the range this branch covered could not be resolved at \
                 archive time.\n\n",
            );
        } else if r.commits.is_empty() {
            out.push_str("None. The agent committed nothing to its branch.\n\n");
        } else {
            for c in &r.commits {
                out.push_str(&format!("- `{}` {}\n", c.short, c.subject));
            }
            out.push('\n');
        }
    }

    if let Some(d) = &r.diffstat {
        out.push_str("## Changes\n\n");
        out.push_str(&format!(
            "{} file{}, +{} −{}\n\n",
            d.files,
            if d.files == 1 { "" } else { "s" },
            d.added,
            d.deleted
        ));
    }

    if let Some(u) = &r.usage {
        out.push_str(&format!("## Cost\n\n{u}\n\n"));
    }

    if let Some(t) = &r.transcript {
        out.push_str("## Transcript\n\n");
        out.push_str(&format!(
            "{} keeps this run's own transcript at `{}`. Agency does not manage that \
             directory and has not touched it.\n",
            r.agent, t
        ));
    }
    out
}

fn outcome_key(o: &Outcome) -> &'static str {
    match o {
        Outcome::Merged { .. } => "merged",
        Outcome::Pushed => "pushed",
        Outcome::Kept { .. } => "kept",
        Outcome::Dropped { .. } => "dropped",
        Outcome::Empty => "empty",
        Outcome::NoWorkspace { .. } => "in-checkout",
    }
}

/// The one line that says where the work is now — the thing a person reading
/// an old record actually wants, and the thing a database row could never say.
fn outcome_sentence(o: &Outcome, branch: &str) -> String {
    match o {
        Outcome::Merged { base } => {
            format!("Merged into `{base}`. The worktree and the `{branch}` branch were removed; the commits are on `{base}`.")
        }
        Outcome::Pushed => {
            format!("Pushed to a remote. The worktree and the local `{branch}` branch were removed; the commits are on the remote.")
        }
        Outcome::Kept { commits } => format!(
            "Not merged. The worktree was removed and the `{branch}` branch kept, carrying {commits} commit{} that {} nowhere else.",
            if *commits == 1 { "" } else { "s" },
            if *commits == 1 { "is" } else { "are" },
        ),
        Outcome::Dropped { commits } => format!(
            "Not merged. The worktree and the `{branch}` branch were both removed, taking {commits} commit{} with them.",
            if *commits == 1 { "" } else { "s" },
        ),
        Outcome::Empty => {
            format!("Nothing was committed. The worktree and the `{branch}` branch were removed.")
        }
        Outcome::NoWorkspace { branch } => format!(
            "This agent worked in the project's own checkout rather than a worktree of its own. Nothing was removed, and whatever it changed is still there on `{branch}`."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RunRecord {
        RunRecord {
            id: "agent-3f9c".into(),
            heading: "Tidy up the teardown flow".into(),
            agent: "claude".into(),
            model: Some("claude-opus-5".into()),
            prompt: "Work on issue AGE-149".into(),
            branch: "agent/3f9c".into(),
            base: "main".into(),
            issue: Some("AGE-149".into()),
            created_at: 1_785_144_600,
            archived_at: 1_785_231_000,
            outcome: Outcome::Merged { base: "main".into() },
            commits: vec![
                Commit {
                    short: "a1b2c3d".into(),
                    subject: "decide teardown from the branch".into(),
                },
                Commit {
                    short: "e4f5a6b".into(),
                    subject: "keep a record of a finished run".into(),
                },
            ],
            commits_known: true,
            diffstat: Some(crate::git::DiffStat { added: 430, deleted: 77, files: 12 }),
            usage: Some("1.2M tokens · $3.40".into()),
            transcript: Some("~/.claude/projects/-x-y".into()),
        }
    }

    #[test]
    fn renders_frontmatter_and_every_section() {
        let md = render(&sample());
        assert!(md.starts_with("---\nrun: agent-3f9c\n"));
        assert!(md.contains("outcome: merged\n"));
        assert!(md.contains("started: 2026-07-27T09:30:00Z\n"));
        assert!(md.contains("# Tidy up the teardown flow"));
        assert!(md.contains("Dispatched from [[AGE-149]]."));
        assert!(md.contains("- `a1b2c3d` decide teardown from the branch"));
        assert!(md.contains("12 files, +430 −77"));
        assert!(md.contains("1.2M tokens · $3.40"));
        assert!(md.contains("~/.claude/projects/-x-y"));
    }

    #[test]
    fn says_where_the_work_is_now() {
        // The sentence is the whole value of the file: an archived run whose
        // branch is gone has to be able to say the commits are on main.
        let md = render(&sample());
        assert!(md.contains("Merged into `main`."), "{md}");
        assert!(md.contains("the commits are on `main`"));
    }

    #[test]
    fn an_unmerged_kept_branch_names_what_it_is_holding() {
        let r = RunRecord { outcome: Outcome::Kept { commits: 1 }, ..sample() };
        let md = render(&r);
        assert!(md.contains("branch kept, carrying 1 commit that is nowhere else"), "{md}");
        assert!(md.contains("outcome: kept"));
    }

    #[test]
    fn a_run_in_the_checkout_promises_nothing_was_removed() {
        let r = RunRecord {
            outcome: Outcome::NoWorkspace { branch: "main".into() },
            branch: String::new(),
            ..sample()
        };
        let md = render(&r);
        assert!(md.contains("Nothing was removed"), "{md}");
        assert!(md.contains("still there on `main`"));
        assert!(md.contains("outcome: in-checkout"));
        // No branch of its own means no range to list, which is different from
        // a range that could not be read.
        assert!(!md.contains("## Commits"), "{md}");
        assert!(!md.contains("Not recoverable"));
        // The sections that still apply to such a run are not lost with it.
        assert!(md.contains("## Cost"), "{md}");
        assert!(md.contains("## Transcript"), "{md}");
    }

    #[test]
    fn a_dropped_branch_admits_what_went_with_it() {
        let md = render(&RunRecord { outcome: Outcome::Dropped { commits: 3 }, ..sample() });
        assert!(md.contains("taking 3 commits with them"), "{md}");
    }

    #[test]
    fn an_unresolvable_commit_range_is_not_reported_as_no_commits() {
        // "we could not read this" and "the agent wrote nothing" are different
        // claims and only one of them may be printed for each case.
        let md = render(&RunRecord { commits: vec![], commits_known: false, ..sample() });
        assert!(md.contains("Not recoverable"), "{md}");
        assert!(!md.contains("committed nothing"));

        let md = render(&RunRecord { commits: vec![], commits_known: true, ..sample() });
        assert!(md.contains("The agent committed nothing"), "{md}");
        assert!(!md.contains("Not recoverable"));
    }

    #[test]
    fn unknown_fields_print_as_nothing_rather_than_zero() {
        let md = render(&RunRecord {
            model: None,
            issue: None,
            prompt: "  ".into(),
            diffstat: None,
            usage: None,
            transcript: None,
            ..sample()
        });
        assert!(!md.contains("model:"));
        assert!(!md.contains("Dispatched from"));
        assert!(!md.contains("## Prompt"));
        assert!(!md.contains("## Changes"));
        assert!(!md.contains("## Cost"));
        assert!(!md.contains("## Transcript"));
        assert!(md.contains("## Commits"), "the commit list is always accounted for");
    }

    #[test]
    fn a_multiline_prompt_is_quoted_line_by_line() {
        // A bare blockquote of a multi-line prompt renders as one run-on
        // paragraph; every line needs its own marker.
        let md = render(&RunRecord { prompt: "first\n\nthird\n".into(), ..sample() });
        assert!(md.contains("> first\n> \n> third\n"), "{md}");
    }
}
