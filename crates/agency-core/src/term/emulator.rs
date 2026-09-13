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
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
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
        let dims = Dims { cols: cols as usize, screen: rows as usize };
        let term = Term::new(Config::default(), &dims, VoidListener);
        Emulator { term, parser: Processor::<StdSyncHandler>::new(), cols, rows }
    }

    /// Feed PTY output.
    ///
    /// A bare line feed moves down and holds the column, the way a real
    /// terminal does. It used to have a carriage return spliced in after it, to
    /// match xterm.js's `convertEol` in the pane; both are gone. The pty is
    /// opened with the default termios, so `ONLCR` is on and the kernel has
    /// already turned every `\n` a cooked-mode child writes into `\r\n` by the
    /// time it reaches us. A bare LF can therefore only come from a child that
    /// turned `ONLCR` off — a TUI in raw mode — and there it means "down one
    /// row, same column", which is exactly what returning the carriage
    /// destroyed.
    ///
    /// AGE-204: crush blanks the area behind its command palette by erasing a
    /// run of cells and stepping down while holding the column,
    /// `\x1b[69X\n\x1b[69X\n`. With the carriage returned every erase after the
    /// first landed at column 0 instead: the palette's rows lost their
    /// left-hand labels, shreds of the sidebar behind it ("LSPs" as `l`, then
    /// `Ldes  e`) were left down the left edge of the pane, and the rows below
    /// drifted apart. Reproduced from a captured crush stream; the pattern is
    /// in `tests::a_bare_line_feed_holds_the_column`.
    ///
    /// The AGE-67 banding this replaces was the two sides *disagreeing*, not
    /// the carriage return itself: the pane converted, the daemon did not, so a
    /// re-opened pane painted a staircase the live one had never shown. They
    /// agree again here, on the faithful reading, so a pane and its reattach
    /// both show whatever the child really drew.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
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
        self.render(lines, false).0
    }

    /// [`capture`](Self::capture)'s text, with a parallel map of how the child
    /// drew each cell.
    ///
    /// The map has the same rows and the same columns as the text: row `i` of
    /// one lines up character-for-character with row `i` of the other, so a
    /// column index found in the text indexes the map directly. Per cell:
    ///
    /// - `' '` the cell is blank,
    /// - `'d'` faint (SGR 2),
    /// - `'i'` inverse (SGR 7),
    /// - `'.'` drawn plainly.
    ///
    /// This exists to tell an agent's own painted hint text from the human's
    /// half-typed draft, which read identically as glyphs. See
    /// `agency-app`'s `sendq::prompt_looks_empty`, which is the only caller.
    pub fn capture_styled(&self, lines: usize) -> (String, String) {
        self.render(lines, true)
    }

    /// Both captures. `want_style` off leaves the second string empty rather
    /// than building a per-cell map for a caller that discards it: a plain
    /// capture is the common one by far (see `ClientMsg::Capture::style`).
    fn render(&self, lines: usize, want_style: bool) -> (String, String) {
        let grid = self.term.grid();
        let total = grid.total_lines();
        let history = total.saturating_sub(self.rows as usize) as i32;
        let want = lines.min(total);
        // Include scrollback only when more lines are requested than the screen holds.
        let scrollback_to_include = (want as i32 - self.rows as i32).max(0).min(history);
        let start = -scrollback_to_include;

        let mut out = String::new();
        let mut styles = String::new();
        for li in start..self.rows as i32 {
            let mut row = String::new();
            let mut style = String::new();
            for col in 0..self.cols as usize {
                let cell = &grid[Line(li)][Column(col)];
                row.push(cell.c);
                if want_style {
                    style.push(if cell.c == ' ' {
                        ' '
                    } else if cell.flags.contains(Flags::DIM) {
                        'd'
                    } else if cell.flags.contains(Flags::INVERSE) {
                        'i'
                    } else {
                        '.'
                    });
                }
            }
            let row = row.trim_end();
            out.push_str(row);
            out.push('\n');
            if want_style {
                // Truncate the style row to the text row rather than trimming
                // it on its own: the two must stay column-aligned, and a blank
                // cell's style is a space, which would trim to a different
                // length.
                let kept: String = style.chars().take(row.chars().count()).collect();
                styles.push_str(&kept);
                styles.push('\n');
            }
        }
        (out, styles)
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
        let cur = grid.cursor.point;
        let cx = cur.column.0 as u16;
        // `cur.line.0` is i32 in alacritty 0.26 (negative = scrollback); clamp to 0.
        let cy = cur.line.0.max(0);

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
        // The layout flags are excluded because a spacer *is* padding: it holds
        // a space only so the grid's column arithmetic works, and a client
        // derives it from the glyph beside it rather than being sent one.
        let is_padding = |li: i32, col: usize| {
            let cell = &grid[Line(li)][Column(col)];
            cell.c == ' '
                && cell.bg == Color::Named(NamedColor::Background)
                && cell.flags.difference(LAYOUT_FLAGS).is_empty()
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
        // The cursor's own row is painted even when it is blank, because the
        // client reaches the cursor by counting rows: a row the trim dropped is
        // a row the client never scrolls past, and the cursor lands that many
        // rows too high. Observed as two rows too high, mid-way along an old
        // output line, after typing a wrapped command at the shell and erasing
        // it again — zsh redraws the prompt on row 7 of 10 and clears to the
        // end of the screen, so the two rows under it are blank with scrollback
        // above. Rows *below* the cursor are still dropped; those are what
        // scrolled blank lines into the pane's scrollback.
        let end = end.max(cy + 1);
        // A row the child never ended: its text ran past the right edge and the
        // terminal carried it onto the next row. The grid marks that on the last
        // cell, and it is the difference between one logical line and two.
        let soft_wrapped = |li: i32| {
            self.cols > 0
                && grid[Line(li)][Column(self.cols as usize - 1)].flags.contains(Flags::WRAPLINE)
        };
        // What the paint emits for a row: the whole width for a soft-wrapped
        // row (filling the last column is what makes the client wrap on its
        // own), otherwise up to the last cell worth keeping.
        let row_end = |li: i32| if soft_wrapped(li) { self.cols as usize } else { content_end(li) };
        // Whether that leaves the client any character to draw — a row of
        // nothing but wide-char spacers emits none, and neither does a blank.
        let row_emits = |li: i32| {
            (0..row_end(li))
                .any(|col| !grid[Line(li)][Column(col)].flags.contains(Flags::WIDE_CHAR_SPACER))
        };
        let mut last = Style::reset();
        // A hyperlink is not an SGR and the `\x1b[0m` closing each row does not
        // end one, so it is tracked across the whole repaint rather than reset
        // per row — a link the child wrote across a wrap is one link.
        let mut link: Option<Hyperlink> = None;
        for li in start..end {
            // A soft-wrapped row is emitted to its full width and *not* followed
            // by a newline: filling the last column is what makes the client
            // wrap on its own, and a client that wrapped on its own remembers
            // the join. Hard-breaking it instead loses that, and the loss only
            // shows later — the pane stops reflowing those lines, so resizing
            // the window after a reattach leaves everything written before it
            // broken at the old width, and a copied line comes out with a
            // newline through the middle of it.
            let wrapped = soft_wrapped(li);
            for col in 0..row_end(li) {
                let cell = &grid[Line(li)][Column(col)];
                // The second half of a double-width glyph. The grid keeps it as
                // a cell of its own so its columns add up, but a client draws
                // both halves from the glyph itself — so sending the spacer's
                // space puts a *third* column on screen and shoves the rest of
                // the row one place right, compounding per glyph. A line of CJK
                // or emoji came back from a reattach visibly staggered.
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let style = Style::of(cell);
                if style != last {
                    data.extend_from_slice(sgr(style).as_bytes());
                    last = style;
                }
                // OSC 8: text the child marked up as a link itself. The pane
                // gives these the same ⌘-click as the ones it finds by scanning
                // (`ui/src/lib/termLinkProvider.ts`), and the marking lives on
                // the cell rather than in the text, so a snapshot that leaves it
                // out is a link that reads the same and no longer opens.
                let want = cell.hyperlink().filter(|h| hyperlink_open(h).is_some());
                if want != link {
                    let open = want.as_ref().and_then(hyperlink_open);
                    data.extend_from_slice(open.as_deref().unwrap_or(OSC8_CLOSE).as_bytes());
                    link = want;
                }
                let mut buf = [0u8; 4];
                data.extend_from_slice(cell.c.encode_utf8(&mut buf).as_bytes());
            }
            data.extend_from_slice(b"\x1b[0m");
            last = Style::reset();
            // CRLF *between* rows, never after the last one: a trailing newline
            // on a full screen scrolls the top row into scrollback and shifts
            // every row up by one — the child then redraws relative to a screen
            // that is off by a line, which is the "cursor stuck at the bottom"
            // desync.
            //
            // A soft-wrapped row is left one character short of the wrap it
            // relies on, and the next row's first character is what completes
            // it — so a next row with nothing to draw (the child erased the
            // continuation) would never wrap at all, and the paint would lose a
            // row. Break that one by hand instead: the join is not worth
            // keeping across an erased row, and every painted row consuming
            // exactly one client row is what the cursor arithmetic below rests
            // on.
            if li + 1 < end && (!wrapped || !row_emits(li + 1)) {
                data.extend_from_slice(b"\r\n");
            }
        }
        if link.is_some() {
            data.extend_from_slice(OSC8_CLOSE.as_bytes());
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

        // Put the cursor back, counted up from the end of the paint rather than
        // down from the top of the client's screen. The paint leaves it on the
        // last row it drew, so the row the child left it on is a fixed number
        // of rows above that one — the same number whatever height the client
        // is and however far the paint scrolled it. Everything else above is
        // height-independent already (rows scroll, modes are modes), so this
        // one move was the whole of the frame's dependence on the client's
        // *height*.
        //
        // Its width it still depends on, and that is not fixed here. `up`
        // counts grid rows, and a grid row is one client row only while the
        // client is `self.cols` wide: wider, and a soft-wrapped row does not
        // wrap there, so it and its continuation merge into one row; narrower,
        // and a row longer than the client splits into two. Rows above the
        // cursor are free, since the count starts below them, but every row
        // between the cursor and the end of the paint that disagrees moves the
        // cursor one row off. Observed both ways against a fresh emulator: a
        // 34-column status row under the input row brought the cursor back onto
        // the status row in a 30-column client, and a soft-wrapped row under it
        // brought the cursor back a row high in a client wider than the frame.
        // A client narrower than the emulator is the normal case, not the odd
        // one (see the note on `content_end`). The fix would be hard-breaking
        // every row, which costs the reflow the CRLF note above exists to keep,
        // so the frame stays pinned to `snap.cols` and the pane's fit is what
        // has to agree.
        //
        // AGE-222: it used to be a single absolute CUP, whose row was worked
        // out from `self.rows` — the height the daemon was last *told*, at
        // attach. That is only the pane's height until the pane fits again, and
        // a pane refits on its own: it watches its container with a
        // ResizeObserver, and `FitAddon.fit()` is a silent no-op until the
        // renderer has measured a cell, so an early fit can leave xterm's
        // placeholder geometry standing. A frame drawn for a screen 16 rows
        // shorter than the pane it landed in put the cursor 16 rows above the
        // child's last line, and cursor-agent — which erases the frame it drew
        // last relative to the cursor and draws it again the moment it is told
        // focus came back — then painted its input box and status bar over the
        // middle of the transcript, leaving the real ones untouched at the
        // bottom. Two input bars, and the typing going into the top one.
        let up = end - 1 - cy;
        // Column first: CHA also clears the deferred wrap a full-width last row
        // leaves the client holding, which is the state CUU would carry up.
        data.extend_from_slice(format!("\x1b[{}G", cx + 1).as_bytes());
        if up > 0 {
            data.extend_from_slice(format!("\x1b[{up}A").as_bytes());
        }
        // The same row, numbered from the top, for the frame header. Nothing
        // reads it today — the pane takes its cursor from the stream above —
        // and it is only true for a client of exactly `snap.rows` rows, which
        // is why the stream stopped saying it this way.
        let scrolled = ((end - start) as usize).saturating_sub(self.rows as usize) as i32;
        let bottom = (self.rows as i32 - 1).max(0);
        let cy = (cy - start - scrolled).clamp(0, bottom) as u16;
        // Hide the hardware cursor if the child had (full-screen TUIs draw their
        // own). A fresh client shows it by default, which is the phantom cursor
        // that appears where input isn't going.
        if !mode.contains(TermMode::SHOW_CURSOR) {
            data.extend_from_slice(b"\x1b[?25l");
        }

        Snapshot { cols: self.cols, rows: self.rows, cx, cy, data }
    }
}

/// Ends whatever OSC 8 hyperlink is open.
const OSC8_CLOSE: &str = "\x1b]8;;\x1b\\";

/// The OSC 8 that reopens `link`, or `None` when it cannot be replayed safely.
///
/// Default-deny, and stricter than either parser: a URI is emitted only when
/// both will read back exactly what the grid holds.
///
/// - A control character would end the OSC string early and hand the rest of
///   the URI to the parser as commands. Nothing else here is a string, so this
///   is the one place a byte of program output could escape its quoting.
/// - A `;` is read by the pane as the separator before the URI, and it keeps
///   only what precedes it, while alacritty rejoins the pieces. The same bytes
///   would mean two different links to the two sides.
///
/// OSC 8 asks for both percent-encoded, so neither belongs in a URI to begin
/// with, and a link dropped here degrades to ordinary text — which the pane's
/// own scanner picks up anyway whenever the text is the URL.
fn hyperlink_open(link: &Hyperlink) -> Option<String> {
    let uri = link.uri();
    if uri.is_empty() || !osc_safe(uri) {
        return None;
    }
    // The id is what lets a client treat the cells of one link as one thing
    // (highlighting all of it on hover, across a wrap). It is optional, so an
    // unusable one costs only that. `:` separates key=value pairs in the
    // parameter field, so it is excluded along with the unsafe bytes.
    let id = Some(link.id()).filter(|id| !id.is_empty() && osc_safe(id) && !id.contains(':'));
    match id {
        Some(id) => Some(format!("\x1b]8;id={id};{uri}\x1b\\")),
        None => Some(format!("\x1b]8;;{uri}\x1b\\")),
    }
}

/// Whether `s` can go inside an OSC string and come back out unchanged.
fn osc_safe(s: &str) -> bool {
    !s.is_empty() && !s.contains(';') && !s.chars().any(char::is_control)
}

/// Flags that record a cell's place in the grid rather than how it is drawn:
/// the two halves of a double-width glyph, the filler standing in for one that
/// did not fit on its row, and the soft-wrap marker. None of them is an SGR, so
/// the run-length encoder must not treat a change in them as a style change —
/// left in, they emitted a pointless reset between every wide glyph and the
/// spacer beside it.
const LAYOUT_FLAGS: Flags = Flags::WIDE_CHAR
    .union(Flags::WIDE_CHAR_SPACER)
    .union(Flags::LEADING_WIDE_CHAR_SPACER)
    .union(Flags::WRAPLINE);

/// Everything one SGR sets, which is everything that has to match before two
/// neighbouring cells can share one.
///
/// A cell carries its underline color outside `flags` (alacritty keeps it in
/// the cell's overflow allocation), so it has to be compared alongside them
/// rather than assumed to follow the text color.
#[derive(Clone, Copy, PartialEq)]
struct Style {
    flags: Flags,
    fg: Color,
    bg: Color,
    underline: Option<Color>,
}

impl Style {
    /// What a client is left in by the `\x1b[0m` that closes every row.
    fn reset() -> Style {
        Style {
            flags: Flags::empty(),
            fg: Color::Named(NamedColor::Foreground),
            bg: Color::Named(NamedColor::Background),
            underline: None,
        }
    }

    fn of(cell: &Cell) -> Style {
        Style {
            flags: cell.flags.difference(LAYOUT_FLAGS),
            fg: cell.fg,
            bg: cell.bg,
            underline: cell.underline_color(),
        }
    }
}

/// Build a minimal SGR sequence for the attributes a cell can carry.
///
/// Every visible one is reproduced. An attribute left out here is not a cell
/// drawn plainly, it is a cell drawn *wrong*: the snapshot is the whole of what
/// a reattaching pane gets, so whatever this does not say is whatever the pane
/// forgets the moment the user navigates away and back.
fn sgr(style: Style) -> String {
    let mut codes: Vec<String> = vec!["0".into()];
    if style.flags.contains(Flags::BOLD) {
        codes.push("1".into());
    }
    if style.flags.contains(Flags::DIM) {
        codes.push("2".into());
    }
    if style.flags.contains(Flags::ITALIC) {
        codes.push("3".into());
    }
    if let Some(code) = underline_sgr(style.flags) {
        codes.push(code.into());
    }
    if style.flags.contains(Flags::INVERSE) {
        codes.push("7".into());
    }
    // Concealed text (SGR 8). The grid stores the real character and leaves it
    // to the renderer to withhold it, so a snapshot that drops this doesn't
    // merely lose an attribute — it *reveals*, on return to the pane, whatever
    // the child had concealed.
    if style.flags.contains(Flags::HIDDEN) {
        codes.push("8".into());
    }
    if style.flags.contains(Flags::STRIKEOUT) {
        codes.push("9".into());
    }
    push_color(&mut codes, style.fg, true);
    push_color(&mut codes, style.bg, false);
    if let Some(c) = style.underline {
        push_underline_color(&mut codes, c);
    }
    format!("\x1b[{}m", codes.join(";"))
}

/// The SGR for whichever underline a cell carries, if any.
///
/// The five styles are mutually exclusive in the grid — alacritty clears the
/// others before setting one — so the first match is the only match.
///
/// Double underline goes out as `4:2` rather than the `21` xterm documents for
/// it, because alacritty reads a bare `21` as "cancel bold" (it follows the
/// older ECMA-48 reading). Emitting `21` would have the two parsers disagree
/// about the same snapshot: the pane would draw a double underline and the
/// daemon's own grid would quietly un-bold instead. That is the exact class of
/// split [`super::vt_pin`] exists to guard, and it costs nothing to avoid —
/// both read `4:<n>` the same way.
fn underline_sgr(flags: Flags) -> Option<&'static str> {
    if flags.contains(Flags::UNDERLINE) {
        Some("4")
    } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Some("4:2")
    } else if flags.contains(Flags::UNDERCURL) {
        Some("4:3")
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        Some("4:4")
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        Some("4:5")
    } else {
        None
    }
}

