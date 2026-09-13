//! `set_status`: the one line an agent writes about itself onto the board
//! (AGE-208). The board could say an agent was "working, 4m" and never what
//! on; the agent always knew and had no way to say.
//!
//! Everything here is pure. The text is agent-authored, so it is untrusted
//! display data: it is cleaned into one short plain line here, rendered as
//! text and never as markup or a link by the UI, and never fed back into
//! anything injected into an agent's context. It is decoration over the run's
//! real state, not a substitute for it.

/// The longest status the board keeps, in characters. A tile shows one line;
/// anything past this is prose, and prose belongs in the pane.
pub const MAX_CHARS: usize = 80;

/// What a `set_status` call asks for once its text is cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing left to show: the call clears the line.
    Clear,
    /// The line to show, and whether it had to be cut to fit.
    Set { text: String, truncated: bool },
}

/// Clean agent-written text into one line the board can show.
///
/// Newlines, tabs and every other control character become spaces, and runs of
/// whitespace collapse, so a multi-line argument cannot push the tile out of
/// shape. Bidirectional overrides and isolates are dropped outright: they are
/// how a line reads one way on screen and holds another, and a status has no
/// use for them. Then it is cut to [`MAX_CHARS`] on a character boundary, with
/// an ellipsis so the board does not present a cut line as the whole one.
pub fn clean(raw: &str) -> Status {
    let mut line = String::with_capacity(raw.len().min(MAX_CHARS * 4));
    let mut pending_space = false;
    for c in raw.chars() {
        if is_bidi_control(c) {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            pending_space = !line.is_empty();
            continue;
        }
        if pending_space {
            line.push(' ');
            pending_space = false;
        }
        line.push(c);
    }
    if line.is_empty() {
        return Status::Clear;
    }
    match line.char_indices().nth(MAX_CHARS - 1) {
        // Room for MAX_CHARS - 1 characters and the ellipsis.
        Some((cut, _)) if line.chars().count() > MAX_CHARS => {
            let mut text = line[..cut].trim_end().to_string();
            text.push('\u{2026}');
            Status::Set { text, truncated: true }
        }
        _ => Status::Set { text: line, truncated: false },
    }
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// What the agent hears back. Never echoes the text: it is the agent's own,
/// and agent-facing copy stays ASCII.
pub fn reply(status: &Status) -> String {
    match status {
        Status::Clear => "Status cleared. The board shows only your run's state now.".to_string(),
        Status::Set { truncated: false, .. } => {
            "Status set. It stays on the board until you set another or clear it with an empty \
             string."
                .to_string()
        }
        Status::Set { truncated: true, .. } => format!(
            "Status set, but cut to {MAX_CHARS} characters to fit the board. Keep it to a short \
             phrase."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str) -> Status {
        Status::Set { text: text.to_string(), truncated: false }
    }

    #[test]
    fn a_short_phrase_is_kept_as_written() {
        assert_eq!(clean("running the migration tests"), set("running the migration tests"));
        assert_eq!(clean("rewriting `notifier.rs`"), set("rewriting `notifier.rs`"));
    }

    #[test]
    fn empty_or_blank_text_clears() {
        assert_eq!(clean(""), Status::Clear);
        assert_eq!(clean("  \n\t "), Status::Clear);
        assert_eq!(clean("\u{202E}\u{2066}"), Status::Clear);
    }

    #[test]
    fn it_is_always_one_line() {
        assert_eq!(clean("  waiting on\n\n npm\tinstall \r\n"), set("waiting on npm install"));
        assert_eq!(clean("a\u{0}b\u{1b}[31mc"), set("a b [31mc"));
    }

    /// A right-to-left override makes "exe.txt" display as "txt.exe". The
    /// status is shown next to the run's own name, so it may not reorder text.
    #[test]
    fn bidi_controls_are_dropped() {
        assert_eq!(clean("fixing \u{202E}txt.exe"), set("fixing txt.exe"));
        assert_eq!(clean("a\u{2067}b\u{2069}c\u{200F}"), set("abc"));
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary_with_an_ellipsis() {
        let exact = "x".repeat(MAX_CHARS);
        assert_eq!(clean(&exact), set(&exact), "exactly the cap is not cut");

        let long = "é".repeat(MAX_CHARS + 20);
        let Status::Set { text, truncated } = clean(&long) else { panic!("expected a status") };
        assert!(truncated);
        assert_eq!(text.chars().count(), MAX_CHARS);
        assert!(text.ends_with('\u{2026}'), "{text}");

        // A cut landing on a space does not leave one before the ellipsis.
        let words = format!("{} tail of the sentence", "y".repeat(MAX_CHARS - 2));
        let Status::Set { text, .. } = clean(&words) else { panic!("expected a status") };
        assert!(!text.contains(" \u{2026}"), "{text}");
    }

    #[test]
    fn replies_are_ascii_and_never_echo_the_text() {
        for s in [Status::Clear, set("secret plan"), clean(&"z".repeat(200))] {
            let r = reply(&s);
            assert!(r.is_ascii(), "{r}");
            assert!(!r.contains("secret plan"), "{r}");
        }
    }
}
