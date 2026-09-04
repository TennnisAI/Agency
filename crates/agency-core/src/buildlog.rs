//! The tail of a long-running command's output, kept in memory for a UI that
//! polls rather than streams.
//!
//! A knowledge-graph build runs for minutes: `graphify . --backend claude-cli`
//! was still going 9 minutes in with the settings panel showing "Building the
//! graph" and nothing else, which reads as a hung app rather than a running
//! build (AGE-180). The build's own progress lines are the answer, and this is
//! where they are parked between polls.
//!
//! Bytes in, display lines out, with a bounded ring so a build that prints for
//! an hour costs the same as one that prints for a second. Pure: reading the
//! child's pipes is the caller's job, so the splitting is testable without
//! spawning anything.

use std::collections::VecDeque;

/// Which of a child's two pipes a line arrived on. Kept per line because a
/// failure's reason is on stderr and its progress noise is on stdout, and
/// [`BuildLog::failure_tail`] has to be able to tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// Lines kept by default: enough that a failure's traceback survives the
/// progress lines printed after it, small enough to hold for every project.
pub const DEFAULT_LINES: usize = 200;

#[derive(Debug, Clone)]
pub struct BuildLog {
    lines: VecDeque<(Stream, String)>,
    cap: usize,
    /// Bytes of a line that has not been terminated yet, per stream. Held as
    /// bytes, not text: a pipe read splits wherever the buffer ends, and
    /// decoding each chunk on its own turns a multi-byte character straddling
    /// that boundary into two replacement characters.
    partial_out: Vec<u8>,
    partial_err: Vec<u8>,
}

impl Default for BuildLog {
    fn default() -> Self {
        Self::new(DEFAULT_LINES)
    }
}

