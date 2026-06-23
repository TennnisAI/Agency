//! Pure helpers for deriving a short run title from a first prompt.

/// Normalize a model- or user-provided title: first line, trimmed, surrounding
/// quotes removed, internal whitespace collapsed, capped at 60 chars.
pub fn sanitize_title(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or("").trim();
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(60).collect()
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
}
