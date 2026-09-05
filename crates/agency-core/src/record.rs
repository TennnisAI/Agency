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

/// Where a run's *superseded* record goes when the run is restored: a numbered
/// sibling of [`path`], oldest first, so `<run>.1.md` is the account of the run
/// as it stood at its first archive.
///
/// Restoring used to delete the record outright, on the grounds that a file
/// left in place would be read as the account of a run that is live again.
/// That is true of `<run>.md` and is why this is a rename rather than nothing,
/// but the deletion also threw away the only account of what the run had done
/// up to that point: its commit list, how the work ended, what it cost. The
/// conversation survived a restore-and-archive cycle and those did not, so a
/// run archived twice remembered everything it *said* and nothing it *did*.
pub fn prior_path(repo: &Path, run_id: &str, n: u32) -> PathBuf {
    dir(repo).join(format!("{run_id}.{n}.md"))
}

/// The ordinal in `<run>.<n>.md`, for the callers that have to find every
/// superseded record a run has: the next free number, and the sweep that
/// deletes them all with the run.
///
/// Deliberately strict about the shape. Run ids are minted by
/// [`crate::branchname`] and carry no dots, so nothing else in the records
/// directory can collide with this, and a name that does not parse is left
/// alone rather than guessed at.
pub fn prior_ordinal(run_id: &str, file_name: &str) -> Option<u32> {
    file_name.strip_prefix(&format!("{run_id}."))?.strip_suffix(".md")?.parse().ok()
}

/// Stamp a record that is about to be superseded, so the file says why it is
/// not the current one. Pure: text in, text out.
pub fn mark_superseded(text: &str, run_id: &str, restored_at: i64) -> String {
    let mut out = text.trim_end().to_string();
    out.push_str("\n\n## Superseded\n\n");
    // Named as a convention rather than as one file: a run restored twice
    // pushes this stint's successor down to `<run>.2.md`, so a pointer at
    // `<run>.md` alone would go stale the second time round.
    out.push_str(&format!(
        "This agent was restored on {} and went on working, so this is the account of one stint \
         of it, not of the whole run. The others are the records beside this one, numbered in \
         order, ending with `{run_id}.md` when the run is next archived.\n",
        epoch_to_rfc3339(restored_at)
    ));
    out
}

/// Where a run's rescued transcript lives: a directory beside the record file
/// holding the agent's session files exactly as it wrote them, moved here at
/// archive time because their original home was keyed by a worktree path that
/// stopped existing (AGE-152).
pub fn transcript_dir(repo: &Path, run_id: &str) -> PathBuf {
    dir(repo).join(format!("{run_id}.transcript"))
}

/// Where the rescued transcript of an *extra* agent tab lives: a sibling of
/// [`transcript_dir`] named for the agent that wrote it (AGE-184).
///
/// A run's extra tabs may run a different agent than the run does, and each
/// agent keeps its transcripts under a root of its own — so one directory per
/// run only ever rescued the run's own agent, and a pi tab in a claude run had
/// its conversation left behind in a directory keyed to a worktree path that
/// had just stopped existing. That is the same leak AGE-152 fixed for the run
/// itself, one level down.
///
/// A sibling rather than a subdirectory of the run's own: the readers descend
/// exactly one level looking for session files (pi nests a store per session),
/// so a directory *inside* the rescue would put a pi tab's files one level too
/// deep to be found, and would be re-read with the wrong dialect besides.
///
/// `agent` is a profile name, which is a user-editable string; anything
/// outside the allowlist is refused rather than escaped, on the same principle
/// as [`crate::sessionstore::dir`] — a path component assembled from arbitrary
/// text is a traversal waiting to be found.
pub fn transcript_dir_for(repo: &Path, run_id: &str, agent: &str) -> Option<PathBuf> {
    if !agent_leaf(agent) {
        return None;
    }
    Some(dir(repo).join(format!("{run_id}.transcript.{agent}")))
}

/// The agent name in `<run>.transcript.<agent>`, for the restore side, which
/// has no session rows left to ask: they are deleted at archive time.
pub fn transcript_dir_agent(run_id: &str, file_name: &str) -> Option<String> {
    let agent = file_name.strip_prefix(&format!("{run_id}.transcript."))?;
    agent_leaf(agent).then(|| agent.to_string())
}

