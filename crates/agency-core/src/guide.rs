//! The workspace's seeded starter note. Every new workspace gets a
//! `Welcome.md` that demonstrates the vault's non-obvious features in place:
//! frontmatter properties (the note has some), checkbox tasks (they appear on
//! Home), wikilinks, the journal, and search. Seeded once at creation, before
//! the initial commit; deleting it is respected — only the explicit
//! "Workspace Guide" command brings it back.
//!
//! An existing `Welcome.md` never self-updates, so a wording change here does
//! not reach a workspace that already has one. The privacy claim is the one
//! exception: `repair_opening` rewrites that paragraph in place, because
//! shipping a claim we cannot stand behind is not something a user should have
//! to opt out of.

use anyhow::Result;
use std::path::Path;

pub const GUIDE_FILE: &str = "Welcome.md";

pub const WORKSPACE_GUIDE: &str = r#"---
type: guide
status: example
---
# Welcome to your workspace

This folder is a plain-markdown vault. Every file in it is yours: edit notes
here, in any other editor, or hand one to an agent. It is all just files on
disk.

Agency does no first-party data collection: no analytics, no telemetry, no
account. Agents are third party, so what you hand one goes to its provider.

Delete this note whenever you like. It comes back only if you run
"Workspace Guide" from the command palette (Cmd+K).

## Notes and links

Link a note by writing its name in double brackets, like [[My first note]].
Cmd+click follows a link; if the note does not exist yet, Agency offers to
create it. The panel on the right of every note has two tabs. Note lists
the note's Backlinks (notes that link here) and Mentions (issues that link
here). Agents puts a live agent or terminal beside the note, so you can
work on it together without leaving the page.

Tag a note by writing #tags anywhere in its body, then search them with `#`.

Drag files in from Finder and drop them on the tree: they are copied into the
folder you drop on, leaving the originals where they were. Markdown becomes a
note. Anything else, a screenshot or a PDF, becomes a dimmed row in the tree
that opens in the Files tab, and Agency offers to link it from the note you
have open.

Right-click anywhere in a note for its formatting menu: Format (bold, italic,
strikethrough, ==highlight==, code), Paragraph (lists, heading levels, quote)
and Insert (links, tables, callouts, code blocks). Each acts on the selection,
or on the word and line under the cursor when there is none.

Cmd+F searches the note you are in, with match case, whole word and regular
expression options; Option+Cmd+F adds the replace field. Both live in the Edit
menu, and both work the same way on a file in the Files tab or on an issue
description. On an issue board, Cmd+F goes to the board's own search instead.
Enter and Shift+Enter walk the matches from inside the find field; Cmd+G and
Shift+Cmd+G do the same from anywhere, so you can close the bar and keep
jumping.

## Properties

The card at the top of this note is its properties (stored as plain
`key: value` frontmatter in the file). Click a name or value to edit it in
place; Enter or clicking away saves, Escape reverts. Hover a row for its
tools: the magnifier finds every note sharing that property, the cross
removes it. Hover the properties label and press + to add one; names and
values suggest what you already use across the vault. A note without
properties gets them from "+ add property" in the side panel. Filter notes
in the Docs search box
with the same syntax, for example `type:guide`, or `status:` to find every
note that has a status at all.

## Tasks

- [ ] check this box from the Home screen
- [ ] then delete these two lines

Any `- [ ]` line in any note is collected on the Home screen (Issues tab,
Tasks section). Checking it there edits the note in place. Hover a task for
"promote", which turns it into a tracker issue linked back to its note.
Right-click a note in that list to hide sources you do not care about.

## The journal

Cmd+Shift+D opens today's note under `journal/`. If you create
`templates/daily.md`, new days start from it (`{{date}}` becomes the day's
stamp). Arrows in the editor header step between days.

"Generate Weekly Note" (File menu, or search the palette for "weekly")
writes `journal/weekly/` from what actually happened this week across your
projects: merges, closed issues, archived agent runs, all as links. It ends
with a Notes section you can ask an agent to narrate.

## Issues and agents

