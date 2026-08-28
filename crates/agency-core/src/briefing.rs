//! What a dispatched agent is told about the project's issue tracker, written
//! into each fresh worktree as `AGENTS.md` — the one instructions file every
//! agent Agency ships (Claude Code, Codex, Cursor, OpenCode, DeepSeek Harness)
//! reads on its own, with no launch flag to arrange it.
//!
//! Issue files live in the project's own checkout under `.agency/issues/` and
//! are untracked by git, so `git worktree add` never materializes them: from
//! inside a worktree there is no copy of the tracker and nothing in the
//! checkout hinting that one exists. Issue-dispatched runs learn where it is
//! from their prompt, but that is one path of several. A plain run carries only
//! the user's text, and the default interactive run is dispatched with an empty
//! prompt and typed into afterwards — neither ever hears of the tracker, which
//! is exactly when an agent asked for a follow-up invents somewhere to put it.
//!
//! Naming the app matters as much as naming the path. An agent that meets a
//! project-local tracker with `AGE-14`-shaped keys and no other context reaches
//! for the tracker it knows best and starts calling this one Linear.
//!
//! The section is fenced by HTML comment markers so it can be refreshed in
//! place on a later run without disturbing anything else in the file.

use anyhow::Result;
use std::path::Path;

pub const AGENTS_FILE: &str = "AGENTS.md";

/// Fences around the generated section. Anything between them is ours to
/// rewrite; everything outside is left exactly as it was found.
pub const SECTION_START: &str = "<!-- agency:issues:start -->";
pub const SECTION_END: &str = "<!-- agency:issues:end -->";

/// The tracker briefing for a project rooted at `repo_root`, whose issue keys
/// are prefixed `issue_key` (`AGE`).
///
/// Every path is absolute: the reader is in `.agency/worktrees/<id>/`, where a
/// relative `.agency/issues/` is a dead path. Deliberately short, and it hands
/// off to the tracker's own README rather than restating the file format.
/// Naming a schema here would put it in front of every agent on every run when
/// only the ones filing an issue need it, and the same measurement that shaped
/// the dispatch prompt says a schema in view is a schema someone goes reading
/// around.
///
/// The exception is a project whose tracker is still empty: the issues
/// directory and its README arrive with the first issue, so until then there is
/// nothing to point at, and an agent left to guess the frontmatter writes a
/// file that `reconcile` skips as malformed without telling anyone. That case
/// gets the minimum schema inline instead.
pub fn issues_section(repo_root: &Path, issue_key: &str) -> String {
    let issues = repo_root.join(crate::issuefs::ISSUES_DIR);
    let readme = issues.join("README.md");
    let filing = if readme.is_file() {
        format!(
            "Read `{}` before filing a follow-up, commenting on an issue, or changing a \
             status. It has the file format and the numbering rule.",
            readme.display()
        )
    } else {
        format!(
            "This tracker has no issues yet, so it has no directory and no README. To file \
             the first one, create `{first}` with this shape, and Agency picks it up:\n\
             \n\
             ```markdown\n\
             ---\n\
             key: {issue_key}-1\n\
             status: todo        # backlog|todo|in_progress|in_review|done|cancelled\n\
             priority: 0         # 0-4\n\
             created: 2026-01-01T00:00:00Z\n\
             updated: 2026-01-01T00:00:00Z\n\
             ---\n\
             # Issue title\n\
             \n\
             Body markdown.\n\
             ```\n\
             \n\
             Timestamps are UTC RFC3339. Once that file exists the directory gains a \
             `README.md` with the full rules; read it before filing anything else.",
            first = issues.join(format!("{issue_key}-1.md")).display(),
        )
    };
    format!(
        "{SECTION_START}\n\
         ## Issues\n\
         \n\
         This project's issues are tracked in Agency, the app that dispatched \
         you. They are markdown files, one per issue, under `{issues}`, each \
         named for its issue key: this project's are `{issue_key}-<n>`, so \
         `{issue_key}-14.md`.\n\
         \n\
         That directory is in the project's own checkout, not in this worktree, \
         and it is untracked by git: there is no copy here, nothing to commit, \
         and nothing to merge. An edit to an issue file takes effect as soon as \
         it is written.\n\
         \n\
         {filing}\n\
         \n\
         Agency generates this section. Edits inside the markers are replaced.\n\
         {SECTION_END}\n",
        issues = issues.display(),
    )
}

