//! The workspace's seeded starter note. Every new workspace gets a
//! `Welcome.md` that demonstrates the vault's non-obvious features in place:
//! frontmatter properties (the note has some), checkbox tasks (they appear on
//! Home), wikilinks, the journal, and search. Seeded once at creation, before
//! the initial commit; deleting it is respected — only the explicit
//! "Workspace Guide" command brings it back.

use anyhow::Result;
use std::path::Path;

pub const GUIDE_FILE: &str = "Welcome.md";

pub const WORKSPACE_GUIDE: &str = r#"---
type: guide
status: example
---
# Welcome to your workspace

This folder is a plain-markdown vault. Every file in it is yours: edit notes
here, in any other editor, or hand one to an agent. Nothing leaves your
machine; it is all just files on disk.

Delete this note whenever you like. It comes back only if you run
"Workspace Guide" from the command palette (Cmd+K).

## Notes and links

Link a note by writing its name in double brackets, like [[My first note]].
Cmd+click follows a link; if the note does not exist yet, Agency offers to
create it. The panel on the right of every note lists its Backlinks (notes
that link here) and Mentions (issues that link here).

Tag a note by writing #tags anywhere in its body, then search them with `#`.

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
follow-ups. Reference any issue from a note by its key, written like
`[[AGE-14]]`, and that issue's detail pane will list the notes mentioning
it. If the workspace uses git, you can dispatch agents on writing tasks the
same way you would in a code project: they work on a branch, you merge.
"#;

/// Seed the guide note if it doesn't exist. Returns whether it was created.
pub fn ensure_guide(root: &Path) -> Result<bool> {
    let path = root.join(GUIDE_FILE);
    if path.exists() {
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
}
