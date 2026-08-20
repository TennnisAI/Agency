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
/// live prompt line. Deep enough to clear the hint lines agents draw under
/// their prompt box, shallow enough that a `>` left in scrollback cannot pass
/// for the prompt itself.
const PROMPT_SCAN_LINES: usize = 8;

/// Prompt markers agents draw at the start of their input line.
const PROMPT_MARKERS: [char; 3] = ['>', '❯', '›'];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Type the text, then the carriage return.
    Send,
    Hold(HoldReason),
    Discard(DiscardReason),
}

/// Whether the head of a session's queue may go out right now.
///
/// Order matters. A gone session discards before anything else looks at the
/// clock. The echo grace outranks the timeout because it is a single second and
/// the pane it protects is the evidence every later rule reads. The timeout
/// then outranks both the busy and the draft holds — that is what "on timeout,
/// append" means.
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
        return Decision::Send;
    }
    if obs.working {
        return Decision::Hold(HoldReason::Working);
    }
    if obs.draft {
        return Decision::Hold(HoldReason::Draft);
    }
    Decision::Send
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
/// True only when a prompt marker is found with nothing after it. Everything
/// else — text after the marker, no marker in range, a pane we cannot parse —
/// is "cannot tell", which holds. That asymmetry is the point: this may only
/// clear a draft block, never create one, so a wrong answer costs a few seconds
/// of delay rather than a clobbered prompt.
pub fn prompt_looks_empty(pane: &str) -> bool {
    for line in
        pane.lines().rev().map(strip_frame).filter(|l| !l.is_empty()).take(PROMPT_SCAN_LINES)
    {
        let mut chars = line.chars();
        let Some(first) = chars.next() else { continue };
        if PROMPT_MARKERS.contains(&first) {
            return chars.as_str().trim().is_empty();
        }
    }
    false
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
        assert_eq!(decide(&msg(0), &clear(0)), Decision::Send);
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
        assert_eq!(decide(&msg(0), &at(ECHO_GRACE_MS)), Decision::Send);
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
        assert_eq!(decide(&msg(0), &obs), Decision::Send, "on timeout it appends and submits");
        let obs = Observation { working: true, draft: true, ..clear(MAX_HOLD_MS - 1) };
        assert_ne!(decide(&msg(0), &obs), Decision::Send, "not a millisecond early");
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
        assert!(prompt_looks_empty(&boxed(">           ")));
        assert!(prompt_looks_empty(&boxed("❯           ")));
        assert!(!prompt_looks_empty(&boxed("> half a tho")), "a draft must keep the block");
    }

    #[test]
    fn a_pane_with_no_prompt_in_range_cannot_tell() {
        assert!(!prompt_looks_empty(""));
        assert!(!prompt_looks_empty("Running tests…\nok 1\nok 2\n"));
        // A `>` far enough up is scrollback, not the live prompt.
        let mut pane = String::from("> \n");
        for i in 0..PROMPT_SCAN_LINES {
            pane.push_str(&format!("output line {i}\n"));
        }
        assert!(!prompt_looks_empty(&pane), "an old prompt in scrollback must not qualify");
    }

    #[test]
    fn a_quoted_line_in_output_does_not_read_as_an_empty_prompt() {
        assert!(!prompt_looks_empty("summary:\n> the fix landed\n"));
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