/// SGR 58, the color an underline is drawn in. A child sets it separately from
/// the text color — an undercurl under otherwise ordinary text is the usual
/// reason — so it survives or is lost on its own.
///
/// The protocol has no named form of it, so a palette color goes out as its
/// index. Nothing is pinned by that: a client resolves 0-15 through the same
/// theme either way.
fn push_underline_color(codes: &mut Vec<String>, c: Color) {
    codes.push("58".into());
    match c {
        Color::Spec(rgb) => {
            codes.push("2".into());
            codes.push(rgb.r.to_string());
            codes.push(rgb.g.to_string());
            codes.push(rgb.b.to_string());
        }
        Color::Indexed(i) => {
            codes.push("5".into());
            codes.push(i.to_string());
        }
        Color::Named(n) => match palette_index(n) {
            Some(i) => {
                codes.push("5".into());
                codes.push(i.to_string());
            }
            // Not a palette entry, so there is nothing to name: drop the `58`
            // again and let the underline take the text color, which is what
            // the default means.
            None => {
                codes.pop();
            }
        },
    }
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
    let index = palette_index(color)? as u16;
    let (base, offset) = match (fg, index >= 8) {
        (true, false) => (30, index),
        (true, true) => (90, index - 8),
        (false, false) => (40, index),
        (false, true) => (100, index - 8),
    };
    Some(base + offset)
}

