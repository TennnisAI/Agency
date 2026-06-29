//! Pure helpers for deriving a short run title from a first prompt.

/// Normalize a model- or user-provided title: first line, trimmed, surrounding
/// quotes removed, internal whitespace collapsed, capped at 60 chars.
pub fn sanitize_title(raw: &str) -> String {
    let cleaned = strip_ansi_and_controls(raw);
    let line = cleaned.lines().next().unwrap_or("").trim();
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(60).collect()
}

/// Remove ESC-introduced escape sequences (CSI/OSC/other) and other C0 control
/// characters, keeping printable text and newlines.
fn strip_ansi_and_controls(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('\u{40}'..='\u{7e}').contains(&n) { break; }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if n == '\u{7}' { break; }
                        if n == '\x1b' { chars.next(); break; } // ST (ESC \)
                    }
                }
                _ => { chars.next(); }
            }
            continue;
        }
        if c == '\n' { out.push('\n'); continue; }
        if c.is_control() { continue; }
        out.push(c);
    }
    out
}

/// Best-effort offline title: the first several words of the first prompt.
pub fn fallback_title(first_prompt: &str) -> String {
    let line = first_prompt.lines().next().unwrap_or("").trim();
    let short = line.split_whitespace().take(8).collect::<Vec<_>>().join(" ");
    sanitize_title(&short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_quotes_and_collapses_ws() {
        assert_eq!(sanitize_title("  \"Add  hunk   staging\"  "), "Add hunk staging");
    }

    #[test]
    fn sanitize_takes_first_line_only() {
        assert_eq!(sanitize_title("Title here\nignored second line"), "Title here");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_title(&long).chars().count(), 60);
    }

    #[test]
    fn fallback_takes_first_words() {
        assert_eq!(
            fallback_title("implement hunk level staging in the git panel for real"),
            "implement hunk level staging in the git panel",
        );
    }

    #[test]
    fn fallback_handles_empty() {
        assert_eq!(fallback_title("   "), "");
    }

    #[test]
    fn sanitize_strips_escape_sequences_and_controls() {
        // An OSC reply + CSI reply with the ESC bytes intact must reduce to clean text.
        let raw = "\x1b]11;rgb:b3b3/bcbc/b2b2\x1b\\\x1b[?1016;2$yreal title";
        assert_eq!(sanitize_title(raw), "real title");
        // Pure escape noise (with ESC) reduces to empty.
        assert_eq!(sanitize_title("\x1b[?2027;0$y\x1b[?1004;h"), "");
    }
}
