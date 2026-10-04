//! Per-run activity tracking: what each session's agent is doing, for the UI
//! to badge and time.
//!
//! Two sources, ranked (AGE-206). An agent with lifecycle hooks
//! (`agency_core::state_hooks`, Claude Code today) says what it is doing, and
//! while it does that is the answer. Everything else is read off the pane: the
//! notifier tick captures every pane anyway, and "did it change" is one bit.
//!
//! - **working** — reported, or the pane changed recently. The one state the
//!   pane gets right: agents redraw while they work.
//! - **blocked** — reported only. Stopped on a permission dialog or a question,
//!   and cannot go on until the user answers it. A pane cannot see this: a
//!   blocked agent and a finished one both sit quiet.
//! - **done** — reported when the turn ends. Without hooks it is the pane's
//!   guess at the same thing, quiet after a turn the user drove, which is why
//!   [`ActivityInfo::reported`] goes to the UI with it: an inferred `done` may
//!   really be blocked, and the board says so. The guess decays to idle after
//!   [`DONE_MAX_MS`] so a long-ignored run doesn't wear an attention badge
//!   forever; a reported one does not, since the agent said so and nothing has
//!   changed.
//! - **idle** — nothing in flight: a fresh agent nobody has prompted, a session
//!   that ended or was cleared, or a guessed `done` the user let lapse.
//!
//! A pin (`pin_rank` on the run) is board order and nothing else. It is a
//! stored column the UI reads straight off `RunInfo`, so it is not this
//! module's business: nothing here derives from it or is derived by it.

/// How long the pane may stay unchanged before a run stops counting as
/// working. Agents redraw constantly while working (spinners, streaming
/// output), so anything quieter than this is sitting at a prompt. Comfortably
/// above the 2s poll so a single slow tick can't flap the state.
pub const WORKING_TTL_MS: i64 = 10_000;

/// How long a reported `working` holds with no further report. It is the one
/// reported state that needs a heartbeat: the tool hooks fire all through a
/// turn, and an Esc mid-turn ends the turn with no `Stop`, so a `working` that
/// has gone unrenewed falls back to the pane. The pane is right about working,
/// so the fallback costs nothing: a long tool call still animates a spinner.
pub const REPORT_WORKING_TTL_MS: i64 = WORKING_TTL_MS;

/// How long a guessed `done` stays before decaying to plain idle. Past this
/// the user has *probably* deprioritized the run — an urgent badge and a
/// climbing timer stop being signal.
pub const DONE_MAX_MS: i64 = 30 * 60 * 1000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityEntry {
    /// Last time the pane content changed.
    pub last_change_ms: i64,
    /// When the current busy streak began (meaningful only while working).
    pub busy_since_ms: i64,
    /// Whether the run was within [`WORKING_TTL_MS`] at the last tick; only
    /// used by `update` to anchor busy streaks. Readers should classify from
    /// the timestamps instead (see [`classify`]) so state keeps advancing
    /// between ticks.
    pub working: bool,
    /// The last thing the agent's own hooks said, if it has any.
    pub report: Option<Report>,
}

/// One session's last report, from its lifecycle hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub state: ActivityState,
    /// When the session entered this state. A repeat of the same state (every
    /// tool call reports `working`) keeps it.
    pub since_ms: i64,
    /// The latest report, which for `working` is the heartbeat.
    pub at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityState {
    Working,
    Blocked,
    Done,
    Idle,
}

impl From<agency_core::preview::report::Reported> for ActivityState {
    fn from(r: agency_core::preview::report::Reported) -> ActivityState {
        use agency_core::preview::report::Reported;
        match r {
            Reported::Working => ActivityState::Working,
            Reported::Blocked => ActivityState::Blocked,
            Reported::Done => ActivityState::Done,
            Reported::Idle => ActivityState::Idle,
        }
    }
}

