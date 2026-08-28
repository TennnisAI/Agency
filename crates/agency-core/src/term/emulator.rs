//! Headless terminal emulator over a PTY byte stream.
//!
//! NOTE: API confirmed against `alacritty_terminal 0.26` in `term::smoke`
//! (Task 1). If `Term::new` / `Processor::advance` / grid indexing change with
//! a version bump, fix them here and in the smoke test together. The version is
//! pinned, with the client's alongside it and the reasoning for both, in
//! [`super::vt_pin`].

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
// API correction vs. brief: `Processor` in vte 0.15 is generic over a Timeout
// handler (E0283 if type is omitted). Must use `Processor::<StdSyncHandler>::new()`.
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, StdSyncHandler};

const SCROLLBACK: usize = 10_000;

/// The line-breaking controls xterm.js's `convertEol` applies to: LF, VT, FF.
/// See [`Emulator::feed`].
const LINE_BREAKS: [u8; 3] = [b'\n', 0x0b, 0x0c];

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
        let dims = Dims { cols: cols as usize, screen: rows as usize };
        let term = Term::new(Config::default(), &dims, VoidListener);
        Emulator { term, parser: Processor::<StdSyncHandler>::new(), cols, rows }
    }

    /// Feed PTY output, returning the carriage on every line break.
    ///
    /// The client is xterm.js with `convertEol` on, which resets the column on
    /// LF, VT and FF; alacritty, faithful to a real terminal, only moves down.
    /// The difference is invisible while a pane is live — the client is the
    /// only thing drawing — and surfaces the moment the pane is re-opened,
    /// because the reattach snapshot is generated from *this* grid. So a child
    /// that emits a bare LF (any TUI in raw mode, or a shell whose tty was left
    /// with `-onlcr` by one that died without restoring it) writes clean lines
    /// in the pane and a staircase in here:
    ///
    /// ```text
    /// one
    ///    two
    ///       three
    /// ```
    ///
    /// and the staircase is what the snapshot paints on return — the "banding"
    /// of AGE-67. It takes the cursor with it, too: the position the snapshot
    /// restores is the end of the staircase rather than where the child left
    /// it, so the child's next cursor-relative redraw lands somewhere else
    /// again. Matching the client is what keeps live and reattached agreeing.
    ///
    /// ANSI mode 20 (LNM) would be the tidy way to ask for this, but alacritty
    /// 0.26 only honours it for NEL — vte dispatches a C0 line feed straight to
    /// `linefeed()`, which doesn't consult the mode. Hence the carriage return
    /// spliced into the stream. It is safe to splice in mid-sequence: CR is a
    /// C0 control, so both parsers ignore it inside an OSC/DCS/APC string
    /// exactly as they ignore the LF it follows, and it can never fall inside a
    /// UTF-8 sequence (no continuation byte is 0x0a).
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut start = 0;
        for (i, b) in bytes.iter().enumerate() {
            if LINE_BREAKS.contains(b) {
                self.parser.advance(&mut self.term, &bytes[start..=i]);
                self.parser.advance(&mut self.term, b"\r");
                start = i + 1;
            }
        }
        self.parser.advance(&mut self.term, &bytes[start..]);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let dims = Dims { cols: cols as usize, screen: rows as usize };
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
    ///
    /// What comes back is *reconstructed* from the grid, never a recording of the
    /// bytes that built it, and that is the property that keeps a replay from
    /// typing into the user's shell. The client replays this stream into a live
    /// terminal whose input is wired straight to the pty, so a sequence in here
    /// that provokes a reply — a DSR cursor-position query, a device-attributes
    /// request, an OSC colour query — has its answer delivered to the child as if
    /// the user had typed it. Queries in the live stream are consumed by the
    /// parser and never reach a cell, so none can survive into here; the tests
    /// below hold that against a future "just replay the tail of the stream"
    /// shortcut, which is the tempting fidelity fix and would reintroduce it
    /// silently. The one sequence below that a client answers is `?1004h`, and
    /// the pane mutes its own input for the length of the replay to swallow it
    /// (`ui/src/components/FocusTerminal.tsx`).
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
            (0..self.cols as usize).rev().find(|&col| !is_padding(li, col)).map_or(0, |col| col + 1)
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
        // Autowrap (DECAWM). On by default, so this only ever has to turn it
        // *off*, and it matters because a child turns it off to write into the
        // last cell of a row without the cursor spilling onto the next one —
        // Copilot CLI does exactly that for the corner of a box. A client left
        // wrapping puts that glyph on a row of its own and every row below it
        // is one out.
        if !mode.contains(TermMode::LINE_WRAP) {
            data.extend_from_slice(b"\x1b[?7l");
        }
        // Mouse reporting, same story as the alt screen: the child turns it on
        // once at startup and never re-announces it. Losing it on reattach is
        // what made the scroll wheel type into the agent — a client that doesn't
        // know the app wants mouse events falls back to "alternate scroll",
        // translating each wheel notch into an Up/Down arrow, which a TUI reads
        // as prompt-history navigation. Order matters: protocol first, then the
        // encoding, mirroring how a child sets them up.
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            data.extend_from_slice(b"\x1b[?1000h");
        }
        if mode.contains(TermMode::MOUSE_DRAG) {
            data.extend_from_slice(b"\x1b[?1002h");
        }
        if mode.contains(TermMode::MOUSE_MOTION) {
            data.extend_from_slice(b"\x1b[?1003h");
        }
        if mode.contains(TermMode::UTF8_MOUSE) {
            data.extend_from_slice(b"\x1b[?1005h");
        }
        if mode.contains(TermMode::SGR_MOUSE) {
            data.extend_from_slice(b"\x1b[?1006h");
        }
        // Focus reporting: without it the child stops being told when the
        // window regains focus and can leave itself dimmed/paused.
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            data.extend_from_slice(b"\x1b[?1004h");
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

        Snapshot { cols: self.cols, rows: self.rows, cx, cy, data }
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
        Color::Named(n) => {
            if let Some(code) = named_sgr(n, fg) {
                codes.push(code.to_string());
            }
        }
    }
}

