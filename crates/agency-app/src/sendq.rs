//! The per-session send queue: text Agency types into a live agent session on
//! the user's behalf, held until the session can actually take it.
//!
//! Three features hand an agent a prompt the human never typed — review
//! comments, failing CI checks, a conflicted merge. All three used to write
//! straight into the pty the moment the button was clicked, checking only that
//! the session was `Running`. So the text landed in the middle of the agent's
//! turn, or on top of a half-typed prompt the human had not sent yet.
//!
//! This module is the pure half of the fix, in the `looper.rs` / `notifier.rs`
//! mold: [`decide`] is a transition function over one observation of a session,
//! and the pty write is the caller's only side effect. `AppState` owns the
//! queues and drains them on the notifier tick, which is also where the
//! busy/idle observation ([`crate::activity`]) comes from.
//!
//! Two asymmetries run through the whole design, and both are deliberate:
//!
//! - **The draft block is one-directional.** Keystroke tracking sets it;
//!   reading the pane may only *clear* it, never set it. Holding a message a
//!   few seconds too long is invisible, typing over a half-typed prompt is not.
//! - **Nothing here ever clears the line.** No Escape (the human may have
//!   opened that menu on purpose), no `^U`. Even the timeout appends after
//!   whatever is sitting there, so a draft is submitted along with our text
//!   rather than thrown away.

use std::collections::VecDeque;

/// Quiet gap after the human's last keystroke before the queue will type
/// anything. The pane hash the notifier diffs only moves once the agent has
/// echoed what was typed, so a decision taken inside this window is a decision
/// taken against a pane that has not caught up yet.
pub const ECHO_GRACE_MS: i64 = 1_000;

/// How long a message may wait before it goes out regardless of what the
/// session looks like. Long enough to sit out any ordinary turn; short enough
/// that a message the user is waiting on isn't lost for an afternoon behind an
/// agent that never stops producing output, or behind a draft the human
/// abandoned mid-word and walked away from.
pub const MAX_HOLD_MS: i64 = 5 * 60 * 1_000;

/// Messages one session may have waiting. A queue this deep means nothing is
/// draining, and silently growing it would turn a stuck agent into a burst of
/// stale prompts the moment it frees up.
pub const MAX_PENDING: usize = 8;

/// How far up from the bottom of the pane [`prompt_looks_empty`] looks for the
/// live prompt line, counted in non-blank rows.
///
/// Deep enough to clear the status rows agents draw *under* their prompt box:
/// observed 2026-09-08, cursor-agent draws two of them (`Auto · 21.2%   Run
/// Everything`, then the cwd and branch), which puts its prompt line third from
/// the bottom.
///
/// It was 8, which left five rows of the agent's own *output* inside the
/// window. That was survivable while a marker row could only answer through the
/// exact [`PROMPT_PLACEHOLDERS`] prose; with the all-faint rule in
/// [`agent_painted`] an ordinary dim trace line beginning `→` now answers
/// "empty" instead, and the scan returns on the first marker row it reaches.
/// The exposed agents are the ones that put no marker on the row their draft
/// lands on (opencode, crush, pi): the scan walks past the draft and answers
/// from a line further up, and the queued text is typed over a prompt the human
/// is half-way through. Keeping the window to the agent's own chrome is what
/// stops it.
const PROMPT_SCAN_LINES: usize = 4;

/// Prompt markers agents draw at the start of their input line.
///
/// → is cursor-agent's. Without it the scan walked past cursor's prompt line
/// entirely and answered "cannot tell" for every pane it ever drew, so a draft
/// in a cursor tab could not be lifted by a pane read at all (AGE-199).
const PROMPT_MARKERS: [char; 4] = ['>', '❯', '›', '→'];

/// Placeholder prose an agent writes *inside* its own empty input line, which
/// is otherwise indistinguishable from a half-typed draft: the marker is there
/// and there is text after it.
///
/// cursor-agent draws `→ Plan, search, build anything` on a fresh session and
/// `→ Add a follow-up` after a turn (both observed; the second is the pane in
/// AGE-199's screenshot). A merge conflict handed to that tab was held the full
/// [`MAX_HOLD_MS`] and then appended, under a marker saying the agent was
/// mid-turn, while it sat at an empty prompt with nothing to finish.
///
/// The *fallback*. [`prompt_looks_empty`] prefers the cell styles, which say
/// the same thing without knowing any prose; this catches the case where the
/// daemon is too old to send them, or an agent paints its hint in plain cells.
///
/// An exact-match allowlist, not a heuristic: this is the one function allowed
/// to lift a draft block, and the cost of a wrong "empty" is typing over a
/// prompt the human is half-way through. A placeholder that changes with the
/// CLI simply stops matching, which puts that tab back to holding, which is the
/// safe direction.
///
/// Each is paired with the marker it was seen on, so it counts as empty only in
/// the agent that draws it. Matched against every marker, a human in a `>` tab
/// whose whole draft is "Add a follow-up" reads as an empty prompt, and the
/// queued text is typed onto their line and submitted with it.
const PROMPT_PLACEHOLDERS: [(char, &str); 2] =
    [('→', "Add a follow-up"), ('→', "Plan, search, build anything")];

/// One message waiting for a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    /// What to type. Single line: the carriage return is the writer's, and it
    /// goes out separately (see `TermClient::send_text`).
    pub text: String,
    /// Which feature composed it, for the log line when it lands or is dropped
    /// and for the marker that says what a run is still owed. One of
    /// [`ORIGINS`].
    pub origin: &'static str,
    pub queued_at_ms: i64,
}

