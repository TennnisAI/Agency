//! Headless terminal emulator over a PTY byte stream.
//!
//! NOTE: API confirmed against `alacritty_terminal 0.26` in `term::smoke`
//! (Task 1). If `Term::new` / `Processor::advance` / grid indexing change with
//! a version bump, fix them here and in the smoke test together.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
// API correction vs. brief: `Processor` in vte 0.15 is generic over a Timeout
// handler (E0283 if type is omitted). Must use `Processor::<StdSyncHandler>::new()`.
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, StdSyncHandler};

const SCROLLBACK: usize = 10_000;

pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    pub cx: u16,
    pub cy: u16,
    pub data: Vec<u8>,
}

struct Dims {
    cols: usize,
    screen: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.screen + SCROLLBACK
    }
    fn screen_lines(&self) -> usize {
        self.screen
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Emulator {
    term: Term<VoidListener>,
    // API correction: `Processor<StdSyncHandler>` not bare `Processor`
    parser: Processor<StdSyncHandler>,
    cols: u16,
    rows: u16,
}

impl Emulator {
    pub fn new(cols: u16, rows: u16) -> Emulator {
        let dims = Dims {
            cols: cols as usize,
            screen: rows as usize,
        };
        let term = Term::new(Config::default(), &dims, VoidListener);
        Emulator {
            term,
            parser: Processor::<StdSyncHandler>::new(),
            cols,
            rows,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let dims = Dims {
            cols: cols as usize,
            screen: rows as usize,
        };
        self.term.resize(dims);
        self.cols = cols;
        self.rows = rows;
    }

    /// Plain text of the last `lines` rows (scrollback + screen), no escapes.
    ///
    /// Correction vs. brief: the brief used `start = rows - want` which
    /// returns the *bottom* N rows of the screen, missing content at row 0
    /// when the terminal hasn't scrolled yet.  The correct semantics are:
    /// "include all visible screen rows; extend into scrollback if lines > rows."
    /// `grid.total_lines()` returns actual history-in-use + screen (not max
    /// capacity), so `history = total - rows` is always safe.
    pub fn capture(&self, lines: usize) -> String {
        let grid = self.term.grid();
        let total = grid.total_lines();
        let history = total.saturating_sub(self.rows as usize) as i32;
        let want = lines.min(total);
        // Include scrollback only when more lines are requested than the screen holds.
        let scrollback_to_include = (want as i32 - self.rows as i32).max(0).min(history);
        let start = -scrollback_to_include;

        let mut out = String::new();
        for li in start..self.rows as i32 {
            let mut row = String::new();
            for col in 0..self.cols as usize {
                row.push(grid[Line(li)][Column(col)].c);
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    /// Reconstruct scrollback + screen as an ANSI repaint stream that, fed into a
    /// fresh emulator (or xterm.js), reproduces the current display.
    ///
    /// This is what a client gets when it (re)attaches. A full-screen TUI —
    /// cursor-agent, vim, anything on the alternate screen — never re-emits its
    /// setup sequences on reattach: it assumes the terminal is still in the mode
    /// it left it in and issues cursor-*relative* redraws from there. So the
    /// snapshot must faithfully restore that mode, not just the visible glyphs.
    /// Getting it wrong desyncs the display from where input actually lands: the
    /// cursor drifts to the bottom, typed characters don't appear (yet arrive at
    /// the child), and paste boundaries break.
    pub fn snapshot(&self) -> Snapshot {
        let mode = *self.term.mode();
        let alt = mode.contains(TermMode::ALT_SCREEN);
        let grid = self.term.grid();
        let total = grid.total_lines();
        let mut data: Vec<u8> = Vec::new();

        // Restore the screen buffer the child is drawing on. On the alternate
        // screen there is no scrollback; entering it (1049h) also clears + homes,
        // matching what the child did when it started. On the normal screen, clear
        // scrollback + screen so a stale client buffer can't bleed through.
        if alt {
            data.extend_from_slice(b"\x1b[?1049h\x1b[H");
        } else {
            data.extend_from_slice(b"\x1b[2J\x1b[3J\x1b[H");
        }

        // Alt screen has no scrollback to reconstruct; only the screen rows exist.
        let history = if alt { 0 } else { total.saturating_sub(self.rows as usize) as i32 };
        // Start at the first non-blank SCROLLBACK line (the grid reserves the
        // full scrollback capacity, most of which is blank); always emit the
        // whole screen (li >= 0) so its layout is preserved.
        // A cell is *padding* — an empty space with no color or attributes — as
        // opposed to real content (a glyph, or a blank with a colored background
        // like a status bar). Both the row-blank scan and the per-row trim use
        // this so a fully-colored bar row is never mistaken for empty.
        let is_padding = |li: i32, col: usize| {
            let cell = &grid[Line(li)][Column(col)];
            cell.c == ' '
                && cell.bg == Color::Named(NamedColor::Background)
                && cell.flags.is_empty()
        };
        let row_blank = |li: i32| (0..self.cols as usize).all(|col| is_padding(li, col));
        // Exclusive column past the last cell worth emitting on a row. Trailing
        // padding is dropped: emitting it pads every row out to the full grid
        // width, and if the client terminal is even one column narrower than the
        // emulator (the norm right after a reattach — sessions start at 220 cols
        // and the client fits to its own width) that padding wraps onto a new
        // line, inserting a blank line between every row (the "double-spaced on
        // reattach" bug) and burning scrollback 2-3x faster.
        let content_end = |li: i32| -> usize {
            (0..self.cols as usize)
                .rev()
                .find(|&col| !is_padding(li, col))
                .map_or(0, |col| col + 1)
        };
        let mut start = 0i32;
        for li in (-history)..0 {
            if !row_blank(li) {
                start = li;
                break;
            }
        }
        // Stop after the last non-blank line: emitting the trailing blank screen
        // rows (each with `\r\n`) scrolls blank lines into xterm's scrollback, the
        // blank space seen above non-full-screen agents. `end` is exclusive; an
        // entirely blank grid emits nothing but the clear + cursor position.
        let mut end = start;
        for li in (start..self.rows as i32).rev() {
            if !row_blank(li) {
                end = li + 1;
                break;
            }
        }
        let mut last_flags = Flags::empty();
        let mut last_fg = Color::Named(NamedColor::Foreground);
        let mut last_bg = Color::Named(NamedColor::Background);
        for li in start..end {
            // CRLF *between* rows, never after the last one: a trailing newline on
            // a full screen scrolls the top row into scrollback and shifts every
            // row up by one — the child then redraws relative to a screen that is
            // off by a line, which is the "cursor stuck at the bottom" desync.
            if li > start {
                data.extend_from_slice(b"\r\n");
                last_flags = Flags::empty();
                last_fg = Color::Named(NamedColor::Foreground);
                last_bg = Color::Named(NamedColor::Background);
            }
            for col in 0..content_end(li) {
                let cell = &grid[Line(li)][Column(col)];
                if cell.flags != last_flags || cell.fg != last_fg || cell.bg != last_bg {
                    data.extend_from_slice(sgr(cell.flags, cell.fg, cell.bg).as_bytes());
                    last_flags = cell.flags;
                    last_fg = cell.fg;
                    last_bg = cell.bg;
                }
                let mut buf = [0u8; 4];
                data.extend_from_slice(cell.c.encode_utf8(&mut buf).as_bytes());
            }
            data.extend_from_slice(b"\x1b[0m");
        }

        // Re-assert the input/display modes the child had turned on. These change
        // how keys are encoded and how the cursor is drawn, not the glyphs, so a
        // fresh client (which defaults them all off/visible) would otherwise send
        // mis-framed input and paint a stray hardware cursor.
        if mode.contains(TermMode::APP_CURSOR) {
            data.extend_from_slice(b"\x1b[?1h");
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            data.extend_from_slice(b"\x1b=");
        }
        if mode.contains(TermMode::BRACKETED_PASTE) {
            data.extend_from_slice(b"\x1b[?2004h");
        }

        let cur = self.term.grid().cursor.point;
        let cx = cur.column.0 as u16;
        // `cur.line.0` is i32 in alacritty 0.26 (negative = scrollback); clamp to 0.
        let cy = cur.line.0.max(0) as u16;
        // Position the cursor (1-based) after the repaint.
        data.extend_from_slice(format!("\x1b[{};{}H", cy + 1, cx + 1).as_bytes());
        // Hide the hardware cursor if the child had (full-screen TUIs draw their
        // own). A fresh client shows it by default, which is the phantom cursor
        // that appears where input isn't going.
        if !mode.contains(TermMode::SHOW_CURSOR) {
            data.extend_from_slice(b"\x1b[?25l");
        }

        Snapshot {
            cols: self.cols,
            rows: self.rows,
            cx,
            cy,
            data,
        }
    }
}

/// Build a minimal SGR sequence for the common attributes we reproduce.
fn sgr(flags: Flags, fg: Color, bg: Color) -> String {
    let mut codes: Vec<String> = vec!["0".into()];
    if flags.contains(Flags::BOLD) {
        codes.push("1".into());
    }
    if flags.contains(Flags::DIM) {
        codes.push("2".into());
    }
    if flags.contains(Flags::ITALIC) {
        codes.push("3".into());
    }
    if flags.contains(Flags::UNDERLINE) {
        codes.push("4".into());
    }
    if flags.contains(Flags::INVERSE) {
        codes.push("7".into());
    }
    push_color(&mut codes, fg, true);
    push_color(&mut codes, bg, false);
    format!("\x1b[{}m", codes.join(";"))
}

fn push_color(codes: &mut Vec<String>, c: Color, fg: bool) {
    match c {
        Color::Spec(rgb) => {
            codes.push(if fg { "38".into() } else { "48".into() });
            codes.push("2".into());
            codes.push(rgb.r.to_string());
            codes.push(rgb.g.to_string());
            codes.push(rgb.b.to_string());
        }
        Color::Indexed(i) => {
            codes.push(if fg { "38".into() } else { "48".into() });
            codes.push("5".into());
            codes.push(i.to_string());
        }
        Color::Named(_) => { /* default fg/bg already reset by leading 0 */ }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_returns_plain_text() {
        let mut e = Emulator::new(80, 24);
        e.feed(b"hello world");
        assert!(e.capture(5).contains("hello world"));
    }

    #[test]
    fn capture_includes_scrollback_lines() {
        let mut e = Emulator::new(80, 3);
        for i in 0..10 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        let cap = e.capture(20);
        assert!(cap.contains("line0"), "scrollback line missing: {cap:?}");
        assert!(cap.contains("line9"));
    }

    #[test]
    fn snapshot_round_trips_through_a_fresh_emulator() {
        let mut a = Emulator::new(40, 6);
        a.feed(b"alpha\r\nbeta\r\ngamma");
        let snap = a.snapshot();

        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert_eq!(b.capture(10).trim_end(), a.capture(10).trim_end());
    }

    #[test]
    fn snapshot_does_not_emit_blank_scrollback_capacity() {
        let mut e = Emulator::new(40, 6);
        e.feed(b"hello");
        let snap = e.snapshot();
        // Before the fix this emitted ~SCROLLBACK blank rows; a fresh 6-row screen
        // must produce at most ~rows lines.
        let newlines = snap.data.iter().filter(|&&b| b == b'\n').count();
        assert!(newlines <= 7, "snapshot emitted {newlines} lines (blank-padding bug)");
        // And it must still reproduce the content.
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert!(b.capture(10).contains("hello"));
    }

    #[test]
    fn snapshot_trims_trailing_blank_screen_rows() {
        // A part-filled screen (content at top, blank below) must NOT emit the
        // blank trailing rows: their `\r\n` scroll blank lines into xterm's
        // scrollback (the cause of blank space above non-full-screen agents).
        let mut e = Emulator::new(40, 20);
        e.feed(b"one\r\ntwo");
        let snap = e.snapshot();
        let newlines = snap.data.iter().filter(|&&b| b == b'\n').count();
        assert!(newlines <= 3, "snapshot emitted {newlines} rows for a 2-line screen (trailing blanks)");
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        let cap = b.capture(20);
        assert!(cap.contains("one") && cap.contains("two"), "content lost: {cap:?}");
    }

    #[test]
    fn snapshot_preserves_alternate_screen_mode() {
        // A full-screen TUI enters the alt screen and hides the cursor; on
        // reattach the snapshot must put a fresh client back into that mode, or
        // the child's cursor-relative redraws land on the wrong rows.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[?1049h\x1b[?25lpainted");
        let snap = e.snapshot();
        let text = String::from_utf8_lossy(&snap.data);
        assert!(text.contains("\x1b[?1049h"), "alt-screen enter missing: {text:?}");
        assert!(text.contains("\x1b[?25l"), "cursor-hide missing: {text:?}");
        // A round-trip client ends up on the alt screen too.
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert!(b.term.mode().contains(TermMode::ALT_SCREEN));
        assert!(b.capture(10).contains("painted"));
    }

    #[test]
    fn snapshot_reasserts_bracketed_paste() {
        // Bracketed paste governs how a paste is framed; a fresh client defaults
        // it off, so the child would mis-handle the paste boundary without this.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[?2004hprompt");
        let snap = e.snapshot();
        assert!(String::from_utf8_lossy(&snap.data).contains("\x1b[?2004h"));
    }

    #[test]
    fn snapshot_trims_trailing_padding_so_it_survives_a_narrower_client() {
        // Emulator is wide (like a fresh 220-col session); rows carry short
        // content. The snapshot must not pad rows out to the full width, or a
        // narrower client wraps the padding into blank lines between each row.
        let mut e = Emulator::new(220, 6);
        e.feed(b"one\r\ntwo\r\nthree");
        let snap = e.snapshot();
        // No run of padding spaces anywhere near the grid width.
        assert!(
            !snap.data.windows(40).any(|w| w.iter().all(|&b| b == b' ')),
            "snapshot still pads rows with trailing spaces",
        );
        // Replaying into a client only 10 cols wide keeps one row per line — no
        // spurious wraps. capture(3) returns exactly the three screen rows.
        let mut client = Emulator::new(10, 6);
        client.feed(&snap.data);
        let cap = client.capture(3);
        let lines: Vec<&str> = cap.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines, vec!["one", "two", "three"], "rows double-spaced or wrapped: {cap:?}");
    }

    #[test]
    fn snapshot_keeps_trailing_colored_cells() {
        // Trailing cells with a non-default background are a colored bar, not
        // padding — they must survive the trim. (Indexed color so the SGR
        // encoder actually reproduces it and we can assert on the bytes.)
        let mut e = Emulator::new(20, 3);
        // 256-color red background over two spaces at cols 0-1, then reset.
        e.feed(b"\x1b[48;5;196m  \x1b[0m");
        let snap = e.snapshot();
        assert!(
            String::from_utf8_lossy(&snap.data).contains("48;5;196"),
            "trailing colored background dropped by trim: {:?}",
            String::from_utf8_lossy(&snap.data),
        );
    }

    #[test]
    fn snapshot_of_full_screen_does_not_scroll_top_row_off() {
        // Every row filled: a trailing CRLF would scroll row 0 into scrollback and
        // shift the whole screen up one — the reattach cursor-drift bug. The first
        // row must survive on the reconstructed screen.
        let mut e = Emulator::new(10, 4);
        e.feed(b"row0\r\nrow1\r\nrow2\r\nrow3");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        // capture(4) returns exactly the 4 screen rows (no scrollback pulled in).
        let cap = b.capture(4);
        assert!(cap.contains("row0"), "top row scrolled off screen: {cap:?}");
        assert!(cap.contains("row3"));
    }
}