impl BuildLog {
    pub fn new(cap: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            cap: cap.max(1),
            partial_out: Vec::new(),
            partial_err: Vec::new(),
        }
    }

    /// Feed a chunk read off one of the child's pipes. Complete lines land in
    /// the ring; whatever trails the last terminator waits for the next chunk.
    pub fn push(&mut self, stream: Stream, chunk: &[u8]) {
        let partial = match stream {
            Stream::Out => &mut self.partial_out,
            Stream::Err => &mut self.partial_err,
        };
        partial.extend_from_slice(chunk);
        // A carriage return terminates a line here as well as a newline. A
        // progress counter rewritten in place ("120/500 files") is one line
        // that never ends, and holding it as a partial would show the user
        // nothing at all until the build finished.
        let mut done: Vec<Vec<u8>> = Vec::new();
        let mut start = 0;
        for (i, b) in partial.iter().enumerate() {
            if *b == b'\n' || *b == b'\r' {
                done.push(partial[start..i].to_vec());
                start = i + 1;
            }
        }
        partial.drain(..start);
        for raw in done {
            self.append(stream, &raw);
        }
    }

    /// Take the unterminated remainder of a stream that has reached EOF. A
    /// command killed mid-line still said something, and its last words are
    /// usually the interesting ones.
    pub fn finish(&mut self, stream: Stream) {
        let partial = match stream {
            Stream::Out => std::mem::take(&mut self.partial_out),
            Stream::Err => std::mem::take(&mut self.partial_err),
        };
        self.append(stream, &partial);
    }

    fn append(&mut self, stream: Stream, raw: &[u8]) {
        let text = strip_control(&String::from_utf8_lossy(raw));
        let text = text.trim_end();
        if text.trim().is_empty() {
            return;
        }
        if self.lines.len() >= self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back((stream, text.to_string()));
    }

    /// Every kept line, oldest first, both streams interleaved in the order
    /// they were read.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().map(|(_, t)| t.clone()).collect()
    }

    /// The last `n` kept lines, oldest first.
    pub fn tail(&self, n: usize) -> Vec<String> {
        self.lines.iter().skip(self.lines.len().saturating_sub(n)).map(|(_, t)| t.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The last `n` lines a failed command wrote, preferring stderr and falling
    /// back to stdout: enough to show *why* a build failed without pasting a
    /// whole log into the settings panel. `None` when it said nothing.
    pub fn failure_tail(&self, n: usize) -> Option<String> {
        let err: Vec<&str> =
            self.lines.iter().filter(|(s, _)| *s == Stream::Err).map(|(_, t)| t.as_str()).collect();
        let all: Vec<&str> = self.lines.iter().map(|(_, t)| t.as_str()).collect();
        [err, all].into_iter().find_map(|lines| {
            let tail = lines[lines.len().saturating_sub(n)..].join("\n");
            (!tail.is_empty()).then_some(tail)
        })
    }
}

/// Drop ANSI escape sequences and stray control bytes, so a build that colours
/// its output does not paint escape codes into the settings panel.
fn strip_control(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameter and intermediate bytes, then a final @ to ~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: runs to a BEL or to the two-byte string terminator.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Anything else is a two-character escape; both are dropped.
                _ => {}
            },
            '\t' => out.push(' '),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(chunks: &[(Stream, &str)]) -> BuildLog {
        let mut l = BuildLog::default();
        for (s, c) in chunks {
            l.push(*s, c.as_bytes());
        }
        l
    }

    #[test]
    fn holds_a_partial_line_until_it_is_terminated() {
        let mut l = BuildLog::default();
        l.push(Stream::Out, b"AST extract");
        assert!(l.is_empty(), "half a line is not a line");
        l.push(Stream::Out, b"ion: 1/2\nnext");
        assert_eq!(l.lines(), ["AST extraction: 1/2"]);
        l.finish(Stream::Out);
        assert_eq!(l.lines(), ["AST extraction: 1/2", "next"]);
    }

    #[test]
    fn a_carriage_return_ends_a_line_too() {
        // A progress counter rewritten in place never sends a newline; each
        // frame has to become its own line or the panel shows nothing.
        let l = log(&[(Stream::Out, "10%\r50%\r100%\r\ndone\n")]);
        assert_eq!(l.lines(), ["10%", "50%", "100%", "done"], "CRLF is one terminator, not two");
    }

    #[test]
    fn keeps_the_newest_lines_within_the_cap() {
        let mut l = BuildLog::new(3);
        for i in 1..=5 {
            l.push(Stream::Out, format!("line {i}\n").as_bytes());
        }
        assert_eq!(l.lines(), ["line 3", "line 4", "line 5"]);
        assert_eq!(l.tail(2), ["line 4", "line 5"]);
        assert_eq!(l.tail(99).len(), 3, "asking for more than is kept is not an error");
    }

    #[test]
    fn strips_escapes_and_skips_blank_lines() {
        let l =
            log(&[(Stream::Out, "\u{1b}[32mgreen\u{1b}[0m\n\n   \n\u{1b}]0;title\u{7}plain\n")]);
        assert_eq!(l.lines(), ["green", "plain"]);
    }

    #[test]
    fn decodes_a_character_split_across_two_reads() {
        let mut l = BuildLog::default();
        let bytes = "café\n".as_bytes();
        l.push(Stream::Out, &bytes[..4]);
        l.push(Stream::Out, &bytes[4..]);
        assert_eq!(l.lines(), ["café"]);
    }

    #[test]
    fn failure_tail_prefers_stderr_and_keeps_the_last_lines() {
        let l = log(&[(Stream::Err, "boom\n"), (Stream::Out, "noise\n")]);
        assert_eq!(l.failure_tail(4).as_deref(), Some("boom"));
        let l = log(&[(Stream::Out, "only stdout\n")]);
        assert_eq!(l.failure_tail(4).as_deref(), Some("only stdout"));
        let l = log(&[(Stream::Err, "  \n\n"), (Stream::Out, "  \n")]);
        assert_eq!(l.failure_tail(4), None, "whitespace is not a reason");
        let l = log(&[(Stream::Err, "a\nb\nc\nd\ne\nf\n")]);
        assert_eq!(l.failure_tail(4).as_deref(), Some("c\nd\ne\nf"));
    }

    #[test]
    fn the_two_streams_interleave_in_read_order() {
        let l = log(&[(Stream::Out, "one\n"), (Stream::Err, "two\n"), (Stream::Out, "three\n")]);
        assert_eq!(l.lines(), ["one", "two", "three"]);
    }
}
