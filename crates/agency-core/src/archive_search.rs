//! Search over the archive: which archived runs a query matches, and the
//! line that explains each match.
//!
//! An archived run is three texts: its summary (the row's label, plus its
//! title, branch, and its issue's key and title), its record, and its
//! conversation.
//! The archive row shows only the start of its label, so a query that matched
//! anything else and came back as a bare row would leave the user to open
//! runs one at a time looking for why. Every hit therefore carries a snippet
//! unless the visible part of the row already explains it.
//!
//! Pure: texts in, a verdict out. Finding and parsing the texts, and caching
//! the parse, is the app's.

use serde::Serialize;

/// Where a match was found, in the order the archive row reveals them: the
/// summary is on or about the row, the record and conversation behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    Summary,
    Record,
    Conversation,
}

/// The matched text with a little context either side, already trimmed to
/// one line, so the UI can mark the match without searching again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    pub before: String,
    pub hit: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub field: Field,
    /// `None` when the row's visible label already shows the match.
    pub snippet: Option<Snippet>,
}

/// One archived run's searchable texts.
pub struct Haystack<'a> {
    /// The text the archive row shows, as it shows it.
    pub label: &'a str,
    /// The rest of what names the run: title, branch, issue key and title.
    pub summary: &'a [&'a str],
    pub record: Option<&'a str>,
    /// One entry per rendered turn, tool lines included: "which run ran the
    /// migration" is answered by a tool line, not by anything said.
    pub conversation: &'a [&'a str],
}

/// Characters of context kept either side of the match.
const BEFORE_CHARS: usize = 40;
const AFTER_CHARS: usize = 80;

/// How much of a row's label the sidebar shows before its ellipsis, at the
/// default width. A match past it is as hidden as one in the conversation:
/// the row would come back looking unrelated to the query.
const LABEL_VISIBLE_CHARS: usize = 32;

/// Case-fold the way every comparison here does. Per char rather than
/// `str::to_lowercase`, which special-cases a word-final sigma: the two would
/// disagree about what matched, and the snippet would then fail to find the
/// match the filter had just accepted.
fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// The query's terms, folded and deduplicated. Whitespace separates them;
/// a run matches only if every term occurs somewhere in it, so adding a word
/// narrows the list the way it does in every other search box.
pub fn terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in query.split_whitespace().map(fold) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Whether `terms` match this run, and where to show why. `None` for no
/// terms: an empty query filters nothing, which is the caller's to handle.
pub fn search(terms: &[String], hay: &Haystack) -> Option<Hit> {
    if terms.is_empty() {
        return None;
    }
    let visible = fold(&hay.label.chars().take(LABEL_VISIBLE_CHARS).collect::<String>());
    let label = fold(hay.label);
    let summary: Vec<String> = hay.summary.iter().map(|s| fold(s)).collect();
    let record = hay.record.map(fold);
    let conversation: Vec<String> = hay.conversation.iter().map(|s| fold(s)).collect();
    let in_summary = |t: &str| label.contains(t) || summary.iter().any(|s| s.contains(t));
    let in_record = |t: &str| record.as_deref().is_some_and(|r| r.contains(t));
    let in_conversation = |t: &str| conversation.iter().any(|s| s.contains(t));
    if !terms.iter().all(|t| in_summary(t) || in_record(t) || in_conversation(t)) {
        return None;
    }
    // The snippet goes to the first term the visible row does not explain.
    // With none such the row explains the whole match and needs no help.
    let Some(term) = terms.iter().find(|t| !visible.contains(t.as_str())) else {
        return Some(Hit { field: Field::Summary, snippet: None });
    };
    if in_summary(term) {
        let text = std::iter::once(hay.label)
            .chain(hay.summary.iter().copied())
            .find(|s| fold(s).contains(term.as_str()));
        return Some(Hit { field: Field::Summary, snippet: text.and_then(|t| snippet(t, term)) });
    }
    if let Some(text) = hay.record.filter(|_| in_record(term)) {
        return Some(Hit { field: Field::Record, snippet: snippet(text, term) });
    }
    let text = hay.conversation.iter().zip(&conversation).find(|(_, f)| f.contains(term.as_str()));
    Some(Hit { field: Field::Conversation, snippet: text.and_then(|(t, _)| snippet(t, term)) })
}

/// Byte range of the first case-insensitive occurrence of `needle` (already
/// folded) in `hay`, in `hay`'s own bytes. Folding can change a char's length
/// (`İ` folds to two chars), so the range is found by walking the original
/// rather than by indexing into a folded copy.
fn find_folded(hay: &str, needle: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return None;
    }
    'start: for (start, _) in hay.char_indices() {
        let mut k = 0;
        for (i, c) in hay[start..].char_indices() {
            for l in c.to_lowercase() {
                if k == needle.len() {
                    break;
                }
                if l != needle[k] {
                    continue 'start;
                }
                k += 1;
            }
            if k == needle.len() {
                return Some((start, start + i + c.len_utf8()));
            }
        }
        // Ran out of text mid-match, and every later start has less of it.
        return None;
    }
    None
}