/// The palette slot a named color occupies, 0-15. `None` for a color that is
/// not a palette entry at all.
fn palette_index(color: NamedColor) -> Option<u8> {
    use NamedColor as N;
    // The `Dim*` entries are the slots alacritty resolves a dimmed color to;
    // they are not separately addressable, so they map to the base color and
    // the `2` that `sgr` writes for `Flags::DIM` carries the rest.
    Some(match color {
        N::Black | N::DimBlack => 0,
        N::Red | N::DimRed => 1,
        N::Green | N::DimGreen => 2,
        N::Yellow | N::DimYellow => 3,
        N::Blue | N::DimBlue => 4,
        N::Magenta | N::DimMagenta => 5,
        N::Cyan | N::DimCyan => 6,
        N::White | N::DimWhite => 7,
        N::BrightBlack => 8,
        N::BrightRed => 9,
        N::BrightGreen => 10,
        N::BrightYellow => 11,
        N::BrightBlue => 12,
        N::BrightMagenta => 13,
        N::BrightCyan => 14,
        N::BrightWhite => 15,
        // The defaults, plus the two slots a cell never holds. Exhaustive on
        // purpose: `NamedColor` is not `#[non_exhaustive]`, so a version bump
        // that adds a color fails this build rather than silently losing it
        // the way the wildcard did.
        N::Foreground | N::Background | N::Cursor | N::BrightForeground | N::DimForeground => {
            return None;
        }
    })
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

    /// The style map has to line up with the text column for column, including
    /// through a leading indent, because its only consumer indexes it by a
    /// column it found in the text (`sendq::prompt_looks_empty`).
    #[test]
    fn capture_styled_marks_faint_and_inverse_cells_in_step_with_the_text() {
        let mut e = Emulator::new(40, 3);
        e.feed(b"  \x1b[2mdim\x1b[0m \x1b[7mI\x1b[0m plain");
        let (text, style) = e.capture_styled(3);
        let (text, style) = (text.lines().next().unwrap(), style.lines().next().unwrap());
        assert_eq!(text, "  dim I plain");
        assert_eq!(style, "  ddd i .....");
        assert_eq!(text.chars().count(), style.chars().count());
    }

    #[test]
    fn a_bare_line_feed_holds_the_column() {
        // AGE-204: the shape crush draws with. It erases a run of cells, steps
        // down with a bare LF expecting to keep its column, and erases the same
        // run again — one row of a dialog's interior per step. Returning the
        // carriage (the pane's old `convertEol`, mirrored here) put every step
        // after the first at column 0, which is what tore the dialog apart.
        let mut e = Emulator::new(20, 4);
        e.feed(b"\x1b[1;6Habcd\x1b[6X\nefgh\x1b[6X\nij");
        let cap = e.capture(4);
        let rows: Vec<&str> = cap.lines().take(3).collect();
        assert_eq!(rows, vec!["     abcd", "         efgh", "             ij"]);
    }

    #[test]
    fn a_bare_line_feed_reads_the_same_across_reads_and_resets() {
        // Nothing about this may depend on where the PTY reads happen to split,
        // and `reset` (RIS) must not change it either.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1bc\x1b[1;3Hon");
        e.feed(b"e\n");
        e.feed(b"two");
        let cap = e.capture(6);
        let rows: Vec<&str> = cap.lines().take(2).collect();
        assert_eq!(rows, vec!["  one", "     two"]);
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
    fn snapshot_replays_the_staircase_a_bare_line_feed_really_draws() {
        // AGE-67 in its corrected form. The pane and the daemon read a bare LF
        // the same way now, so the question is no longer whether the snapshot
        // paints a staircase — it must, because that is what the child drew —
        // but whether it paints the *same* one, at the same columns.
        let line = "Recycle mTLS Workloads 0:36";
        let mut e = Emulator::new(60, 6);
        e.feed(format!("{line}\n{line}").as_bytes());
        // The second line starts where the first one ended, because that is
        // where the child left the cursor.
        let cap = e.capture(6);
        assert_eq!(cap.lines().nth(1), Some(format!("{}{line}", " ".repeat(line.len())).as_str()));

        let snap = e.snapshot();
        let mut back = Emulator::new(snap.cols, snap.rows);
        back.feed(&snap.data);
        assert_eq!(back.capture(10).trim_end(), cap.trim_end());
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

    /// The cursor's column, and the text of the row it is sitting on.
    ///
    /// What "the cursor is in the right place" means to someone looking at the
    /// pane: not the row *number* (the replay's screen is scrolled differently
    /// from the emulator's grid, deliberately — see the trims in `snapshot`)
    /// but the line it is on and where along it.
    fn cursor_view(e: &Emulator) -> (usize, String) {
        let li = e.term.grid().cursor.point.line.0;
        let mut row = String::new();
        for col in 0..e.cols as usize {
            row.push(e.term.grid()[Line(li)][Column(col)].c);
        }
        (e.term.grid().cursor.point.column.0, row.trim_end().to_string())
    }

    /// Replay the snapshot into a fresh emulator — what the pane does on
    /// reattach — and assert the cursor came back where the child left it.
    fn assert_cursor_survives_a_reattach(a: &Emulator, what: &str) {
        let snap = a.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert_eq!(cursor_view(&b), cursor_view(a), "cursor moved on reattach: {what}");
        // And the frame header's own cursor fields describe the same place. No
        // client reads them — the pane takes the cursor from the stream — but a
        // header that says something the frame does not is a trap for the one
        // that starts to.
        assert_eq!(
            (snap.cx as usize, snap.cy as i32),
            (b.term.grid().cursor.point.column.0, b.term.grid().cursor.point.line.0),
            "snapshot cx/cy disagree with the stream it carries: {what}",
        );
    }

    #[test]
    fn snapshot_keeps_the_cursor_on_its_line_in_a_client_taller_than_the_frame() {
        // AGE-222. A frame is drawn for the height the daemon was told at
        // attach, and lands in whatever height the pane has by then: a pane
        // watches its container and refits on its own, and `FitAddon.fit()` is
        // a silent no-op until xterm has measured a cell, so the two part by
        // however many rows the pane gained in between. Counted from the top of
        // the screen the cursor then came back that many rows high, and
        // cursor-agent redrew its input box and status bar there — over the
        // middle of the transcript, with the real ones still at the bottom.
        let mut e = Emulator::new(40, 10);
        for i in 0..30 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        // A TUI's footer, with the cursor parked at the top of it: that is the
        // row the next frame erases from, and the row this has to restore.
        e.feed(b"> type here\r\nstatus");
        e.feed(b"\x1b[1A\r");
        let snap = e.snapshot();

        let mut tall = Emulator::new(snap.cols, snap.rows + 16);
        tall.feed(&snap.data);
        assert_eq!(
            cursor_view(&tall),
            cursor_view(&e),
            "the cursor came back off its line in a client taller than the frame",
        );
    }

    #[test]
    fn snapshot_keeps_the_cursor_on_its_own_line_when_rows_below_it_are_blank() {
        // The toggle-the-terminal-and-type bug. A shell with scrollback whose
        // last rows are blank — zsh redraws the prompt higher up and clears to
        // the end of the screen every time a wrapped command line is erased —
        // came back with the cursor two rows above the prompt, part-way along
        // an old output line, because the snapshot numbered the cursor's row in
        // the emulator's grid while the trimmed repaint had scrolled the client
        // further down.
        let mut e = Emulator::new(40, 10);
        for i in 0..20 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        // A command line long enough to have wrapped, erased: the prompt is
        // redrawn two rows higher and everything under it cleared.
        e.feed(b"$ a-long-command");
        e.feed(b"\x1b[2A\r$ \x1b[J");
        assert_cursor_survives_a_reattach(&e, "prompt with blank rows under it");
    }

    #[test]
    fn snapshot_keeps_the_cursor_on_a_blank_row_below_the_last_output() {
        // Same arithmetic, one row further: the child left the cursor on a row
        // of its own below everything it drew (a command still running, its
        // output ending in a newline). The row is blank, so the trim drops it —
        // and the cursor then lands on the last line of output instead, with
        // whatever the user types appearing in the middle of it.
        let mut e = Emulator::new(40, 10);
        for i in 0..20 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        assert_cursor_survives_a_reattach(&e, "cursor alone on the row below the output");
    }

    #[test]
    fn snapshot_keeps_the_cursor_after_a_screen_clear_that_kept_the_scrollback() {
        // ED 2 without ED 3: the screen is blank and the prompt is back at the
        // top of it, but the scrollback the snapshot replays is not, so the
        // client's screen ends up scrolled a long way from the emulator's.
        let mut e = Emulator::new(40, 10);
        for i in 0..20 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        e.feed(b"\x1b[2J\x1b[H$ ");
        assert_cursor_survives_a_reattach(&e, "cleared screen over kept scrollback");
    }

    #[test]
    fn snapshot_keeps_the_cursor_when_a_wrapped_line_lost_its_continuation() {
        // A soft-wrapped row relies on the next row's first character to make
        // the client wrap. Where the child erased that continuation there is no
        // such character, so the row has to be broken by hand or the paint
        // loses a row and everything below it — the cursor included — comes
        // back one row high.
        let mut e = Emulator::new(10, 6);
        e.feed(b"0123456789continued\r\n");
        // Erase the continuation row, then park the cursor below it.
        e.feed(b"\x1b[2;1H\x1b[2K\x1b[4;3Hx\x1b[4;4H");
        assert_cursor_survives_a_reattach(&e, "wrapped row with an erased continuation");
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
    fn snapshot_keeps_text_the_child_concealed_concealed() {
        // SGR 8 is stored as a flag over the real character, so dropping it
        // does not blank the cell — it un-conceals it. Whatever a child hid
        // came back visible the first time the user navigated away and
        // returned, which is the one attribute here whose loss shows something
        // rather than hiding it.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[8msecret\x1b[0m");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert!(
            b.term.grid()[Line(0)][Column(0)].flags.contains(Flags::HIDDEN),
            "concealed text was revealed by the reattach snapshot",
        );
    }

    #[test]
    fn snapshot_preserves_strikeout_and_every_underline_style() {
        // The five underline styles are mutually exclusive in the grid, so
        // each is checked on its own cell rather than in one run.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[9ms\x1b[0m\x1b[4ma\x1b[4:2mb\x1b[4:3mc\x1b[4:4md\x1b[4:5me\x1b[0m");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        let flags = |col: usize| b.term.grid()[Line(0)][Column(col)].flags;
        assert!(flags(0).contains(Flags::STRIKEOUT), "strikeout lost");
        assert!(flags(1).contains(Flags::UNDERLINE), "single underline lost");
        assert!(flags(2).contains(Flags::DOUBLE_UNDERLINE), "double underline lost");
        assert!(flags(3).contains(Flags::UNDERCURL), "undercurl lost");
        assert!(flags(4).contains(Flags::DOTTED_UNDERLINE), "dotted underline lost");
        assert!(flags(5).contains(Flags::DASHED_UNDERLINE), "dashed underline lost");
    }

    #[test]
    fn snapshot_does_not_spell_a_double_underline_as_21() {
        // `21` is xterm's code for it, but alacritty reads a bare `21` as
        // "cancel bold" — so a snapshot using it would have the pane and the
        // daemon's own grid disagree about what they just exchanged. `4:2` is
        // read the same by both; the round-trip above is what proves it.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[4:2mtext");
        let text = String::from_utf8_lossy(&e.snapshot().data).to_string();
        assert!(text.contains("4:2"), "double underline not emitted as a sub-parameter: {text:?}");
        assert!(!text.contains(";21"), "double underline emitted as 21: {text:?}");
    }

    #[test]
    fn snapshot_preserves_the_underline_color() {
        // Set apart from the text color (SGR 58), and stored outside `flags`,
        // so it is dropped by anything that reproduces the flags alone. An
        // undercurl is normally the only thing colored this way.
        let mut e = Emulator::new(40, 6);
        e.feed(b"\x1b[4:3m\x1b[58;5;196mtypo\x1b[0m\x1b[4:3mplain");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        let cell = |col: usize| b.term.grid()[Line(0)][Column(col)].underline_color();
        assert_eq!(cell(0), Some(Color::Indexed(196)), "underline color lost");
        assert_eq!(cell(4), None, "underline color leaked past its reset");
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
    fn snapshot_does_not_stagger_double_width_glyphs() {
        // A wide glyph owns two cells: the glyph, then a spacer holding a
        // space so the column arithmetic works. A client draws both halves
        // from the glyph, so emitting the spacer put a third column on screen
        // and pushed the rest of the row one place right — compounding per
        // glyph, so a line of CJK or emoji came back from a reattach
        // staggered, one column further off with every character.
        let mut e = Emulator::new(20, 3);
        e.feed("日本語 tail".as_bytes());
        let snap = e.snapshot();
        assert!(
            String::from_utf8_lossy(&snap.data).contains("日本語"),
            "spacers (or a reset between them) broke up the glyphs: {:?}",
            String::from_utf8_lossy(&snap.data),
        );
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert_eq!(b.capture(3), e.capture(3), "row drifted");
        assert_eq!(b.term.grid()[Line(0)][Column(2)].c, '本');
        assert_eq!(b.term.grid()[Line(0)][Column(7)].c, 't', "tail shifted right");
    }

    #[test]
    fn snapshot_keeps_a_soft_wrapped_line_joined() {
        // Text that ran past the right edge is one logical line. Ending the
        // row with a CRLF instead makes it two, and the difference only shows
        // afterwards: the pane stops reflowing those lines, so resizing the
        // window after a reattach leaves everything written before it broken
        // at the old width. Widening is what makes the loss visible, so that
        // is what this checks.
        let mut e = Emulator::new(10, 4);
        e.feed(b"abcdefghijklmnopqrs");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert!(
            b.term.grid()[Line(0)][Column(9)].flags.contains(Flags::WRAPLINE),
            "the wrap was replayed as a hard line break",
        );
        e.resize(20, 4);
        b.resize(20, 4);
        assert_eq!(b.capture(4), e.capture(4));
        assert!(b.capture(4).starts_with("abcdefghijklmnopqrs"), "line did not rejoin");
    }

    #[test]
    fn snapshot_keeps_a_hard_break_hard() {
        // The other half of the same rule, and the reason it keys off the
        // grid's own flag rather than "is the row full": a row filled exactly
        // to the edge and then ended by the child is two lines, and must not
        // be rejoined by a widening.
        let mut e = Emulator::new(10, 4);
        e.feed(b"abcdefghij\r\nklm");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        e.resize(20, 4);
        b.resize(20, 4);
        assert_eq!(b.capture(4), e.capture(4));
        assert!(!b.capture(4).contains("abcdefghijklm"), "a hard break was joined");
    }

    #[test]
    fn snapshot_preserves_osc_8_hyperlinks() {
        // Text the child marked up as a link itself. The pane gives these the
        // same ⌘-click as the ones it finds by scanning, and the marking is on
        // the cell rather than in the text — so dropping it left a link that
        // reads exactly the same and no longer opens.
        let mut e = Emulator::new(40, 3);
        e.feed(b"see \x1b]8;;https://example.com\x07link\x1b]8;;\x07 end");
        let snap = e.snapshot();
        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        let uri = |col: usize| {
            b.term.grid()[Line(0)][Column(col)].hyperlink().map(|l| l.uri().to_string())
        };
        assert_eq!(uri(4).as_deref(), Some("https://example.com"), "hyperlink lost");
        assert_eq!(uri(0), None, "link leaked onto the text before it");
        assert_eq!(uri(8), None, "link was never closed");
    }

    #[test]
    fn a_hyperlink_that_cannot_be_quoted_safely_is_dropped() {
        // Default-deny on the one place program output travels inside a quoted
        // string. A control character would close the string early and hand
        // the rest to the parser as commands; a `;` means two different links
        // to the two parsers, since the client keeps only what precedes it.
        assert!(hyperlink_open(&Hyperlink::new(Some("id"), "https://ok/x".into())).is_some());
        for bad in ["https://x/\x07\x1b[6n", "https://x/a;b", "https://x/\u{9b}c", ""] {
            assert!(
                hyperlink_open(&Hyperlink::new(Some("id"), bad.into())).is_none(),
                "emitted an unsafe URI: {bad:?}",
            );
        }
        // An unusable id costs only the id — the link itself still replays.
        let odd = Hyperlink::new(Some("a;b"), "https://example.com".into());
        let open = hyperlink_open(&odd).expect("link dropped for its id");
        assert!(!open.contains("a;b") && open.contains("https://example.com"), "{open:?}");
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
    /// Only the three shapes [`Emulator::snapshot`] emits are recognised: a CSI
    /// (`ESC [`, parameters, final byte), an OSC (`ESC ]`, a string, `ESC \`)
    /// and a two-byte `ESC x`. Anything else — a DCS, an OSC that never
    /// terminates, an unterminated CSI — comes back as one blob that fails the
    /// allowlist below, which is the point of scanning this way.
    ///
    /// The OSC arm matters most: its string swallows the bytes inside it, so
    /// scanning without it would let a sequence smuggled into a URI past the
    /// check as ordinary text. Only `ESC \` closes one here — a BEL-terminated
    /// OSC runs to the end of the input and fails, which is deliberate, since
    /// `snapshot` is not allowed to emit one.
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
            match data.get(i) {
                Some(&b'[') => {
                    i += 1;
                    while i < data.len() && !(0x40..=0x7e).contains(&data[i]) {
                        i += 1;
                    }
                    i = (i + 1).min(data.len());
                }
                Some(&b']') => {
                    i += 1;
                    while i < data.len() && !data[i..].starts_with(b"\x1b\\") {
                        i += 1;
                    }
                    i = (i + 2).min(data.len());
                }
                _ => i = (i + 1).min(data.len()),
            }
            out.push(data[start..i].to_vec());
        }
        out
    }

    /// Default-deny: the exact sequences `snapshot` declares it emits, plus the
    /// three parameterised shapes (SGR, and the column-then-up pair that puts
    /// the cursor back). Nothing here provokes a reply from a client except
    /// `?1004h`; see the note on `snapshot`.
    fn is_declared(seq: &[u8]) -> bool {
        const EXACT: [&[u8]; 16] = [
            b"\x1b[?1049h",
            b"\x1b[2J",
            b"\x1b[3J",
            b"\x1b[H",
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
        // An OSC 8 hyperlink, the one string sequence `snapshot` emits. It
        // provokes no reply, but it is the only place program output travels
        // inside a quoted string, so the interior is held to what
        // `hyperlink_open` promises: ST-terminated, and no control byte that
        // could close the string early and let the tail run as commands.
        if let Some(body) = seq.strip_prefix(b"\x1b]8;") {
            let Some(body) = body.strip_suffix(b"\x1b\\") else { return false };
            return !body.iter().any(u8::is_ascii_control);
        }
        let Some(body) = seq.strip_prefix(b"\x1b[") else { return false };
        let Some((&last, params)) = body.split_last() else { return false };
        match last {
            // SGR. `:` as well as `;`, for the sub-parameter of an underline
            // style (`4:2`); see `underline_sgr`.
            b'm' => params.iter().all(|b| b.is_ascii_digit() || matches!(b, b';' | b':')),
            // The cursor's column (CHA) and the rows it goes up (CUU). One
            // parameter each, no sub-parameters, and an absolute CUP is no
            // longer among them — see the note above the pair in `snapshot`.
            b'G' | b'A' => !params.is_empty() && params.iter().all(u8::is_ascii_digit),
            _ => false,
        }
    }

    #[test]
    fn a_snapshot_emits_only_the_escape_sequences_it_declares() {
        let mut e = Emulator::new(30, 4);
        e.feed(
            b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?2004h\x1b[?7l\x1b[?1h\x1b=\
              \x1b[?25l\x1b[1;38;5;196mbar\x1b[0m\r\n\
              \x1b[8;9;4:3;58;2;255;0;0;31;44mrow\x1b[0m\
              \x1b]8;;https://example.com\x07link\x1b]8;;\x07",
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
    fn the_escape_scanner_is_not_fooled_by_a_string_sequence() {
        // A guard on the guard. Now that `snapshot` emits an OSC, the scanner
        // has to swallow the string whole: one that walked past `ESC ]` and
        // kept scanning would read the bytes *inside* a URI as ordinary text
        // and wave a smuggled query through the allowlist below.
        let smuggled = b"pre\x1b]8;;http://x\x07\x1b[6n\x1b\\post";
        assert!(
            escapes(smuggled).iter().any(|s| !is_declared(s)),
            "a query hidden inside an OSC string passed the check",
        );
        // A BEL-terminated OSC is not a shape `snapshot` may emit, so it must
        // fail rather than being scanned as if it had ended.
        assert!(escapes(b"\x1b]8;;http://x\x07").iter().any(|s| !is_declared(s)));
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