/// Splice the tracker section into `text`, replacing an existing one in place
/// and otherwise appending it. Pure, so the merge rule is testable without a
/// worktree.
pub fn splice_section(text: &str, section: &str) -> String {
    let Some(start) = text.find(SECTION_START) else {
        if text.trim().is_empty() {
            return section.to_string();
        }
        // Append, with exactly one blank line between the file's last content
        // and our heading.
        return format!("{}\n\n{section}", text.trim_end());
    };
    // A start marker with no end marker means someone truncated the file
    // mid-section; replacing to the end is the only reading that doesn't leave
    // a stray fragment behind.
    let end = text[start..].find(SECTION_END).map_or(text.len(), |i| start + i + SECTION_END.len());
    let head = &text[..start];
    let tail = text[end..].trim_start_matches('\n');
    if tail.is_empty() {
        format!("{head}{}", section.trim_start())
    } else {
        format!("{head}{}\n{tail}", section.trim_start().trim_end())
    }
}

/// Whether git tracks `AGENTS.md` in this worktree.
///
/// A tracked file is the repo's own: appending to it would show up in the run's
/// diff and ride into the project's main branch at merge, which is not a change
/// Agency gets to make on the user's behalf.
fn agents_file_tracked(worktree: &Path) -> bool {
    std::process::Command::new("git")
        .args(["ls-files", "--", AGENTS_FILE])
        .current_dir(worktree)
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// Write Agency's issue-tracker section into a fresh worktree's `AGENTS.md`,
/// and keep the file out of git so no agent commits it into the project.
///
/// Returns whether the file was written. Skipped, with a reason logged, when
/// the repo tracks `AGENTS.md` itself — see [`agents_file_tracked`].
pub fn emit_agents_md(worktree: &Path, repo_root: &Path, issue_key: &str) -> Result<bool> {
    if agents_file_tracked(worktree) {
        log::info!(
            "{AGENTS_FILE} is tracked in {}: leaving it alone, so the issue tracker is not \
             introduced to agents there",
            worktree.display()
        );
        return Ok(false);
    }
    let path = worktree.join(AGENTS_FILE);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let next = splice_section(&current, &issues_section(repo_root, issue_key));
    if next != current {
        crate::issuefs::atomic_write(&path, &next)?;
    }
    // The file is untracked and generated, so git must not offer it: agents
    // reach for `git add -A` constantly, and a briefing committed onto the
    // branch lands in the project at merge. Excluding costs nothing if the user
    // later wants it tracked — `git add -f` wins, and an exclude has no say
    // over a file once it is tracked.
    if let Err(e) = crate::worktree::ensure_exclude_pattern(repo_root, "/AGENTS.md") {
        log::warn!("excluding {AGENTS_FILE} in {}: {e}", repo_root.display());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project root whose tracker already holds issues, so the README the
    /// briefing points at actually exists.
    fn stocked_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let issues = dir.path().join(crate::issuefs::ISSUES_DIR);
        std::fs::create_dir_all(&issues).unwrap();
        std::fs::write(issues.join("README.md"), crate::issuefs::ISSUES_README).unwrap();
        dir
    }

    fn section_in(root: &Path) -> String {
        issues_section(root, "AGE")
    }

    #[test]
    fn section_names_the_app_and_uses_absolute_paths() {
        let root = stocked_root();
        let s = section_in(root.path());
        // The whole point: an agent that can't name the tracker calls it Linear.
        assert!(s.contains("tracked in Agency"), "{s}");
        // A relative `.agency/issues` is a dead path from inside a worktree, so
        // every path the agent is handed has to be absolute.
        let issues = root.path().join(".agency/issues");
        assert!(s.contains(&issues.display().to_string()), "{s}");
        assert!(s.contains(&issues.join("README.md").display().to_string()), "{s}");
        // The project's own key, so `AGE-14` reads as this tracker's, not a
        // shape the agent recognizes from somewhere else.
        assert!(s.contains("`AGE-<n>`"), "{s}");
        // With a README to point at, the schema stays behind it rather than in
        // front of every agent on every run.
        assert!(!s.contains("priority:"), "{s}");
        assert!(s.starts_with(SECTION_START) && s.trim_end().ends_with(SECTION_END), "{s}");
    }

    #[test]
    fn an_empty_tracker_gets_the_schema_inline_instead_of_a_dead_pointer() {
        // No `.agency/issues/` yet: the directory and its README arrive with
        // the first issue, so pointing at the README would send an agent to a
        // file that isn't there, and a guessed frontmatter is silently dropped
        // by reconcile.
        let dir = tempfile::tempdir().unwrap();
        let s = section_in(dir.path());
        // No absolute path to a README that isn't there to be read.
        let readme = dir.path().join(".agency/issues/README.md");
        assert!(!s.contains(&readme.display().to_string()), "{s}");
        assert!(s.contains("no issues yet"), "{s}");
        assert!(s.contains("key: AGE-1"), "{s}");
        assert!(s.contains("backlog|todo|in_progress|in_review|done|cancelled"), "{s}");
        assert!(
            s.contains(&dir.path().join(".agency/issues/AGE-1.md").display().to_string()),
            "{s}"
        );

        // And what it tells the agent to write must actually parse, or the
        // first follow-up in every new project is dropped on the floor.
        let body = s
            .split("```markdown\n")
            .nth(1)
            .and_then(|t| t.split("```").next())
            .expect("a fenced example");
        let parsed = crate::issuefs::parse_issue_file("AGE-1", body).expect("example parses");
        assert_eq!(parsed.key, "AGE-1");
        assert_eq!(parsed.status, crate::registry::IssueStatus::Todo);
        assert_eq!(parsed.title, "Issue title");
    }

    #[test]
    fn splice_creates_appends_and_replaces() {
        let root = stocked_root();
        let s = section_in(root.path());

        // No file yet: the section is the whole file.
        assert_eq!(splice_section("", &s), s);

        // A repo's own AGENTS.md keeps every word, and gains ours at the end.
        let theirs = "# Our rules\n\nRun the tests.\n";
        let appended = splice_section(theirs, &s);
        assert!(appended.starts_with("# Our rules\n\nRun the tests.\n\n"), "{appended}");
        assert!(appended.contains(SECTION_START), "{appended}");

        // Re-emitting is idempotent, and a stale section is refreshed in place
        // rather than duplicated — a worktree can be written to more than once.
        assert_eq!(splice_section(&appended, &s), appended);
        // A project that moved on disk: the old absolute paths must not survive.
        let stale = appended.replace(&root.path().display().to_string(), "/moved");
        let fixed = splice_section(&stale, &s);
        assert_eq!(fixed, appended);
        assert!(!fixed.contains("/moved"), "{fixed}");
        assert_eq!(fixed.matches(SECTION_START).count(), 1, "{fixed}");
    }

    #[test]
    fn splice_preserves_content_after_the_section() {
        let root = stocked_root();
        let s = section_in(root.path());
        let with_tail = format!("# Top\n\n{s}\n## Theirs\n\nkeep me\n");
        let out = splice_section(&with_tail, &s);
        assert!(out.starts_with("# Top\n"), "{out}");
        assert!(out.ends_with("## Theirs\n\nkeep me\n"), "{out}");
        assert_eq!(out.matches(SECTION_START).count(), 1, "{out}");
        // And it stays stable across a second pass.
        assert_eq!(splice_section(&out, &s), out);
    }

    #[test]
    fn splice_repairs_a_truncated_section() {
        // Start marker, no end marker: replace to the end rather than leave a
        // fragment that would then never match again.
        let root = stocked_root();
        let s = section_in(root.path());
        let broken = format!("# Top\n\n{SECTION_START}\n## Issues\n\nhalf a bri");
        let out = splice_section(&broken, &s);
        assert!(out.starts_with("# Top\n"), "{out}");
        assert!(!out.contains("half a bri"), "{out}");
        assert_eq!(out.matches(SECTION_START).count(), 1, "{out}");
    }
}