/// The match in `text` with context either side, on one line.
fn snippet(text: &str, term: &str) -> Option<Snippet> {
    let (start, end) = find_folded(text, term)?;
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let before: Vec<char> = text[..start].chars().collect();
    let cut = before.len().saturating_sub(BEFORE_CHARS);
    let mut b = one_line(&before[cut..].iter().collect::<String>());
    if cut > 0 {
        b = format!("…{b}");
    }
    // Keep the space between the context and the match, which `one_line`
    // trims from the context's end.
    if text[..start].ends_with(char::is_whitespace) && !b.is_empty() {
        b.push(' ');
    }
    let rest = &text[end..];
    let mut a: String = rest.chars().take(AFTER_CHARS).collect();
    let clipped = rest.chars().count() > AFTER_CHARS;
    let lead = a.starts_with(char::is_whitespace);
    a = one_line(&a);
    if lead && !a.is_empty() {
        a.insert(0, ' ');
    }
    if clipped {
        a.push('…');
    }
    Some(Snippet { before: b, hit: one_line(&text[start..end]), after: a })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hay<'a>(
        label: &'a str,
        record: Option<&'a str>,
        conversation: &'a [&'a str],
    ) -> Haystack<'a> {
        Haystack { label, summary: &[], record, conversation }
    }

    #[test]
    fn terms_are_folded_split_and_deduplicated() {
        assert_eq!(terms("  Merge  CONFLICT merge "), vec!["merge", "conflict"]);
        assert!(terms("   ").is_empty());
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        assert_eq!(search(&[], &hay("anything", None, &[])), None);
    }

    #[test]
    fn a_match_the_row_shows_needs_no_snippet() {
        let hit = search(&terms("sidebar"), &hay("claude: Fix the Sidebar", None, &[])).unwrap();
        assert_eq!(hit, Hit { field: Field::Summary, snippet: None });
    }

    #[test]
    fn a_match_past_the_rows_ellipsis_shows_a_snippet() {
        let label = "claude: Fix the sidebar so that it collapses when the window narrows";
        let hit = search(&terms("narrows"), &hay(label, None, &[])).unwrap();
        assert_eq!(hit.field, Field::Summary);
        assert_eq!(hit.snippet.unwrap().hit, "narrows");
    }

    #[test]
    fn a_match_in_a_name_the_row_does_not_show_shows_a_snippet() {
        let h = Haystack {
            label: "claude: fix it",
            summary: &["agent/age-212-x9a1", "AGE-212", "Serialize the merge queue"],
            record: None,
            conversation: &[],
        };
        let hit = search(&terms("age-212"), &h).unwrap();
        assert_eq!(hit.field, Field::Summary);
        assert_eq!(hit.snippet.unwrap().before, "agent/");
        // The issue is remembered by its title as often as by its key.
        let hit = search(&terms("serialize"), &h).unwrap();
        assert_eq!(hit.field, Field::Summary);
        assert_eq!(hit.snippet.unwrap().hit, "Serialize");
    }

    #[test]
    fn a_conversation_match_says_where_and_shows_the_line() {
        let turns = ["fix the bug", "The race is in the PTY reader, which drops a byte."];
        let hit = search(&terms("pty"), &hay("claude: Fix the bug", None, &turns)).unwrap();
        assert_eq!(hit.field, Field::Conversation);
        let s = hit.snippet.unwrap();
        assert_eq!(s.before, "The race is in the ");
        assert_eq!(s.hit, "PTY");
        assert_eq!(s.after, " reader, which drops a byte.");
    }

    #[test]
    fn the_record_is_preferred_to_the_conversation() {
        let hit = search(
            &terms("migration"),
            &hay("claude: title", Some("Ran the migration."), &["the migration ran"]),
        )
        .unwrap();
        assert_eq!(hit.field, Field::Record);
    }

    #[test]
    fn every_term_must_match_but_each_may_match_anywhere() {
        let h = hay("claude: Batch edit issues", Some("merged into main"), &["ran cargo test"]);
        let hit = search(&terms("batch cargo"), &h).unwrap();
        // The row explains "batch"; the snippet explains the other term.
        assert_eq!(hit.field, Field::Conversation);
        assert_eq!(hit.snippet.unwrap().hit, "cargo");
        assert_eq!(search(&terms("batch nowhere"), &h), None);
    }

    #[test]
    fn long_context_is_clipped_and_collapsed_to_one_line() {
        let text = format!("{}\n\nthe needle\nis {}", "a ".repeat(50), "b ".repeat(60));
        let s = search(&terms("needle"), &hay("", None, &[&text])).unwrap().snippet.unwrap();
        assert!(s.before.starts_with('…'), "{:?}", s.before);
        assert!(s.before.ends_with("the "), "{:?}", s.before);
        assert!(s.after.ends_with('…'));
        assert!(!s.after.contains('\n'));
        assert_eq!(s.hit, "needle");
    }

    #[test]
    fn a_match_whose_fold_changes_length_is_still_cut_on_the_original() {
        // `İ` folds to two chars; slicing a folded copy's offsets into the
        // original would land mid-char and panic.
        let s = search(&terms("i̇stanbul"), &hay("", None, &["Flew to İstanbul today"]))
            .unwrap()
            .snippet
            .unwrap();
        assert_eq!(s.hit, "İstanbul");
        assert_eq!(s.before, "Flew to ");
        assert_eq!(s.after, " today");
    }

    #[test]
    fn a_false_start_does_not_hide_a_later_match() {
        let s = search(&terms("abc"), &hay("", None, &["ababc"])).unwrap().snippet.unwrap();
        assert_eq!((s.before.as_str(), s.hit.as_str()), ("ab", "abc"));
    }
}