/// Every feature allowed to put text in a session's mouth, as an allowlist.
///
/// Restoring a queue from the database maps the stored string back through
/// this, so a row written by a build that queued something we no longer
/// understand is dropped rather than typed into a live agent. Default-deny: a
/// list of origins to *reject* would let exactly the unknown case through.
pub const ORIGINS: [&str; 3] = ["check feedback", "merge conflict", "review comments"];

/// The `'static` origin matching a stored string, or None if it is not one of
/// ours.
pub fn known_origin(s: &str) -> Option<&'static str> {
    ORIGINS.into_iter().find(|o| *o == s)
}

/// What one session looks like at the moment a drain is considered.
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    pub now_ms: i64,
    pub session_running: bool,
    /// `activity::classify` says the pane is `Working` — the agent is mid-turn.
    pub working: bool,
    /// Epoch ms of the human's last keystroke in this session, if any.
    pub last_key_ms: Option<i64>,
    /// The human has typed something they have not submitted. Set by keystroke
    /// tracking, cleared by their Enter or by a pane read that can see the
    /// prompt is empty — never set by a pane read.
    pub draft: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    /// Inside the echo grace: the human just pressed a key.
    EchoGrace,
    /// The agent is mid-turn.
    Working,
    /// Something unsent is on the prompt line.
    Draft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardReason {
    /// The agent exited. Nothing typed now could reach it, and a message
    /// replayed into whatever session takes its place would arrive with no
    /// context at all.
    SessionGone,
}

/// Why a message is going out. The write is the same either way; the event is
/// not, which is why the two are told apart at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendReason {
    /// The session was ready for it: between turns, with an empty prompt line.
    Clear,
    /// [`MAX_HOLD_MS`] ran out with the agent still working or a draft still on
    /// the line, so the text goes in after whatever is sitting there and is
    /// submitted along with it.
    Appended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Type the text, then the carriage return.
    Send(SendReason),
    Hold(HoldReason),
    Discard(DiscardReason),
}

/// Whether the head of a session's queue may go out right now.
///
/// Order matters. A gone session discards before anything else looks at the
/// clock. The echo grace outranks the timeout because it is a single second and
/// the pane it protects is the evidence every later rule reads. The timeout
/// then outranks both the busy and the draft holds — that is what "on timeout,
/// append" means, and whether either of them was actually in the way is what
/// separates [`SendReason::Appended`] from an ordinary send that happens to be
/// late.
pub fn decide(head: &Queued, obs: &Observation) -> Decision {
    if !obs.session_running {
        return Decision::Discard(DiscardReason::SessionGone);
    }
    if let Some(key_ms) = obs.last_key_ms {
        if obs.now_ms.saturating_sub(key_ms) < ECHO_GRACE_MS {
            return Decision::Hold(HoldReason::EchoGrace);
        }
    }
    if obs.now_ms.saturating_sub(head.queued_at_ms) >= MAX_HOLD_MS {
        let over = obs.working || obs.draft;
        return Decision::Send(if over { SendReason::Appended } else { SendReason::Clear });
    }
    if obs.working {
        return Decision::Hold(HoldReason::Working);
    }
    if obs.draft {
        return Decision::Hold(HoldReason::Draft);
    }
    Decision::Send(SendReason::Clear)
}

/// A queue event the user has to be told about after the fact.
///
/// The three senders each say in place whether the text went in or is waiting,
/// and the run's marker says so for as long as it is held. Both of those are
/// about a message that is still on its way. These two are the moment it stops
/// being held, which is exactly when the surface that sent it is closed: a
/// queue thrown away for a session that has since exited, and a message that
/// waited out [`MAX_HOLD_MS`] and went in on top of something.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// The run whose queue it was, for the UI that shows it.
    pub run_id: String,
    pub kind: NoticeKind,
    /// The whole sentence, composed here so it can be tested without a running
    /// app.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeKind {
    /// Nothing was typed and nothing will be: a loss, not a status change.
    Dropped,
    /// It went in, but not cleanly.
    Appended,
}

/// How a message is named in a sentence of its own.
///
/// The popover can show the bare origin next to the whole text; a toast that
/// arrives minutes later, with the sending surface long closed, cannot. Falls
/// back to something true for anything outside [`ORIGINS`] rather than naming
/// a string we do not recognise.
fn origin_phrase(origin: &str) -> &'static str {
    match origin {
        "review comments" => "the review comments",
        "check feedback" => "the CI feedback",
        "merge conflict" => "the merge conflict prompt",
        _ => "the message",
    }
}

/// The hold window as the copy below spells it, kept honest by
/// `the_copy_matches_the_hold_window`.
const MAX_HOLD_WORDS: &str = "five minutes";

/// "Agency dropped the review comments it was holding for Fix login: that
/// session is gone."
///
/// `n` is the whole queue, head included: a discard takes all of it, and
/// reporting only the head would understate the loss.
pub fn dropped_notice(run_id: &str, label: &str, head_origin: &str, n: usize) -> Notice {
    let what =
        if n > 1 { format!("the {n} messages") } else { origin_phrase(head_origin).to_string() };
    Notice {
        run_id: run_id.to_string(),
        kind: NoticeKind::Dropped,
        text: format!("Agency dropped {what} it was holding for {label}: that session is gone."),
    }
}