/// The SGR code for one of the sixteen palette colors: 30-37 / 90-97 as a
/// foreground, 40-47 / 100-107 as a background. `None` for a color that has no
/// SGR of its own, which the leading `0` has already restored.
///
/// This arm used to be `Color::Named(_) => {}` on the reasoning that a named
/// color is the default fg/bg. It is not: `Named` is also every color a child
/// asks for with a plain SGR 30-37/90-97/40-47, which is most of them, and
/// dropping those repainted them in the default foreground. So navigating away
/// from an agent and back — the only thing that replays a snapshot — turned the
/// whole pane, scrollback included, into flat white-on-black or black-on-white.
/// Only 256-color and truecolor output kept its color, which is why the loss
/// looked total for some agents and partial for others.
///
/// Emitted as the palette code rather than resolved to an index or an RGB
/// triple, so the pane keeps painting these cells from its own theme and a
/// theme switch after a reattach still moves them.
fn named_sgr(color: NamedColor, fg: bool) -> Option<u16> {
    use NamedColor as N;
    // The `Dim*` entries are the palette slots alacritty resolves a dimmed
    // color to; they have no SGR of their own, so emit the base color and let
    // the `2` that `sgr` writes for `Flags::DIM` carry the rest.
    let (index, bright) = match color {
        N::Black | N::DimBlack => (0, false),
        N::Red | N::DimRed => (1, false),
        N::Green | N::DimGreen => (2, false),
        N::Yellow | N::DimYellow => (3, false),
        N::Blue | N::DimBlue => (4, false),
        N::Magenta | N::DimMagenta => (5, false),
        N::Cyan | N::DimCyan => (6, false),
        N::White | N::DimWhite => (7, false),
        N::BrightBlack => (0, true),
        N::BrightRed => (1, true),
        N::BrightGreen => (2, true),
        N::BrightYellow => (3, true),
        N::BrightBlue => (4, true),
        N::BrightMagenta => (5, true),
        N::BrightCyan => (6, true),
        N::BrightWhite => (7, true),
        // The defaults, plus the two slots a cell never holds. Exhaustive on
        // purpose: `NamedColor` is not `#[non_exhaustive]`, so a version bump
        // that adds a color fails this build rather than silently losing it
        // the way the wildcard did.
        N::Foreground | N::Background | N::Cursor | N::BrightForeground | N::DimForeground => {
            return None;
        }
    };
    let base = match (fg, bright) {
        (true, false) => 30,
        (true, true) => 90,
        (false, false) => 40,
        (false, true) => 100,
    };
    Some(base + index)
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
    fn bare_line_feed_returns_the_carriage() {
        // AGE-67: the client (xterm.js, convertEol) resets the column on every
        // LF. Without matching it here, output from a child in raw mode walks
        // right across the grid — and the snapshot replays that staircase into
        // the pane the next time it is opened.
        let mut e = Emulator::new(40, 6);
        e.feed(b"one\ntwo\nthree");
        let cap = e.capture(6);
        let rows: Vec<&str> = cap.lines().take(3).collect();
        assert_eq!(rows, vec!["one", "two", "three"], "bare LF staircased");
    }

    #[test]
    fn line_feed_returns_the_carriage_across_reads_and_resets() {
        // Nothing about this may depend on where the PTY reads happen to split,
        // and `reset` (RIS) must not switch it off — the client's convertEol is
        // an option rather than a mode and survives its own reset.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1bcon");
        e.feed(b"e\n");
        e.feed(b"two");
        let cap = e.capture(6);
        let rows: Vec<&str> = cap.lines().take(2).collect();
        assert_eq!(rows, vec!["one", "two"]);
    }

    #[test]
    fn a_repaint_reads_the_same_however_the_pty_chunks_it() {
        // The daemon reads 4096 bytes at a time and a repaint can be split
        // anywhere in it — mid-sequence, mid-UTF-8. Splicing the carriage
        // returns in must not care where.
        let stream: &[u8] = "\x1b[?1049h\x1b[H\x1b[1;38;5;196mheader\x1b[0m\r\n\
             ╭─────╮\nrow \u{2502}two\u{2502}\x1b[K\n\x1b]0;title\x07\x1b[3;5Hx"
            .as_bytes();
        let mut whole = Emulator::new(30, 8);
        whole.feed(stream);
        let mut drip = Emulator::new(30, 8);
        for b in stream {
            drip.feed(std::slice::from_ref(b));
        }
        assert_eq!(whole.capture(8), drip.capture(8));
        assert_eq!(whole.snapshot().data, drip.snapshot().data);
    }

    #[test]
    fn a_line_feed_inside_an_osc_string_stays_inert() {
        // The carriage return is spliced in without parsing, so it has to be
        // harmless where the line feed it follows is: both are C0 controls the
        // parser ignores inside a string. A title set either side of one must
        // still land on screen unchanged.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b]0;ti\ntle\x07after");
        assert_eq!(e.capture(6).lines().next(), Some("after"));
    }

    #[test]
    fn snapshot_of_bare_line_feed_output_is_not_a_staircase() {
        let mut e = Emulator::new(60, 6);
        e.feed(b"Recycle mTLS Workloads 0:36\nRecycle mTLS Workloads 0:37\n");
        let snap = e.snapshot();
        let text = String::from_utf8_lossy(&snap.data);
        assert!(
            !text.contains("  Recycle"),
            "snapshot indents a line the pane drew at column 0: {text:?}",
        );
    }

    #[test]
    fn snapshot_reasserts_autowrap_when_the_child_turned_it_off() {
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[?7lcorner");
        let snap = e.snapshot();
        assert!(String::from_utf8_lossy(&snap.data).contains("\x1b[?7l"));
        // On (the default) it stays unsaid — nothing to restore.
        let mut e = Emulator::new(40, 6);
        e.feed(b"plain");
        assert!(!String::from_utf8_lossy(&e.snapshot().data).contains("\x1b[?7"));
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
        assert!(
            newlines <= 3,
            "snapshot emitted {newlines} rows for a 2-line screen (trailing blanks)"
        );
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
    fn snapshot_reasserts_mouse_reporting() {
        // The wheel-scrolls-history bug: an agent TUI enables mouse reporting at
        // startup, so the client sends wheel events. If the snapshot drops those
        // modes, the reattached client falls back to alternate-scroll arrow keys
        // and every scroll walks the prompt history instead.
        // (The three tracking protocols are mutually exclusive — 1002 replaces
        // 1000 — so this is what a client that sends wheel + drag looks like.)
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?1004hprompt");
        let snap = e.snapshot();
        let text = String::from_utf8_lossy(&snap.data);
        for seq in ["\x1b[?1002h", "\x1b[?1006h", "\x1b[?1004h"] {
            assert!(text.contains(seq), "mouse/focus mode {seq:?} missing: {text:?}");
        }
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        let m = *b.term.mode();
        assert!(m.contains(TermMode::MOUSE_DRAG));
        assert!(m.contains(TermMode::SGR_MOUSE));
        assert!(m.contains(TermMode::FOCUS_IN_OUT));

        // Plain click reporting (1000) survives on its own too.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[?1000h\x1b[?1006hprompt");
        let snap = e.snapshot();
        let text = String::from_utf8_lossy(&snap.data);
        assert!(text.contains("\x1b[?1000h"), "click reporting missing: {text:?}");
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert!(b.term.mode().contains(TermMode::MOUSE_REPORT_CLICK));
    }

    #[test]
    fn snapshot_omits_mouse_reporting_when_the_child_never_asked() {
        // A plain shell gets no mouse modes forced on it: enabling reporting
        // would swallow click-drag text selection in the client.
        let mut e = Emulator::new(40, 6);
        e.feed(b"$ ls");
        let text = String::from_utf8_lossy(&e.snapshot().data).to_string();
        assert!(!text.contains("\x1b[?100"), "unexpected mouse mode: {text:?}");
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
    fn snapshot_preserves_palette_colors() {
        // The colors-lost-on-return bug. A child that colors with plain SGR
        // 31/92/44 — most of them do — had every one of those cells rebuilt
        // with no color at all, so switching to another tab and back repainted
        // the pane and its whole scrollback in the default foreground: flat
        // white or black, depending on the theme.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[31mred\x1b[92mbright\x1b[44mon-blue\x1b[0mplain");
        let snap = e.snapshot();

        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        let cell = |col: usize| {
            let c = &b.term.grid()[Line(0)][Column(col)];
            (c.fg, c.bg)
        };
        let default_bg = Color::Named(NamedColor::Background);
        assert_eq!(cell(0), (Color::Named(NamedColor::Red), default_bg), "red lost");
        assert_eq!(cell(3), (Color::Named(NamedColor::BrightGreen), default_bg), "bright lost");
        assert_eq!(
            cell(9),
            (Color::Named(NamedColor::BrightGreen), Color::Named(NamedColor::Blue)),
            "background lost",
        );
        assert_eq!(
            cell(16),
            (Color::Named(NamedColor::Foreground), default_bg),
            "the reset after them was not reproduced",
        );
    }

    #[test]
    fn snapshot_keeps_a_trailing_bar_painted_with_a_palette_color() {
        // Same trim question as the 256-color case above, on the encoding an
        // ordinary status bar actually uses: two spaces on a blue background
        // are content, not padding, and must not be trimmed away.
        let mut e = Emulator::new(20, 3);
        e.feed(b"\x1b[44m  \x1b[0m");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert_eq!(b.term.grid()[Line(0)][Column(1)].bg, Color::Named(NamedColor::Blue));
    }

    #[test]
    fn snapshot_spells_out_no_color_for_default_text() {
        // The leading `0` of each SGR restores the default fg/bg, so naming
        // them would only bloat the replay — and would pin the cells to a
        // palette entry, which a later theme change could no longer move.
        let mut e = Emulator::new(40, 6);
        e.feed(b"plain");
        let text = String::from_utf8_lossy(&e.snapshot().data).to_string();
        assert!(!text.contains("\x1b[0;"), "default text carries color codes: {text:?}");
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

    /// Split `data` into its escape sequences, ignoring the printable text.
    ///
    /// Only the two shapes [`Emulator::snapshot`] emits are recognised: a CSI
    /// (`ESC [`, parameters, final byte) and a two-byte `ESC x`. Anything else —
    /// a DCS or OSC string, an unterminated CSI — comes back as one blob that
    /// fails the allowlist below, which is the point of scanning this way.
    fn escapes(data: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if data[i] != 0x1b {
                i += 1;
                continue;
            }
            let start = i;
            i += 1;
            if data.get(i) == Some(&b'[') {
                i += 1;
                while i < data.len() && !(0x40..=0x7e).contains(&data[i]) {
                    i += 1;
                }
            }
            i = (i + 1).min(data.len());
            out.push(data[start..i].to_vec());
        }
        out
    }

    /// Default-deny: the exact sequences `snapshot` declares it emits, plus the
    /// two parameterised shapes (SGR and cursor position). Nothing here provokes
    /// a reply from a client except `?1004h`; see the note on `snapshot`.
    fn is_declared(seq: &[u8]) -> bool {
        const EXACT: [&[u8]; 15] = [
            b"\x1b[?1049h",
            b"\x1b[2J",
            b"\x1b[3J",
            b"\x1b[?1h",
            b"\x1b=",
            b"\x1b[?2004h",
            b"\x1b[?7l",
            b"\x1b[?1000h",
            b"\x1b[?1002h",
            b"\x1b[?1003h",
            b"\x1b[?1005h",
            b"\x1b[?1006h",
            b"\x1b[?1004h",
            b"\x1b[?25l",
            b"\x1b[0m",
        ];
        if EXACT.contains(&seq) {
            return true;
        }
        let Some(body) = seq.strip_prefix(b"\x1b[") else { return false };
        let Some((&last, params)) = body.split_last() else { return false };
        params.iter().all(|b| b.is_ascii_digit() || *b == b';') && matches!(last, b'm' | b'H')
    }

    #[test]
    fn a_snapshot_emits_only_the_escape_sequences_it_declares() {
        let mut e = Emulator::new(30, 4);
        e.feed(
            b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?2004h\x1b[?7l\x1b[?1h\x1b=\
              \x1b[?25l\x1b[1;38;5;196mbar\x1b[0m\r\nrow",
        );
        for seq in escapes(&e.snapshot().data) {
            assert!(
                is_declared(&seq),
                "snapshot emitted an undeclared sequence {:?}",
                String::from_utf8_lossy(&seq),
            );
        }
    }

    #[test]
    fn a_device_query_in_the_stream_never_reaches_the_snapshot() {
        // A client replays the snapshot into a terminal whose input goes to the
        // pty, so a query that survived into it would be answered *by the client*
        // and the answer would arrive at the child as typed input — a stray
        // `\x1b[24;1R` in the prompt, or a shell running whatever the reply
        // happened to spell. Nothing that provokes a reply may reach the replay.
        let mut e = Emulator::new(40, 4);
        e.feed(
            b"before\
              \x1b[6n\x1b[5n\x1b[?6n\
              \x1b[c\x1b[>c\x1b[=c\x1bZ\
              \x1b[?2004$p\x1b[18t\x1b[>0q\
              \x1b]10;?\x07\x1b]11;?\x1b\\\
              \x1bP+q544e\x1b\\\
              after",
        );
        let snap = e.snapshot();
        for seq in escapes(&snap.data) {
            assert!(
                is_declared(&seq),
                "a device query survived into the snapshot: {:?}",
                String::from_utf8_lossy(&seq),
            );
        }
        // And the surrounding text is still there, so the queries were really
        // parsed away rather than the whole feed being swallowed.
        let text = String::from_utf8_lossy(&snap.data);
        assert!(text.contains("before") && text.contains("after"), "{text:?}");
    }
}