/// What the UI sees on `RunInfo`: the state plus when it began, so elapsed
/// time is computable client-side without further round-trips.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityInfo {
    pub state: ActivityState,
    /// Epoch ms when the current state began: busy-streak start while
    /// working, last pane change otherwise.
    pub since: i64,
    /// The agent said so, rather than the pane suggesting it. A `done` that is
    /// not reported is "quiet after a turn", which may be a prompt.
    pub reported: bool,
}

/// Advance one run's activity bookkeeping given whether its pane changed this
/// tick. First observation counts as working — a freshly spawned run is busy
/// starting up, and a long-idle run decays within `WORKING_TTL_MS`.
pub fn update(prev: Option<ActivityEntry>, pane_changed: bool, now_ms: i64) -> ActivityEntry {
    let Some(p) = prev else { return fresh(now_ms) };
    let last_change_ms = if pane_changed { now_ms } else { p.last_change_ms };
    let working = now_ms - last_change_ms < WORKING_TTL_MS;
    // An idle→working edge anchors the new busy streak at the change itself.
    let busy_since_ms = if working && !p.working { last_change_ms } else { p.busy_since_ms };
    ActivityEntry { last_change_ms, busy_since_ms, working, report: p.report }
}

fn fresh(now_ms: i64) -> ActivityEntry {
    ActivityEntry { last_change_ms: now_ms, busy_since_ms: now_ms, working: true, report: None }
}

/// Fold in one report from the session's hooks. A report can beat the
/// notifier's first look at a session, so it may start the entry.
pub fn reported(prev: Option<ActivityEntry>, state: ActivityState, now_ms: i64) -> ActivityEntry {
    let mut e = prev.unwrap_or_else(|| fresh(now_ms));
    let since_ms = match e.report {
        Some(r) if r.state == state => r.since_ms,
        _ => now_ms,
    };
    e.report = Some(Report { state, since_ms, at_ms: now_ms });
    e
}

/// The user pressed a key that can answer a prompt (see
/// `sendq::answers_a_prompt`), which ends a reported `blocked`.
///
/// Answering "No" to a Claude Code permission dialog, or pressing Esc on it,
/// fires no hook at all (2.1.289), so without this the run would read blocked
/// until the agent's next turn. Approving fires `PostToolUse` soon after, which
/// says `working` on its own; this only decides what is shown in between, and
/// the pane's answer is the right one for that.
pub fn answered(entry: ActivityEntry) -> ActivityEntry {
    match entry.report {
        Some(r) if r.state == ActivityState::Blocked => ActivityEntry { report: None, ..entry },
        _ => entry,
    }
}

/// Classify a run's current state from its bookkeeping. `turn_driven` says a
/// user-driven turn is in flight (typed prompt or created-with-prompt, and not
/// a self-driving loop) — without it a quiet pane is idle, never done.
///
/// Whether the session is alive is the caller's to compose, and every caller
/// does: a dead process is neither blocked nor done, it is gone, and every
/// surface that shows this already checks the session's status first.
pub fn classify(entry: &ActivityEntry, turn_driven: bool, now_ms: i64) -> ActivityInfo {
    match entry.report {
        Some(r) if r.state != ActivityState::Working => {
            ActivityInfo { state: r.state, since: r.since_ms, reported: true }
        }
        Some(r) if now_ms - r.at_ms < REPORT_WORKING_TTL_MS => {
            ActivityInfo { state: ActivityState::Working, since: r.since_ms, reported: true }
        }
        // The agent said a turn started and has gone quiet about it: the pane
        // decides. With the caller's `turn_driven`, not "the agent said so": a
        // headless loop attempt draws nothing while its model thinks, and
        // reading that quiet as a finished turn would flash every loop amber
        // between its tool calls.
        Some(r) => match from_pane(entry, turn_driven, now_ms) {
            ActivityInfo { state: ActivityState::Working, .. } => {
                ActivityInfo { state: ActivityState::Working, since: r.since_ms, reported: false }
            }
            other => other,
        },
        None => from_pane(entry, turn_driven, now_ms),
    }
}