The Issues tab here is a tracker for personal work: plans, errands,
follow-ups. An issue's description is the same editor a note is: markdown
renders as you write it, right-click gives the formatting menu, and
[[links]] and #tags work the same way. Reference any issue from a note by
its key, written like `[[AGE-14]]`, and that issue's detail pane will list
the notes mentioning it. Issues can also be linked to each other: the Links
section in the detail pane, + to pick another issue (any project's). Links
go both ways and are clickable, so either issue leads to the other.

Comments go under the description, in the same pane. They are markdown too,
they are stored in the issue's own file, and an agent dispatched on the
issue is handed the thread along with the description, so a comment is a
way to correct or narrow the ask before you start one. Agents can write
comments back the same way. You can dispatch agents on writing tasks the
same way you would in a code project. If the workspace uses git they work
on a branch and you merge; if it does not, they edit the notes in place,
and there is no Source Control tab and nothing to merge.

The Agents tab in a note's right panel is the same set of agents, shown
next to what you are writing. Start one with + there, switch between them
from the picker, and use the arrow to open the current one full size in
the Agents tab.

Drop a file onto an issue's detail pane, paste an image into its
description, or use the ⊕ button to attach it. Attachments are copied in
beside the issue files and linked from the description, so they travel with
the issue, and an agent working the issue can read them.
"#;

/// The guide's opening, carrying the privacy claim in the only form allowed:
/// scoped to what Agency itself collects, and honest that the agents are
/// somebody else's. Never "nothing leaves your machine" — launching third-party
/// agents that talk to their own providers is the app's whole job, so a claim
/// that omits them is the one a reader catches. Two sentences on
/// purpose. This is a starter note, not a privacy page, and the long version
/// (the update check, git remotes) belongs on the site. Held as its own const
/// so `repair_opening` can splice it into a `Welcome.md` seeded before this
/// wording existed; the tests keep it and `WORKSPACE_GUIDE` in sync.
const OPENING: &str = r#"This folder is a plain-markdown vault. Every file in it is yours: edit notes
here, in any other editor, or hand one to an agent. It is all just files on
disk.

Agency does no first-party data collection: no analytics, no telemetry, no
account. Agents are third party, so what you hand one goes to its provider.
"#;

/// What the guide said until 2026-08-20: "Nothing leaves your machine". Agency
/// cannot make that claim, because the agent CLIs it launches are the whole
/// point of the app and every one of them talks to its own provider.
const STALE_OPENING: &str = r#"This folder is a plain-markdown vault. Every file in it is yours: edit notes
here, in any other editor, or hand one to an agent. Nothing leaves your
machine; it is all just files on disk.
"#;

/// Swap the stale opening for the current one, or `None` when there is nothing
/// to do. Exact match only: a user who reworded that paragraph, or wrote their
/// own note over it, keeps what they wrote.
pub fn repair_opening(text: &str) -> Option<String> {
    text.contains(STALE_OPENING).then(|| text.replace(STALE_OPENING, OPENING))
}

/// Seed the guide note if it doesn't exist. Returns whether it was created.
pub fn ensure_guide(root: &Path) -> Result<bool> {
    let path = root.join(GUIDE_FILE);
    if path.exists() {
        // Welcome.md never self-updates, so a workspace seeded before
        // 2026-08-20 still opens with "Nothing leaves your machine". Repair
        // that one paragraph here, which is every place the guide is touched:
        // workspace create/adopt, and the palette's "Workspace Guide". An
        // unreadable note is left alone rather than failing the caller, which
        // is otherwise just creating a workspace.
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(fixed) = repair_opening(&text) {
                crate::issuefs::atomic_write(&path, &fixed)?;
            }
        }
        return Ok(false);
    }
    crate::issuefs::atomic_write(&path, WORKSPACE_GUIDE)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_guide_seeds_once_and_respects_edits() {
        let dir = tempdir().unwrap();
        assert!(ensure_guide(dir.path()).unwrap());
        let text = std::fs::read_to_string(dir.path().join(GUIDE_FILE)).unwrap();
        // The guide demos frontmatter and tasks by carrying real ones.
        assert!(text.starts_with("---\n"));
        assert!(text.contains("# Welcome"));
        assert!(text.contains("- [ ]"));

        // Second call is a no-op that preserves user edits.
        std::fs::write(dir.path().join(GUIDE_FILE), "mine now").unwrap();
        assert!(!ensure_guide(dir.path()).unwrap());
        assert_eq!(std::fs::read_to_string(dir.path().join(GUIDE_FILE)).unwrap(), "mine now");
    }

    #[test]
    fn guide_scopes_the_privacy_claim_to_first_party_collection() {
        // The claim is scoped to what Agency itself collects and never implies
        // the agents are ours. "Nothing leaves your machine" is the specific
        // phrasing this rules out: the agents are third party and talk to their
        // own providers.
        assert!(WORKSPACE_GUIDE.contains(OPENING), "the guide must carry OPENING verbatim");
        assert!(!WORKSPACE_GUIDE.contains("Nothing leaves your machine"));
        assert!(WORKSPACE_GUIDE.contains("no first-party data collection"));
        assert!(WORKSPACE_GUIDE.contains("Agents are third party"));
    }

    #[test]
    fn repair_opening_rewrites_a_stale_note_and_nothing_else() {
        let stale = format!("---\ntype: guide\n---\n# Welcome\n\n{STALE_OPENING}\nMy own line.\n");
        let fixed = repair_opening(&stale).expect("the stale claim is rewritten");
        assert!(!fixed.contains("Nothing leaves your machine"));
        assert!(fixed.contains(OPENING));
        // Everything the user wrote around it survives.
        assert!(fixed.starts_with("---\ntype: guide\n---\n# Welcome\n\n"));
        assert!(fixed.ends_with("My own line.\n"));

        // Already current, or reworded by hand: pure no-op either way.
        assert!(repair_opening(&fixed).is_none());
        assert!(repair_opening("nothing leaves your machine, roughly").is_none());
        assert!(repair_opening("mine now").is_none());
    }

    #[test]
    fn ensure_guide_repairs_the_stale_claim_in_place() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(GUIDE_FILE);
        std::fs::write(&path, format!("# Welcome\n\n{STALE_OPENING}\nMy own line.\n")).unwrap();

        // Not a creation, but the claim is fixed and the user's line is kept.
        assert!(!ensure_guide(dir.path()).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("Nothing leaves your machine"));
        assert!(text.contains("no first-party data collection"));
        assert!(text.ends_with("My own line.\n"));
    }
}