/// "Agency waited five minutes for Fix login to come free, then added the
/// review comments to what was already on its prompt line."
///
/// Phrased around the wait rather than around the message, so one sentence
/// covers both things the timeout overrides: an agent that never stopped
/// working, and a draft the human walked away from.
pub fn appended_notice(run_id: &str, label: &str, origin: &str) -> Notice {
    Notice {
        run_id: run_id.to_string(),
        kind: NoticeKind::Appended,
        text: format!(
            "Agency waited {MAX_HOLD_WORDS} for {label} to come free, then added {} to what \
             was already on its prompt line.",
            origin_phrase(origin)
        ),
    }
}

/// What a chunk of input from the pane means for the draft on the prompt line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typed {
    /// The human pressed Enter: whatever was on the line is gone to the agent.
    Submitted,
    /// The human changed the line.
    Edited,
    /// Nothing a human typed — a mouse report, a focus event, an arrow key.
    Nothing,
}

/// Classify one write from the pane.
///
/// The pane's input handler also receives xterm mouse-tracking and focus escape
/// sequences: hovering or scrolling an agent's pane emits a steady stream of
/// them, and counting those as typing would block every queued message behind a
/// draft that does not exist.
pub fn classify_input(data: &[u8]) -> Typed {
    if data.contains(&b'\r') || data.contains(&b'\n') {
        return Typed::Submitted;
    }
    let mut i = 0;
    let mut edited = false;
    while i < data.len() {
        if data[i] == 0x1b {
            i = skip_escape(data, i);
            continue;
        }
        // Printable, space, or a rubout: the human is editing the line. Other
        // control bytes (Ctrl-C, Ctrl-U, Tab) are deliberately not read as
        // *clearing* it — the block is one-directional, and only a pane read
        // may lift it.
        if data[i] >= 0x20 || data[i] == 0x7f || data[i] == 0x08 {
            edited = true;
        }
        i += 1;
    }
    if edited {
        Typed::Edited
    } else {
        Typed::Nothing
    }
}

/// Index just past the escape sequence starting at `start`.
///
/// CSI and SS3 sequences end at the first byte in `0x40..=0x7e`. X10 mouse
/// reports (`ESC [ M` then three raw coordinate bytes) are the exception: their
/// payload is ordinary printable bytes, so without consuming it every mouse
/// move over the pane would read as typing.
fn skip_escape(data: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    if i < data.len() && (data[i] == b'[' || data[i] == b'O') {
        let introducer = data[i];
        i += 1;
        if introducer == b'[' && i < data.len() && data[i] == b'M' {
            return (i + 4).min(data.len());
        }
    }
    while i < data.len() && !(0x40..=0x7e).contains(&data[i]) {
        i += 1;
    }
    i + 1
}

/// Whether the pane's live prompt line is visibly empty.
///
/// True only when a prompt marker is found and what follows it is not the
/// human's: nothing at all, prose the agent painted (see [`agent_painted`]), or
/// a known [`PROMPT_PLACEHOLDERS`] string. Everything else — text after the
/// marker, no marker in range, a pane we cannot parse — is "cannot tell", which
/// holds. That asymmetry is the point: this may only clear a draft block, never
/// create one, so a wrong answer costs a few seconds of delay rather than a
/// clobbered prompt.
///
/// `style` is the cell-style map from `Emulator::capture_styled`, or None from
/// a daemon that predates it.
pub fn prompt_looks_empty(pane: &str, style: Option<&str>) -> bool {
    let styles: Vec<&str> = style.map(|s| s.lines().collect()).unwrap_or_default();
    let rows: Vec<&str> = pane.lines().collect();
    let mut scanned = 0;
    for (i, raw) in rows.iter().enumerate().rev() {
        let line = strip_frame(raw);
        if line.is_empty() {
            continue;
        }
        scanned += 1;
        if scanned > PROMPT_SCAN_LINES {
            break;
        }
        let mut chars = line.chars();
        let Some(first) = chars.next() else { continue };
        if !PROMPT_MARKERS.contains(&first) {
            continue;
        }
        let rest = chars.as_str().trim();
        if rest.is_empty() {
            return true;
        }
        let style_row = styles.get(i).copied().unwrap_or("");
        let painted = match (column_of(raw, line), column_of(raw, rest)) {
            (Some(from), Some(at)) => agent_painted(style_row, from, at + rest.chars().count()),
            _ => false,
        };
        return painted || PROMPT_PLACEHOLDERS.contains(&(first, rest));
    }
    false
}

