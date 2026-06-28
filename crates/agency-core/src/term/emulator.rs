//! Headless terminal emulator over a PTY byte stream.
//!
//! NOTE: API confirmed against `alacritty_terminal 0.26` in `term::smoke`
//! (Task 1). If `Term::new` / `Processor::advance` / grid indexing change with
//! a version bump, fix them here and in the smoke test together.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
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
    pub fn snapshot(&self) -> Snapshot {
        let grid = self.term.grid();
        let total = grid.total_lines();
        let mut data: Vec<u8> = Vec::new();
        // Clear screen + scrollback + home, then emit all lines.
        data.extend_from_slice(b"\x1b[2J\x1b[3J\x1b[H");

        let history = (total - self.rows as usize) as i32;
        let mut last_flags = Flags::empty();
        let mut last_fg = Color::Named(NamedColor::Foreground);
        let mut last_bg = Color::Named(NamedColor::Background);
        for li in (-history)..self.rows as i32 {
            for col in 0..self.cols as usize {
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
            data.extend_from_slice(b"\x1b[0m\r\n");
            last_flags = Flags::empty();
            last_fg = Color::Named(NamedColor::Foreground);
            last_bg = Color::Named(NamedColor::Background);
        }

        let cur = self.term.grid().cursor.point;
        let cx = cur.column.0 as u16;
        // `cur.line.0` is i32 in alacritty 0.26 (negative = scrollback); clamp to 0.
        let cy = cur.line.0.max(0) as u16;
        // Position the cursor (1-based) after the repaint.
        data.extend_from_slice(format!("\x1b[{};{}H", cy + 1, cx + 1).as_bytes());

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
}