fn agent_leaf(agent: &str) -> bool {
    !agent.is_empty()
        && agent != "."
        && agent != ".."
        && agent.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
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

/// What the record can say about the agent's own conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptNote {
    /// The session files were moved into the archive beside this record
    /// (see [`transcript_dir`]) and the agent's own directory, keyed by the
    /// removed worktree path, was cleaned up.
    Rescued {
        /// Top-level session files rescued.
        sessions: usize,
        /// The agent's own command for picking the newest session up again,
        /// e.g. `claude --resume <id>`. Only written for agents whose resume
        /// verb is actually known; a guessed command would be worse than none.
        resume: Option<String>,
    },
    /// Still in the agent's own store at this path. Named, not copied: the
    /// rescue did not happen (it failed, or the run had no worktree of its
    /// own), and pointing is the most that can honestly be claimed.
    Named { dir: String },
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
    /// What became of the agent's own transcript, when it exists and we know
    /// the shape of it: rescued into the archive, or named where it still is.
    pub transcript: Option<TranscriptNote>,
    /// The file names of this run's superseded records, oldest first, when it
    /// has been restored and archived before (see [`prior_path`]). Empty for
    /// the run that has only ended once, which is nearly all of them.
    pub prior: Vec<String>,
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

    match &r.transcript {
        Some(TranscriptNote::Rescued { sessions, resume }) => {
            out.push_str("## Transcript\n\n");
            out.push_str(&format!(
                "The conversation is in the archive beside this record: {sessions} session \
                 file{} under `{}.transcript/`, moved out of {}'s own store when the worktree \
                 went, since that store was keyed by the worktree's path. If this agent is \
                 restored, the sessions go back first, so its own resume picks the \
                 conversation up.\n",
                if *sessions == 1 { "" } else { "s" },
                r.id,
                r.agent
            ));
            if let Some(cmd) = resume {
                out.push_str(&format!(
                    "\nWithout Agency, `{cmd}` resumes the newest session once its file is \
                     back in that store.\n"
                ));
            }
        }
        Some(TranscriptNote::Named { dir }) => {
            out.push_str("## Transcript\n\n");
            out.push_str(&format!(
                "{} keeps this run's own transcript at `{}`. Agency does not manage that \
                 directory and has not touched it.\n",
                r.agent, dir
            ));
        }
        None => {}
    }

    // The pointer that keeps a restored run's history legible. Without it the
    // numbered files beside this one are unexplained, and this record reads as
    // the whole of a run that had already been archived once.
    if !r.prior.is_empty() {
        // The transcript section ends on a single newline; every other one
        // ends on a blank line. Separate on whichever came before.
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("## Earlier\n\n");
        out.push_str(&format!(
            "This agent was archived and restored {} before, so what is above covers its most \
             recent stint only. The earlier record{} beside this one, oldest first: {}.\n",
            if r.prior.len() == 1 { "once".into() } else { format!("{} times", r.prior.len()) },
            if r.prior.len() == 1 { " is" } else { "s are" },
            r.prior.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
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
            transcript: Some(TranscriptNote::Rescued {
                sessions: 2,
                resume: Some("claude --resume 16fc10c1".into()),
            }),
            prior: Vec::new(),
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
        assert!(md.contains("2 session files under `agent-3f9c.transcript/`"));
        assert!(md.contains("`claude --resume 16fc10c1`"));
    }

    #[test]
    fn a_transcript_that_was_not_rescued_is_named_where_it_still_is() {
        // The old contract, kept for the cases the rescue cannot cover: point
        // at the agent's own directory and claim nothing about its contents.
        let r = RunRecord {
            transcript: Some(TranscriptNote::Named { dir: "~/.claude/projects/-x-y".into() }),
            ..sample()
        };
        let md = render(&r);
        assert!(md.contains("~/.claude/projects/-x-y"), "{md}");
        assert!(md.contains("has not touched it"));
        assert!(!md.contains(".transcript/"));
    }

    #[test]
    fn a_rescue_with_no_known_resume_verb_offers_no_command() {
        let r = RunRecord {
            transcript: Some(TranscriptNote::Rescued { sessions: 1, resume: None }),
            ..sample()
        };
        let md = render(&r);
        assert!(md.contains("1 session file under"), "{md}");
        assert!(!md.contains("resumes the newest session"));
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

    /// AGE-184: a tab's rescue sits beside the run's own, never inside it —
    /// the readers descend exactly one level looking for session files, and pi
    /// already spends that level on a store per session.
    #[test]
    fn a_tabs_transcript_is_a_sibling_of_the_runs_own() {
        let repo = Path::new("/repo");
        let own = transcript_dir(repo, "fix-a1");
        let tab = transcript_dir_for(repo, "fix-a1", "pi").unwrap();
        assert_eq!(own.file_name().unwrap(), "fix-a1.transcript");
        assert_eq!(tab.file_name().unwrap(), "fix-a1.transcript.pi");
        assert_eq!(own.parent(), tab.parent());
    }

    /// A restored run's earlier record is a numbered sibling, and the run that
    /// is live again owns `<run>.md` alone.
    #[test]
    fn a_superseded_record_is_a_numbered_sibling() {
        let repo = Path::new("/repo");
        assert_eq!(prior_path(repo, "fix-a1", 1).file_name().unwrap(), "fix-a1.1.md");
        assert_eq!(prior_path(repo, "fix-a1", 2).parent(), path(repo, "fix-a1").parent());
        assert_eq!(prior_ordinal("fix-a1", "fix-a1.1.md"), Some(1));
        assert_eq!(prior_ordinal("fix-a1", "fix-a1.12.md"), Some(12));
        // The live record, a neighbour's, a rescue directory and anything else
        // in the folder are all left alone.
        assert_eq!(prior_ordinal("fix-a1", "fix-a1.md"), None);
        assert_eq!(prior_ordinal("fix-a1", "fix-a12.1.md"), None);
        assert_eq!(prior_ordinal("fix-a1", "fix-a1.transcript"), None);
        assert_eq!(prior_ordinal("fix-a1", "fix-a1.transcript.pi"), None);
        assert_eq!(prior_ordinal("fix-a1", "fix-a1.notes.md"), None);
    }

    /// The stamp is what stops a numbered file from being an unexplained
    /// duplicate: it says the run went on, and where the rest of it is.
    #[test]
    fn a_superseded_record_says_it_was_restored() {
        let md = mark_superseded(&render(&sample()), "agent-3f9c", 1_785_231_600);
        assert!(md.starts_with("---\nrun: agent-3f9c\n"), "the original is left intact");
        assert!(md.contains("## Commits"), "including everything it accounted for");
        assert!(md.contains("## Superseded"));
        assert!(md.contains("restored on 2026-07-28T09:40:00Z"), "{md}");
        assert!(md.contains("`agent-3f9c.md`"), "and where the rest of the run is: {md}");
    }

    /// And the current record points back, so the history reads in either
    /// direction.
    #[test]
    fn the_current_record_names_the_ones_it_was_restored_out_of() {
        let one = render(&RunRecord { prior: vec!["agent-3f9c.1.md".into()], ..sample() });
        assert!(one.contains("## Earlier"));
        assert!(one.contains("archived and restored once before"), "{one}");
        assert!(one.contains("The earlier record is beside this one"), "{one}");
        assert!(one.contains("`agent-3f9c.1.md`."), "{one}");

        let two = render(&RunRecord {
            prior: vec!["agent-3f9c.1.md".into(), "agent-3f9c.2.md".into()],
            ..sample()
        });
        assert!(two.contains("archived and restored 2 times before"), "{two}");
        assert!(two.contains("The earlier records are beside this one"), "{two}");
        assert!(two.contains("`agent-3f9c.1.md`, `agent-3f9c.2.md`."), "{two}");

        // The common run has ended once and says nothing about stints at all.
        assert!(!render(&sample()).contains("## Earlier"));
    }

    #[test]
    fn a_tabs_agent_reads_back_off_the_directory_name() {
        assert_eq!(transcript_dir_agent("fix-a1", "fix-a1.transcript.pi").as_deref(), Some("pi"));
        // The run's own rescue is not a tab's, and neither is a neighbour's.
        assert_eq!(transcript_dir_agent("fix-a1", "fix-a1.transcript"), None);
        assert_eq!(transcript_dir_agent("fix-a1", "fix-a12.transcript.pi"), None);
        assert_eq!(transcript_dir_agent("fix-a1", "fix-a1.md"), None);
    }

    /// Profile names are user-editable, so the path component is allowlisted
    /// rather than escaped: a rescue that could be talked into `..` would be
    /// deleting directories outside the archive.
    #[test]
    fn an_agent_name_that_is_not_a_path_component_is_refused() {
        let repo = Path::new("/repo");
        assert!(transcript_dir_for(repo, "fix-a1", "../../etc").is_none());
        assert!(transcript_dir_for(repo, "fix-a1", "..").is_none());
        assert!(transcript_dir_for(repo, "fix-a1", "").is_none());
        assert!(transcript_dir_agent("fix-a1", "fix-a1.transcript.../x").is_none());
    }
}