/// Whether columns `from..to` of one row were drawn by the agent rather than
/// typed by the human, going on how they were drawn rather than what they say.
///
/// An agent draws the hint sitting in its empty input line faint and the
/// human's keystrokes plainly, so "all faint" is the same judgement
/// [`PROMPT_PLACEHOLDERS`] makes without having to know the prose.
/// Observed 2026-09-08 against a real cursor-agent pty: the placeholder row
/// `→ Plan, search, build anything` comes back with SGR 2 on the arrow and on
/// every letter, and the same row holding the draft `→ half a thou` comes back
/// with no attributes at all.
///
/// The one exception is a single inverse cell. cursor-agent hides the hardware
/// cursor and paints its own block over the hint's first character, so `P` in
/// that row arrives INVERSE while its neighbours are DIM; requiring faintness
/// with no exceptions would call that row plain and hold the message the full
/// [`MAX_HOLD_MS`], which is the bug this is here to fix. Exactly one, though:
/// a *run* of inverse cells is a selection, or the block cursor sitting on a
/// draft short enough to fit under it, and either read as empty is the human's
/// own line typed over.
///
/// A style row that does not reach `to` decides nothing and answers false.
/// Today the emulator renders text and styles in lockstep, but the client takes
/// whatever the daemon sends: on a short row the span would be the visible
/// *prefix* of the line, and a dim marker followed by the human's plainly drawn
/// draft would read as empty precisely because the draft was cut off.
///
/// The known way to be wrong is an agent that draws the human's draft faint
/// too, which would read as empty and be typed over. cursor-agent and claude
/// were checked at a real pty and neither does; opencode, crush and pi were
/// not. Those three put no marker on the row their draft lands on, so the scan
/// walks past it, which means this can still be asked about a row *above* the
/// draft; [`PROMPT_SCAN_LINES`] is what keeps that row inside the agent's own
/// chrome rather than out in its output. The symptom, if one appears, is the
/// clobbered prompt this module exists to prevent, so a new agent's empty and
/// typed-in prompt rows are worth a look before trusting this.
fn agent_painted(style_row: &str, from: usize, to: usize) -> bool {
    let cells: Vec<char> = style_row.chars().collect();
    if to > cells.len() {
        return false;
    }
    let Some(span) = cells.get(from..to) else { return false };
    let mut faint = false;
    let mut inverse = 0;
    for c in span {
        match c {
            'd' => faint = true,
            // A gap.
            ' ' => {}
            // The block cursor the agent drew over its own hint.
            'i' => {
                inverse += 1;
                if inverse > 1 {
                    return false;
                }
            }
            // A plainly drawn cell: the human typed this.
            _ => return false,
        }
    }
    faint
}

/// The column `inner`, a slice of `outer`, starts at. Used to line a row of
/// text up with its row of cell styles, which is indexed by column.
///
/// None if `inner` is not in fact a slice of `outer`. Every caller passes one
/// (`strip_frame`, `trim` and `chars().as_str()` all return subslices), but the
/// subtraction below is unchecked pointer arithmetic: an owned or reallocated
/// string underflows it, and the enormous offset that comes back panics on the
/// slice, inside the notifier's 2 s tick. The caller reads None as "cannot
/// tell", which holds the message.
fn column_of(outer: &str, inner: &str) -> Option<usize> {
    let offset = (inner.as_ptr() as usize).checked_sub(outer.as_ptr() as usize)?;
    outer.get(..offset).map(|s| s.chars().count())
}

/// A pane line with the box drawing agents wrap their prompt in taken off, so
/// `│ > hello │` reads as `> hello`.
fn strip_frame(line: &str) -> &str {
    line.trim().trim_matches(|c: char| matches!(c, '│' | '┃' | '┆' | '┊' | '╎' | '|')).trim()
}

/// Human input observed on one session's own keyboard, for the echo-grace and
/// draft rules. Written by the pane's input handler.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HumanInput {
    pub last_key_ms: i64,
    pub draft: bool,
}

impl HumanInput {
    /// Fold one classified write in. Enter clears the draft; a pane read is the
    /// only other thing allowed to (see [`prompt_looks_empty`]).
    pub fn observe(&mut self, typed: Typed, now_ms: i64) {
        match typed {
            Typed::Submitted => {
                self.last_key_ms = now_ms;
                self.draft = false;
            }
            Typed::Edited => {
                self.last_key_ms = now_ms;
                self.draft = true;
            }
            Typed::Nothing => {}
        }
    }
}

/// One session's pending messages, oldest first.
#[derive(Debug, Default)]
pub struct SendQueue {
    pending: VecDeque<Queued>,
}

impl SendQueue {
    /// Queue a message. False if the session is already at [`MAX_PENDING`], in
    /// which case the caller must tell the user rather than dropping it quietly.
    pub fn push(&mut self, msg: Queued) -> bool {
        if self.pending.len() >= MAX_PENDING {
            return false;
        }
        self.pending.push_back(msg);
        true
    }

    pub fn head(&self) -> Option<&Queued> {
        self.pending.front()
    }

    pub fn pop(&mut self) -> Option<Queued> {
        self.pending.pop_front()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Everything waiting, oldest first, for the marker that shows the user
    /// what a run is still owed.
    pub fn messages(&self) -> impl Iterator<Item = &Queued> {
        self.pending.iter()
    }

    /// Drop the first message with this text, and say whether one was there.
    ///
    /// Matched on the text rather than on a position: the drain runs on the
    /// notifier tick, so an index the UI read a second ago may by then point at
    /// a different message, and dropping the wrong one is worse than a click
    /// that reports nothing to drop. Two identical texts are interchangeable.
    pub fn remove(&mut self, text: &str) -> bool {
        match self.pending.iter().position(|q| q.text == text) {
            Some(i) => {
                self.pending.remove(i);
                true
            }
            None => false,
        }
    }
}

/// One queued message as it is stored between launches. The origin travels as a
/// plain string and comes back through the [`ORIGINS`] allowlist; the queueing
/// time does not travel at all (see [`decode`]).
#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    text: String,
    origin: String,
}