fn from_pane(entry: &ActivityEntry, turn_driven: bool, now_ms: i64) -> ActivityInfo {
    let quiet_ms = now_ms - entry.last_change_ms;
    if quiet_ms < WORKING_TTL_MS {
        let since = entry.busy_since_ms;
        return ActivityInfo { state: ActivityState::Working, since, reported: false };
    }
    let state = if turn_driven && quiet_ms < DONE_MAX_MS {
        ActivityState::Done
    } else {
        ActivityState::Idle
    };
    ActivityInfo { state, since: entry.last_change_ms, reported: false }
}

/// Whether the run is quiet now after a busy streak that began strictly after
/// something happened at `at_ms`: that is, whether its agent has gone on to a
/// separate stretch of work since and finished it.
///
/// Not `at_ms < last_change_ms`. An agent's own tool call is drawn in its
/// pane, so the last pane change always postdates the call, and that test
/// called every line an agent set as its last act stale (AGE-208 review). The
/// streak boundary is what separates "then" from "since": a new streak only
/// starts after [`WORKING_TTL_MS`] with no pane change at all, and the call's
/// own redraw is a change, so any later streak begins more than that past
/// `at_ms`. The call's own streak begins at most a tick after it, which is why
/// the margin is the TTL rather than zero: a quiet agent whose call is the
/// first thing it draws starts its streak on the tick that sees the call.
pub fn worked_and_went_quiet_since(entry: &ActivityEntry, at_ms: i64, now_ms: i64) -> bool {
    now_ms - entry.last_change_ms >= WORKING_TTL_MS && entry.busy_since_ms > at_ms + WORKING_TTL_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(e: &ActivityEntry, turn_driven: bool, now: i64) -> ActivityState {
        classify(e, turn_driven, now).state
    }

    #[test]
    fn first_observation_is_working() {
        let e = update(None, true, 1_000);
        assert_eq!(state(&e, false, 1_000), ActivityState::Working);
        assert_eq!(classify(&e, false, 1_000).since, 1_000);
    }

    #[test]
    fn stays_working_while_pane_keeps_changing() {
        let mut e = update(None, true, 0);
        for t in (2_000..20_000).step_by(2_000) {
            e = update(Some(e), true, t);
            assert_eq!(state(&e, true, t), ActivityState::Working);
            assert_eq!(e.busy_since_ms, 0, "busy streak must keep its anchor");
        }
    }

    #[test]
    fn quiet_run_waits_only_when_turn_driven() {
        let mut e = update(None, true, 0);
        e = update(Some(e), true, 2_000); // last output at 2s
        let quiet = 2_000 + WORKING_TTL_MS;
        e = update(Some(e), false, quiet);
        assert_eq!(state(&e, true, quiet), ActivityState::Done, "turn driven: done");
        assert_eq!(state(&e, false, quiet), ActivityState::Idle, "never prompted: just idle");
        assert_eq!(classify(&e, true, quiet).since, 2_000, "done since = last pane change");
    }

    #[test]
    fn a_guessed_done_decays_to_idle_after_the_cap() {
        let mut e = update(None, true, 0);
        let just_under = DONE_MAX_MS - 1;
        e = update(Some(e), false, just_under);
        assert_eq!(state(&e, true, just_under), ActivityState::Done);
        assert_eq!(state(&e, true, DONE_MAX_MS), ActivityState::Idle, "cap reached: idle");
        assert_eq!(classify(&e, true, DONE_MAX_MS).since, 0);
    }

    #[test]
    fn classify_advances_between_ticks() {
        // Last tick left the entry marked working; classification at read time
        // must still go quiet once the TTL passes with no further ticks.
        let e = update(None, true, 0);
        assert!(e.working);
        assert_eq!(state(&e, true, WORKING_TTL_MS), ActivityState::Done);
    }

    #[test]
    fn idle_to_working_starts_a_new_busy_streak() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, WORKING_TTL_MS); // idle
        assert!(!e.working);
        e = update(Some(e), true, 60_000); // output again
        assert_eq!(state(&e, false, 60_000), ActivityState::Working);
        assert_eq!(e.busy_since_ms, 60_000);
        assert_eq!(classify(&e, false, 60_000).since, 60_000);
    }

    #[test]
    fn quiet_gaps_within_ttl_do_not_reset_the_streak() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, 4_000); // brief lull (long tool call)
        e = update(Some(e), true, 8_000); // output resumes before TTL
        assert_eq!(state(&e, false, 8_000), ActivityState::Working);
        assert_eq!(e.busy_since_ms, 0, "lull under TTL must not restart the streak");
    }

    /// An agent that sets a line as its last act, while quiet, and stops: the
    /// call's redraw starts a streak on the next tick, and that streak is the
    /// call's own, not work done since.
    #[test]
    fn a_last_act_does_not_count_as_work_since() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, 60_000); // long quiet
        e = update(Some(e), true, 62_000); // the set_status call at 61s, seen at 62s
        let quiet = 62_000 + WORKING_TTL_MS;
        e = update(Some(e), false, quiet);
        assert!(!worked_and_went_quiet_since(&e, 61_000, quiet));
    }

    #[test]
    fn a_call_inside_a_streak_does_not_count_as_work_since() {
        let mut e = update(None, true, 0);
        for t in (2_000..=20_000).step_by(2_000) {
            e = update(Some(e), true, t);
        }
        let quiet = 20_000 + WORKING_TTL_MS;
        e = update(Some(e), false, quiet);
        assert!(!worked_and_went_quiet_since(&e, 8_000, quiet));
    }

    #[test]
    fn a_later_streak_that_has_gone_quiet_counts() {
        let mut e = update(None, true, 0); // the call at 0, in this streak
        e = update(Some(e), false, WORKING_TTL_MS); // quiet: streak over
        e = update(Some(e), true, 30_000); // another stretch of work
        assert!(!worked_and_went_quiet_since(&e, 0, 30_000), "still working is not gone quiet");
        let quiet = 30_000 + WORKING_TTL_MS;
        e = update(Some(e), false, quiet);
        assert!(worked_and_went_quiet_since(&e, 0, quiet));
    }

    #[test]
    fn info_serializes_camel_case_for_the_ui() {
        let e = update(None, true, 1_234);
        let info = classify(&e, true, 1_234 + WORKING_TTL_MS);
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(json, r#"{"state":"done","since":1234,"reported":false}"#);
        let e = reported(Some(e), ActivityState::Blocked, 5_000);
        let json = serde_json::to_string(&classify(&e, true, 9_000)).unwrap();
        assert_eq!(json, r#"{"state":"blocked","since":5000,"reported":true}"#);
    }

    /// The case the pane cannot tell apart, and the reason reports exist: two
    /// quiet panes, one blocked and one done.
    #[test]
    fn a_report_tells_blocked_from_done_where_the_pane_cannot() {
        let quiet = |r: ActivityState| {
            let e = update(None, true, 0);
            let e = reported(Some(e), r, 1_000);
            update(Some(e), false, 60_000)
        };
        let (b, d) = (quiet(ActivityState::Blocked), quiet(ActivityState::Done));
        assert_eq!(from_pane(&b, true, 60_000).state, from_pane(&d, true, 60_000).state);
        assert_eq!(state(&b, true, 60_000), ActivityState::Blocked);
        assert_eq!(state(&d, true, 60_000), ActivityState::Done);
        assert!(classify(&b, true, 60_000).reported);
    }

    #[test]
    fn resting_reports_outrank_the_pane_and_never_decay() {
        let mut e = update(None, true, 0);
        e = reported(Some(e), ActivityState::Done, 1_000);
        // The agent redraws as its turn ends, and the user types at the
        // prompt: the pane changes, and the turn is still over.
        e = update(Some(e), true, 2_000);
        assert_eq!(state(&e, false, 2_000), ActivityState::Done);
        let later = 1_000 + DONE_MAX_MS * 4;
        e = update(Some(e), false, later);
        assert_eq!(state(&e, false, later), ActivityState::Done, "the agent said so");
        assert_eq!(classify(&e, false, later).since, 1_000);
        e = reported(Some(e), ActivityState::Idle, later);
        assert_eq!(state(&e, true, later), ActivityState::Idle, "session ended or cleared");
    }

    #[test]
    fn a_reported_working_needs_its_heartbeat() {
        let e = reported(None, ActivityState::Working, 0);
        assert_eq!(state(&e, false, REPORT_WORKING_TTL_MS - 1), ActivityState::Working);
        // Every tool call renews it, keeping the turn's start.
        let e = reported(Some(e), ActivityState::Working, 8_000);
        let info = classify(&e, false, 8_000 + REPORT_WORKING_TTL_MS - 1);
        assert_eq!((info.state, info.since, info.reported), (ActivityState::Working, 0, true));
        // Unrenewed (an Esc mid-turn sends no Stop): the pane decides, the
        // same way it would with no report at all.
        let quiet = update(Some(e), false, 8_000 + REPORT_WORKING_TTL_MS);
        let info = classify(&quiet, true, 8_000 + REPORT_WORKING_TTL_MS);
        assert_eq!((info.state, info.reported), (ActivityState::Done, false));
        // A loop attempt is never turn-driven, and its quiet pane mid-thought
        // stays idle rather than reading as a finished turn.
        let info = classify(&quiet, false, 8_000 + REPORT_WORKING_TTL_MS);
        assert_eq!(info.state, ActivityState::Idle);
        // A long tool call still animates the pane, and stays working.
        let busy = update(Some(e), true, 8_000 + REPORT_WORKING_TTL_MS);
        let info = classify(&busy, false, 8_000 + REPORT_WORKING_TTL_MS);
        assert_eq!((info.state, info.since), (ActivityState::Working, 0));
    }

    #[test]
    fn a_new_state_restarts_since_and_a_repeat_keeps_it() {
        let e = reported(None, ActivityState::Working, 100);
        let e = reported(Some(e), ActivityState::Working, 200);
        assert_eq!(e.report.unwrap().since_ms, 100);
        let e = reported(Some(e), ActivityState::Blocked, 300);
        assert_eq!(e.report.unwrap().since_ms, 300);
    }

    /// "No" and Esc on a permission dialog fire nothing (Claude Code 2.1.289).
    #[test]
    fn answering_ends_blocked_and_only_blocked() {
        let mut e = update(None, true, 0);
        e = reported(Some(e), ActivityState::Blocked, 1_000);
        let after = answered(e);
        assert_eq!(after.report, None);
        let after = update(Some(after), true, 2_000); // the dialog closes
        assert_eq!(state(&after, true, 2_000), ActivityState::Working);
        assert_eq!(state(&after, true, 2_000 + WORKING_TTL_MS), ActivityState::Done);
        // A key typed at a finished turn is the next prompt being written.
        let done = reported(Some(e), ActivityState::Done, 3_000);
        assert_eq!(answered(done), done);
    }

    #[test]
    fn a_report_may_come_before_the_first_tick() {
        let e = reported(None, ActivityState::Blocked, 500);
        assert_eq!(state(&e, false, 500), ActivityState::Blocked);
        let e = update(Some(e), true, 2_000);
        assert_eq!(state(&e, false, 2_000), ActivityState::Blocked, "ticks keep the report");
    }

    #[test]
    fn hook_reports_map_one_to_one() {
        use agency_core::preview::report::Reported;
        assert_eq!(ActivityState::from(Reported::Working), ActivityState::Working);
        assert_eq!(ActivityState::from(Reported::Blocked), ActivityState::Blocked);
        assert_eq!(ActivityState::from(Reported::Done), ActivityState::Done);
        assert_eq!(ActivityState::from(Reported::Idle), ActivityState::Idle);
    }
}