/// A queue as one JSON row, for [`decode`] to read back after a quit.
pub fn encode(q: &SendQueue) -> String {
    let stored: Vec<Stored> = q
        .messages()
        .map(|m| Stored { text: m.text.clone(), origin: m.origin.to_string() })
        .collect();
    serde_json::to_string(&stored).unwrap_or_else(|_| "[]".into())
}

/// Read a stored queue back, as of `now_ms`.
///
/// Every message is re-stamped to `now_ms` rather than keeping the time it was
/// first queued. A message held overnight would otherwise be past
/// [`MAX_HOLD_MS`] the moment the app opened, and the timeout branch types
/// regardless of what the pane looks like — so the first thing Agency did on
/// launch would be to append it to whatever the agent was in the middle of. The
/// restored message waits out the ordinary rules instead.
///
/// Anything unreadable — bad JSON, an origin outside [`ORIGINS`], more messages
/// than [`MAX_PENDING`] — is dropped rather than guessed at.
pub fn decode(json: &str, now_ms: i64) -> SendQueue {
    let stored: Vec<Stored> = serde_json::from_str(json).unwrap_or_default();
    let mut q = SendQueue::default();
    for s in stored {
        let Some(origin) = known_origin(&s.origin) else { continue };
        q.push(Queued { text: s.text, origin, queued_at_ms: now_ms });
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(at: i64) -> Queued {
        Queued { text: "please fix the merge".into(), origin: "merge conflict", queued_at_ms: at }
    }

    /// A session with nothing in the way: running, quiet, no draft, no keys.
    fn clear(now_ms: i64) -> Observation {
        Observation {
            now_ms,
            session_running: true,
            working: false,
            last_key_ms: None,
            draft: false,
        }
    }

    #[test]
    fn a_clear_session_takes_the_message() {
        assert_eq!(decide(&msg(0), &clear(0)), Decision::Send(SendReason::Clear));
    }

    #[test]
    fn a_working_agent_holds_the_message() {
        let obs = Observation { working: true, ..clear(1_000) };
        assert_eq!(decide(&msg(0), &obs), Decision::Hold(HoldReason::Working));
    }

    #[test]
    fn a_fresh_keystroke_holds_until_the_echo_grace_passes() {
        let at = |now| Observation { last_key_ms: Some(0), ..clear(now) };
        assert_eq!(decide(&msg(0), &at(0)), Decision::Hold(HoldReason::EchoGrace));
        assert_eq!(
            decide(&msg(0), &at(ECHO_GRACE_MS - 1)),
            Decision::Hold(HoldReason::EchoGrace),
            "still inside the grace"
        );
        assert_eq!(decide(&msg(0), &at(ECHO_GRACE_MS)), Decision::Send(SendReason::Clear));
    }

    #[test]
    fn an_unsent_draft_holds_the_message() {
        let obs = Observation { draft: true, ..clear(5_000) };
        assert_eq!(decide(&msg(0), &obs), Decision::Hold(HoldReason::Draft));
    }

    #[test]
    fn a_gone_session_discards_rather_than_waiting_for_the_timeout() {
        let obs = Observation { session_running: false, ..clear(0) };
        assert_eq!(decide(&msg(0), &obs), Decision::Discard(DiscardReason::SessionGone));
        // Even mid-keystroke, and even with time left on the clock.
        let obs = Observation { session_running: false, last_key_ms: Some(0), ..clear(10) };
        assert_eq!(decide(&msg(0), &obs), Decision::Discard(DiscardReason::SessionGone));
    }

    #[test]
    fn the_timeout_sends_through_a_busy_agent_and_through_a_draft() {
        let obs = Observation { working: true, draft: true, ..clear(MAX_HOLD_MS) };
        assert_eq!(
            decide(&msg(0), &obs),
            Decision::Send(SendReason::Appended),
            "on timeout it appends and submits"
        );
        let obs = Observation { working: true, draft: true, ..clear(MAX_HOLD_MS - 1) };
        assert!(!matches!(decide(&msg(0), &obs), Decision::Send(_)), "not a millisecond early");
    }

    #[test]
    fn a_late_send_into_a_clear_session_is_not_reported_as_appended() {
        // The timeout branch is reached whenever the clock has run out, but
        // with nothing in the way it is an ordinary send that happens to be
        // late — telling the user their text landed on top of something would
        // be a lie.
        assert_eq!(decide(&msg(0), &clear(MAX_HOLD_MS)), Decision::Send(SendReason::Clear));
        let working = Observation { working: true, ..clear(MAX_HOLD_MS) };
        assert_eq!(decide(&msg(0), &working), Decision::Send(SendReason::Appended));
        let draft = Observation { draft: true, ..clear(MAX_HOLD_MS) };
        assert_eq!(decide(&msg(0), &draft), Decision::Send(SendReason::Appended));
    }

    #[test]
    fn the_copy_matches_the_hold_window() {
        // The sentence spells the constant out, so the two must be changed
        // together.
        assert_eq!(MAX_HOLD_MS / 60_000, 5, "MAX_HOLD_WORDS says {MAX_HOLD_WORDS}");
    }

    #[test]
    fn a_dropped_queue_names_what_was_lost_and_why() {
        let one = dropped_notice("fix-login", "Fix login", "review comments", 1);
        assert_eq!(one.kind, NoticeKind::Dropped);
        assert_eq!(
            one.text,
            "Agency dropped the review comments it was holding for Fix login: \
             that session is gone."
        );
        assert_eq!(one.run_id, "fix-login");
        // A discard takes the whole queue, not just the head it decided on.
        let many = dropped_notice("fix-login", "Fix login", "review comments", 3);
        assert!(many.text.contains("the 3 messages"), "{}", many.text);
        // Every origin has a phrase of its own: the toast arrives with the
        // sending surface closed, so "the message" would name nothing.
        for origin in ORIGINS {
            let text = dropped_notice("r", "Fix login", origin, 1).text;
            assert!(!text.contains("the message "), "{origin} has no phrase: {text}");
        }
    }

    #[test]
    fn an_appended_message_says_it_went_in_on_top_of_something() {
        let n = appended_notice("fix-login", "Fix login", "check feedback");
        assert_eq!(n.kind, NoticeKind::Appended);
        assert_eq!(
            n.text,
            "Agency waited five minutes for Fix login to come free, then added the CI feedback \
             to what was already on its prompt line."
        );
    }

    #[test]
    fn the_echo_grace_outranks_the_timeout() {
        // One more tick of waiting is nothing next to writing into a pane whose
        // echo of the human's last keystroke has not landed yet.
        let obs = Observation { last_key_ms: Some(MAX_HOLD_MS), ..clear(MAX_HOLD_MS) };
        assert_eq!(decide(&msg(0), &obs), Decision::Hold(HoldReason::EchoGrace));
    }

    #[test]
    fn typing_is_told_apart_from_mouse_and_focus_traffic() {
        assert_eq!(classify_input(b"h"), Typed::Edited);
        assert_eq!(classify_input("é".as_bytes()), Typed::Edited, "utf-8 is typing");
        assert_eq!(classify_input(b" "), Typed::Edited);
        assert_eq!(classify_input(b"\x7f"), Typed::Edited, "a rubout still edits the line");
        assert_eq!(classify_input(b"fix it\r"), Typed::Submitted);
        assert_eq!(classify_input(b"\n"), Typed::Submitted);
        assert_eq!(classify_input(b"\x1b[A"), Typed::Nothing, "arrow key");
        assert_eq!(classify_input(b"\x1b[I"), Typed::Nothing, "focus in");
        assert_eq!(classify_input(b"\x1bOP"), Typed::Nothing, "SS3 function key");
        assert_eq!(classify_input(b"\x1b"), Typed::Nothing, "a lone escape");
        assert_eq!(classify_input(b"\x03"), Typed::Nothing, "ctrl-c is not typing");
    }

    #[test]
    fn mouse_reports_are_not_typing_in_either_encoding() {
        // SGR (1006): ESC [ < b ; x ; y M — the whole thing is one sequence.
        assert_eq!(classify_input(b"\x1b[<35;80;12M"), Typed::Nothing);
        assert_eq!(classify_input(b"\x1b[<0;5;5m"), Typed::Nothing);
        // X10: ESC [ M then three raw bytes that are ordinary printables. Left
        // unconsumed, every mouse move over the pane would read as typing.
        assert_eq!(classify_input(b"\x1b[M !!"), Typed::Nothing);
        // A drag is a stream of them in one write.
        assert_eq!(classify_input(b"\x1b[<35;1;1M\x1b[<35;2;1M\x1b[<35;3;1M"), Typed::Nothing);
    }

    #[test]
    fn a_bracketed_paste_is_typing() {
        assert_eq!(classify_input(b"\x1b[200~some pasted text\x1b[201~"), Typed::Edited);
    }

    #[test]
    fn an_empty_prompt_box_clears_the_block_but_a_full_one_does_not() {
        let boxed = |line: &str| {
            format!("thinking about the tests…\n╭──────────────╮\n│ {line} │\n╰──────────────╯\n  ? for shortcuts\n")
        };
        assert!(prompt_looks_empty(&boxed(">           "), None));
        assert!(prompt_looks_empty(&boxed("❯           "), None));
        assert!(!prompt_looks_empty(&boxed("> half a tho"), None), "a draft must keep the block");
    }

    #[test]
    fn a_pane_with_no_prompt_in_range_cannot_tell() {
        assert!(!prompt_looks_empty("", None));
        assert!(!prompt_looks_empty("Running tests…\nok 1\nok 2\n", None));
        // A `>` far enough up is scrollback, not the live prompt.
        let mut pane = String::from("> \n");
        for i in 0..PROMPT_SCAN_LINES {
            pane.push_str(&format!("output line {i}\n"));
        }
        assert!(!prompt_looks_empty(&pane, None), "an old prompt in scrollback must not qualify");
    }

    #[test]
    fn a_quoted_line_in_output_does_not_read_as_an_empty_prompt() {
        assert!(!prompt_looks_empty("summary:\n> the fix landed\n", None));
    }

    /// cursor-agent's prompt line, as it draws it: a → rather than a >, and its
    /// own placeholder prose sitting where an empty line would be. Neither the
    /// marker nor the placeholder was known, so a draft in a cursor tab was
    /// never lifted by a pane read and every hand-off to it waited out
    /// MAX_HOLD_MS (AGE-199).
    #[test]
    fn cursor_agents_empty_prompt_line_reads_as_empty() {
        let pane = |line: &str| {
            format!(
                "Updated agency.webp to the released site shot.\n  {line}\n                   Auto · 21.2%   Run Everything\n  ~/Dev/site · agent/overhaul\n"
            )
        };
        assert!(prompt_looks_empty(&pane("→ Add a follow-up"), None));
        assert!(prompt_looks_empty(&pane("→ Plan, search, build anything"), None));
        assert!(prompt_looks_empty(&pane("→"), None));
        assert!(
            !prompt_looks_empty(&pane("→ half a thou"), None),
            "a real draft on cursor's line must still keep the block"
        );
    }

    /// The placeholders are matched whole, not as a prefix: a draft that starts
    /// with one is still a draft, and a line of output that merely contains one
    /// is not the prompt.
    #[test]
    fn a_placeholder_only_counts_as_the_whole_prompt_line() {
        assert!(!prompt_looks_empty("→ Add a follow-up to the changelog\n", None));
        assert!(!prompt_looks_empty("> Add a follow-up, she said\n", None));
    }

    /// A placeholder counts only under the marker it was observed on. The prose
    /// is ordinary enough to be someone's whole prompt, and in a `>` tab nobody
    /// draws it, so reading it as empty would clear the draft block and type the
    /// queued text onto a line the human is still writing.
    #[test]
    fn a_placeholder_does_not_read_as_empty_under_another_agents_marker() {
        assert!(!prompt_looks_empty("> Add a follow-up\n", None));
        assert!(!prompt_looks_empty("❯ Plan, search, build anything\n", None));
        assert!(prompt_looks_empty("> \n", None), "a bare marker is still empty in any agent");
    }

    /// The rows a real cursor-agent pty produced on 2026-09-08, cell styles and
    /// all: its placeholder line, and the same line holding a draft. The styles
    /// alone separate them, so a reworded hint is still read as empty — which
    /// is the whole reason they are on the wire.
    #[test]
    fn a_faintly_drawn_prompt_line_reads_as_empty_whatever_it_says() {
        let hint = "  \u{2192} Plan, search, build anything";
        let hint_style = "  d idddd ddddddd ddddd dddddddd";
        let draft = "  \u{2192} half a thou";
        let draft_style = "  . .... . ....";
        aligned(hint, hint_style);
        aligned(draft, draft_style);

        assert!(prompt_looks_empty(hint, Some(hint_style)));
        assert!(
            !prompt_looks_empty(draft, Some(draft_style)),
            "a plainly drawn line is the human's, whatever it says"
        );

        // The prose is not in PROMPT_PLACEHOLDERS and never has to be.
        let reworded = "  \u{2192} Ask for anything";
        let reworded_style = "  d iddddddddddddddd";
        aligned(reworded, reworded_style);
        assert!(prompt_looks_empty(reworded, Some(reworded_style)));
        assert!(
            !prompt_looks_empty(reworded, None),
            "without the styles the same row is a draft, and holds"
        );
    }

    /// The style map is indexed by column, so it has to survive the box drawing
    /// `strip_frame` takes off the text. An off-by-one here reads the wrong
    /// cells and can call a draft empty.
    #[test]
    fn the_style_map_stays_aligned_through_a_boxed_prompt() {
        let text = "  \u{2502} \u{2192} Plan, search  \u{2502}";
        let style = "  . d idddd dddddd  .";
        aligned(text, style);
        assert!(prompt_looks_empty(text, Some(style)));

        let typed_style = "  . . ..... ......  .";
        aligned(text, typed_style);
        assert!(!prompt_looks_empty(text, Some(typed_style)), "same glyphs, typed by a human");
    }

    /// Both style fixtures above were a cell longer than the row they claimed
    /// to describe (22 against 21, 19 against 20), so the test that says an
    /// off-by-one reads the wrong cells passed only because every span in it
    /// was uniform. Nothing checks a fixture but the fixture's author.
    fn aligned(text: &str, style: &str) {
        assert_eq!(
            text.chars().count(),
            style.chars().count(),
            "the style map is indexed by column, so the fixture has to be one cell per glyph"
        );
    }

    /// A line of the agent's own output that happens to begin with a prompt
    /// marker, drawn dim like the rest of a tool trace, is not the prompt line.
    /// Widening the markers to include → (AGE-199) put a very common trace
    /// idiom in range of a scan that returns on the first marker it finds, and
    /// the all-faint rule answers "empty" for it. Only the scan window keeps it
    /// out: the two panes below differ in nothing but how far up the line sits.
    #[test]
    fn a_dim_trace_line_further_up_the_pane_is_not_the_prompt() {
        let pane = "\u{2192} read\nout\nout\nout\nhalf a thou\n";
        let style = "dddddd\n...\n...\n...\n...........\n";
        for (text, cells) in pane.lines().zip(style.lines()) {
            aligned(text, cells);
        }
        assert!(!prompt_looks_empty(pane, Some(style)));
        assert!(
            prompt_looks_empty("\u{2192} read\nout\n", Some("dddddd\n...\n")),
            "the same row inside the window is the prompt, and answers"
        );
    }

    /// `column_of` is unchecked pointer arithmetic on the assumption that the
    /// inner string is a slice of the outer one. When that stopped being true
    /// the subtraction underflowed, and the enormous offset it returned then
    /// panicked on the slice below, inside the notifier's 2 s tick.
    #[test]
    fn a_column_from_outside_the_row_is_not_a_column() {
        let row = "  \u{2192} hi";
        assert_eq!(column_of(row, &row[2..]), Some(2), "the marker is the third column");
        assert_eq!(column_of(&row[2..], row), None, "an inner string that starts earlier");
        assert_eq!(column_of(&row[..2], &row[5..]), None, "one that ends past the row");
    }

    /// The block cursor is one cell. A whole row of inverse cells is the human
    /// selecting their draft, or the cursor sitting on a draft short enough to
    /// fit under it, and calling that empty types the queued text onto it.
    #[test]
    fn a_run_of_inverse_cells_is_a_draft_not_a_block_cursor() {
        let text = "\u{2192} hi";
        aligned(text, "d ii");
        assert!(!prompt_looks_empty(text, Some("d ii")), "two inverse cells are not a cursor");
        assert!(prompt_looks_empty(text, Some("d id")), "one still is");
    }

    /// A style row shorter than the text row describes only the line's visible
    /// prefix, and reading the span it does cover would call a dim marker
    /// followed by the human's plainly drawn draft empty, on the strength of
    /// the part that was cut off. The emulator sends them in lockstep; the
    /// client takes what the daemon gives it.
    #[test]
    fn a_style_row_that_stops_short_of_the_draft_decides_nothing() {
        let text = "\u{2192} Ask for anything";
        let whole = "didddddddddddddddd";
        aligned(text, whole);
        assert!(!prompt_looks_empty(text, Some("diddd")), "a prefix of the styles decides nothing");
        assert!(prompt_looks_empty(text, Some(whole)), "the whole row answers");
    }

    /// A style map that does not reach the prompt line — a short row, or a
    /// daemon too old to send one — decides nothing on its own and leaves the
    /// prose allowlist to answer.
    #[test]
    fn a_missing_style_row_falls_back_to_the_allowlist() {
        let pane = "\u{2192} Add a follow-up\n";
        assert!(prompt_looks_empty(pane, None));
        assert!(prompt_looks_empty(pane, Some("")));
        assert!(prompt_looks_empty(pane, Some(".")), "a truncated row must not be read past");
        assert!(!prompt_looks_empty("\u{2192} Ask for anything\n", Some("")));
    }

    #[test]
    fn human_input_folds_enter_and_keys_the_way_the_draft_rule_needs() {
        let mut h = HumanInput::default();
        h.observe(Typed::Edited, 1_000);
        assert_eq!(h, HumanInput { last_key_ms: 1_000, draft: true });
        h.observe(Typed::Nothing, 2_000);
        assert_eq!(
            h,
            HumanInput { last_key_ms: 1_000, draft: true },
            "mouse traffic changes nothing"
        );
        h.observe(Typed::Submitted, 3_000);
        assert_eq!(h, HumanInput { last_key_ms: 3_000, draft: false });
    }

    #[test]
    fn a_queue_survives_a_round_trip_through_storage() {
        let mut q = SendQueue::default();
        q.push(Queued {
            text: "CI feedback: 2 check(s) failing".into(),
            origin: "check feedback",
            queued_at_ms: 10,
        });
        q.push(msg(20));
        let back = decode(&encode(&q), 9_000);
        assert_eq!(back.len(), 2);
        let texts: Vec<&str> = back.messages().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["CI feedback: 2 check(s) failing", "please fix the merge"]);
        assert_eq!(back.head().unwrap().origin, "check feedback");
        assert!(
            back.messages().all(|m| m.queued_at_ms == 9_000),
            "the hold clock restarts at launch, or the timeout fires on the first tick"
        );
    }

    #[test]
    fn a_stored_queue_we_cannot_read_is_dropped_rather_than_typed() {
        assert!(decode("", 0).is_empty(), "not even JSON");
        assert!(decode("[{\"text\":\"hi\"}]", 0).is_empty(), "no origin at all");
        assert!(
            decode("[{\"text\":\"hi\",\"origin\":\"some future feature\"}]", 0).is_empty(),
            "an origin outside the allowlist must not reach a live agent"
        );
        assert_eq!(known_origin("review comments"), Some("review comments"));
        assert_eq!(known_origin("Review Comments"), None, "exact values only");
    }

    #[test]
    fn a_stored_queue_cannot_grow_past_the_bound_on_the_way_back_in() {
        let stored: Vec<String> = (0..MAX_PENDING + 4)
            .map(|i| format!("{{\"text\":\"m{i}\",\"origin\":\"merge conflict\"}}"))
            .collect();
        let q = decode(&format!("[{}]", stored.join(",")), 0);
        assert_eq!(q.len(), MAX_PENDING);
    }

    #[test]
    fn a_message_is_dropped_by_its_text_and_a_stale_click_reports_nothing() {
        let mut q = SendQueue::default();
        q.push(msg(1));
        q.push(Queued { text: "second".into(), origin: "check feedback", queued_at_ms: 2 });
        assert!(q.remove("please fix the merge"));
        assert_eq!(q.len(), 1);
        assert_eq!(q.head().unwrap().text, "second");
        assert!(!q.remove("please fix the merge"), "already gone: nothing to drop");
    }

    #[test]
    fn the_queue_is_fifo_and_bounded() {
        let mut q = SendQueue::default();
        assert!(q.is_empty());
        for i in 0..MAX_PENDING as i64 {
            assert!(q.push(msg(i)), "message {i} must fit");
        }
        assert_eq!(q.len(), MAX_PENDING);
        assert!(!q.push(msg(99)), "a full queue must refuse rather than grow");
        assert_eq!(q.head().unwrap().queued_at_ms, 0, "oldest first");
        assert_eq!(q.pop().unwrap().queued_at_ms, 0);
        assert_eq!(q.head().unwrap().queued_at_ms, 1);
    }
}
