//! Per-run activity tracking, fed by the notifier tick's pane-hash diff.
//!
//! The notifier thread already captures every run's pane each tick to drive
//! notifications; this module turns the same "did the pane change" bit into a
//! live state the UI can badge and time:
//!
//! - **working** — the pane changed recently: the agent is producing output.
//! - **waiting** — quiet after a turn the user actually drove (typed prompt or
//!   a run created with one): the agent finished or is blocked on input.
//!   Decays to idle after [`WAITING_MAX_MS`] so a long-ignored run doesn't
//!   wear an attention badge (and an ever-growing counter) forever.
//! - **idle** — quiet with no turn in flight: a fresh agent nobody has
//!   prompted yet, or a waiting run the user let lapse.
//!
//! Over the top of that sits the user's own word on the run — settled, active
//! or snoozed (`agency_core::attention`). The derived state stays derived; the
//! standing says what to do with it, and [`standing_now`] says whether the
//! standing still holds. The time decay above is the fallback for a run nobody
//! has classified: it stops applying the moment the user speaks, because past
//! that point there is nothing left to guess.

use agency_core::attention::{Standing, StandingKind};

/// How long the pane may stay unchanged before a run stops counting as
/// working. Agents redraw constantly while working (spinners, streaming
/// output), so anything quieter than this is sitting at a prompt. Comfortably
/// above the 2s poll so a single slow tick can't flap the state.
pub const WORKING_TTL_MS: i64 = 10_000;

/// How long a quiet run stays "waiting" before decaying to plain idle. Past
/// this the user has *probably* deprioritized the run — an urgent badge and a
/// climbing timer stop being signal. A guess, and only a guess: it applies to
/// a run carrying no [`Standing`], and to no other.
pub const WAITING_MAX_MS: i64 = 30 * 60 * 1000;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityState {
    Working,
    Waiting,
    Idle,
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
}

/// Advance one run's activity bookkeeping given whether its pane changed this
/// tick. First observation counts as working — a freshly spawned run is busy
/// starting up, and a long-idle run decays within `WORKING_TTL_MS`.
pub fn update(prev: Option<ActivityEntry>, pane_changed: bool, now_ms: i64) -> ActivityEntry {
    let Some(p) = prev else {
        return ActivityEntry { last_change_ms: now_ms, busy_since_ms: now_ms, working: true };
    };
    let last_change_ms = if pane_changed { now_ms } else { p.last_change_ms };
    let working = now_ms - last_change_ms < WORKING_TTL_MS;
    // An idle→working edge anchors the new busy streak at the change itself.
    let busy_since_ms = if working && !p.working { last_change_ms } else { p.busy_since_ms };
    ActivityEntry { last_change_ms, busy_since_ms, working }
}

/// Classify a run's current state from its bookkeeping. `turn_driven` says a
/// user-driven turn is in flight (typed prompt or created-with-prompt, and not
/// a self-driving loop) — without it a quiet run is idle, never waiting.
///
/// `standing` is the user's recorded word on the run, and its only job here is
/// to switch the time decay off: a classified run keeps its waiting state for
/// as long as it is quiet, however long that is. Which *kind* of word it is
/// makes no difference to the derived state, only to whether the run is
/// surfaced (see [`standing_now`]) — a snoozed run "stays active in the model
/// and is only suppressed from the attention list", and it has to still be
/// waiting when the snooze runs out or waking it would show nothing.
pub fn classify(
    entry: &ActivityEntry,
    turn_driven: bool,
    standing: Option<&Standing>,
    now_ms: i64,
) -> ActivityInfo {
    let quiet_ms = now_ms - entry.last_change_ms;
    let decayed = standing.is_none() && quiet_ms >= WAITING_MAX_MS;
    if quiet_ms < WORKING_TTL_MS {
        ActivityInfo { state: ActivityState::Working, since: entry.busy_since_ms }
    } else if turn_driven && !decayed {
        ActivityInfo { state: ActivityState::Waiting, since: entry.last_change_ms }
    } else {
        ActivityInfo { state: ActivityState::Idle, since: entry.last_change_ms }
    }
}

/// The user's standing on a run as it applies *right now*, or `None` if they
/// have said nothing or what they said has been used up.
///
/// A settle and a snooze are both answers to "not now", and both are consumed
/// by the run coming back: output the user has not seen (later than `at_ms`)
/// that has since gone quiet on a turn they drove — which is exactly the run
/// raising its hand. A snooze also lapses on its own clock. `Active` is never
/// consumed; only the user takes it back.
///
/// Waiting is the whole test for "came back", not the bare timestamp, and that
/// is deliberate. A run mid-turn has not finished producing, so settling one
/// stays settled while it works — which is what makes a settled loop quiet
/// (a loop is self-driving, never turn-driven, so it never reaches waiting)
/// instead of un-settling itself on the next spinner frame.
pub fn standing_now(
    standing: Option<&Standing>,
    entry: Option<&ActivityEntry>,
    waiting: bool,
    now_ms: i64,
) -> Option<StandingKind> {
    let standing = standing?;
    let came_back = waiting && entry.is_some_and(|e| e.last_change_ms > standing.at_ms);
    match standing.kind {
        StandingKind::Active => Some(StandingKind::Active),
        _ if came_back => None,
        StandingKind::Snoozed { until_ms } if now_ms >= until_ms => None,
        kind => Some(kind),
    }
}

/// What the user has said about a run and what it means right now, as the UI
/// sees it on `RunInfo`. Always present, unlike [`ActivityInfo`]: a pin is the
/// user's, not a sample the notifier may not have taken yet.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionInfo {
    /// The live standing from [`standing_now`], never the stale record: a
    /// consumed settle and a woken snooze both report `None`.
    pub standing: Option<StandingKind>,
    /// Ascending order among pinned runs; `None` = unpinned. Untouched by
    /// activity — a pin holds regardless of lifecycle.
    pub pin_rank: Option<f64>,
    /// The bottom line the attention list reads: this run is waiting on the
    /// user and nothing they have said suppresses it. Says nothing about the
    /// session being alive or the run being an agent rather than a terminal —
    /// the UI composes those in (`runstate.ts`), because they are the same
    /// conditions its dots and labels already key off.
    pub needs_attention: bool,
}

/// Assemble the attention read for one run: its live standing, its pin, and
/// whether it belongs on the attention list. `state` is `None` for a run the
/// notifier has not observed yet.
pub fn attention(
    standing: Option<&Standing>,
    pin_rank: Option<f64>,
    entry: Option<&ActivityEntry>,
    state: Option<ActivityState>,
    now_ms: i64,
) -> AttentionInfo {
    let waiting = state == Some(ActivityState::Waiting);
    let live = standing_now(standing, entry, waiting, now_ms);
    AttentionInfo {
        standing: live,
        pin_rank,
        needs_attention: waiting && !live.is_some_and(|k| k.suppresses()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(e: &ActivityEntry, turn_driven: bool, now: i64) -> ActivityState {
        classify(e, turn_driven, None, now).state
    }

    /// A run the user has classified, `at_ms` ago.
    fn said(kind: StandingKind, at_ms: i64) -> Standing {
        Standing::new(kind, at_ms)
    }

    #[test]
    fn first_observation_is_working() {
        let e = update(None, true, 1_000);
        assert_eq!(state(&e, false, 1_000), ActivityState::Working);
        assert_eq!(classify(&e, false, None, 1_000).since, 1_000);
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
        assert_eq!(state(&e, true, quiet), ActivityState::Waiting, "turn driven: waiting");
        assert_eq!(state(&e, false, quiet), ActivityState::Idle, "never prompted: just idle");
        assert_eq!(
            classify(&e, true, None, quiet).since,
            2_000,
            "waiting since = last pane change"
        );
    }

    #[test]
    fn waiting_decays_to_idle_after_the_cap() {
        let mut e = update(None, true, 0);
        let just_under = WAITING_MAX_MS - 1;
        e = update(Some(e), false, just_under);
        assert_eq!(state(&e, true, just_under), ActivityState::Waiting);
        assert_eq!(state(&e, true, WAITING_MAX_MS), ActivityState::Idle, "cap reached: idle");
        assert_eq!(classify(&e, true, None, WAITING_MAX_MS).since, 0);
    }

    #[test]
    fn classify_advances_between_ticks() {
        // Last tick left the entry marked working; classification at read time
        // must still go quiet once the TTL passes with no further ticks.
        let e = update(None, true, 0);
        assert!(e.working);
        assert_eq!(state(&e, true, WORKING_TTL_MS), ActivityState::Waiting);
    }

    #[test]
    fn idle_to_working_starts_a_new_busy_streak() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, WORKING_TTL_MS); // idle
        assert!(!e.working);
        e = update(Some(e), true, 60_000); // output again
        assert_eq!(state(&e, false, 60_000), ActivityState::Working);
        assert_eq!(e.busy_since_ms, 60_000);
        assert_eq!(classify(&e, false, None, 60_000).since, 60_000);
    }

    #[test]
    fn quiet_gaps_within_ttl_do_not_reset_the_streak() {
        let mut e = update(None, true, 0);
        e = update(Some(e), false, 4_000); // brief lull (long tool call)
        e = update(Some(e), true, 8_000); // output resumes before TTL
        assert_eq!(state(&e, false, 8_000), ActivityState::Working);
        assert_eq!(e.busy_since_ms, 0, "lull under TTL must not restart the streak");
    }

    // ── the user's own word (AGE-141) ───────────────────────────────────────

    /// The decay is the fallback for a run nobody has classified. Once the
    /// user has spoken there is nothing left to guess, so it stops applying —
    /// for every standing, not just the one that means "this matters".
    #[test]
    fn a_classified_run_never_decays_to_idle() {
        let e = update(None, true, 0);
        let long_after = WAITING_MAX_MS * 10;
        assert_eq!(state(&e, true, long_after), ActivityState::Idle, "unclassified: decayed");
        for kind in
            [StandingKind::Active, StandingKind::Settled, StandingKind::Snoozed { until_ms: 1 }]
        {
            let st = said(kind, 0);
            assert_eq!(
                classify(&e, true, Some(&st), long_after).state,
                ActivityState::Waiting,
                "{kind:?} must hold the waiting state"
            );
        }
    }

    /// A snooze that has run out has to find the run still waiting, or waking
    /// it would surface nothing.
    #[test]
    fn a_snooze_wakes_a_run_that_is_still_waiting() {
        let e = update(None, true, 0);
        let woke = WAITING_MAX_MS * 3;
        let st = said(StandingKind::Snoozed { until_ms: woke - 1 }, 0);
        let info = classify(&e, true, Some(&st), woke);
        assert_eq!(info.state, ActivityState::Waiting);
        assert!(attention(Some(&st), None, Some(&e), Some(info.state), woke).needs_attention);
    }

    #[test]
    fn settling_holds_until_the_run_comes_back() {
        let mut e = update(None, true, 0);
        let quiet = WORKING_TTL_MS;
        let st = said(StandingKind::Settled, quiet);
        let att = |e: &ActivityEntry, now: i64| {
            let state = classify(e, true, Some(&st), now).state;
            attention(Some(&st), None, Some(e), Some(state), now)
        };
        assert_eq!(att(&e, quiet).standing, Some(StandingKind::Settled), "just settled");
        assert!(!att(&e, quiet).needs_attention, "settled: off the list");

        // The agent speaks again, and goes quiet on the user's turn: that is
        // the run coming back, and it un-settles itself.
        e = update(Some(e), true, quiet + 1_000);
        assert_eq!(att(&e, quiet + 1_000).standing, Some(StandingKind::Settled), "mid-turn: held");
        let back = quiet + 1_000 + WORKING_TTL_MS;
        assert_eq!(att(&e, back).standing, None, "came back: the settle is used up");
        assert!(att(&e, back).needs_attention);
    }

    /// The composition the loop suppression wants: a loop drives itself, so it
    /// never reaches waiting, so a settle on it is never consumed. Its pane
    /// churns the whole time and that must not undo the settle.
    #[test]
    fn a_settled_loop_stays_settled_through_its_own_churn() {
        let st = said(StandingKind::Settled, 0);
        let mut e = update(None, true, 0);
        for t in (2_000..600_000).step_by(2_000) {
            e = update(Some(e), true, t);
            // A loop is never turn-driven, so it is never waiting.
            let state = classify(&e, false, Some(&st), t).state;
            let a = attention(Some(&st), None, Some(&e), Some(state), t);
            assert_eq!(a.standing, Some(StandingKind::Settled), "at {t}ms");
            assert!(!a.needs_attention);
        }
    }

    #[test]
    fn a_snooze_lapses_on_its_own_clock() {
        let e = update(None, true, 0);
        let st = said(StandingKind::Snoozed { until_ms: 60_000 }, 0);
        assert_eq!(
            standing_now(Some(&st), Some(&e), true, 59_999),
            Some(StandingKind::Snoozed { until_ms: 60_000 })
        );
        assert_eq!(standing_now(Some(&st), Some(&e), true, 60_000), None, "wake time reached");
    }

    /// Snoozing is never a way to miss something: new output that has gone
    /// quiet raises the run's hand well before the wake time.
    #[test]
    fn new_output_wakes_a_snooze_early() {
        let mut e = update(None, true, 0);
        let st = said(StandingKind::Snoozed { until_ms: 60 * 60 * 1000 }, 1_000);
        e = update(Some(e), true, 5_000);
        let back = 5_000 + WORKING_TTL_MS;
        assert_eq!(standing_now(Some(&st), Some(&e), true, back), None, "raised its hand early");
    }

    /// "Active" is the opposite instruction, so nothing the agent does takes
    /// it back — only the user can.
    #[test]
    fn active_is_never_consumed_and_never_suppresses() {
        let mut e = update(None, true, 0);
        let st = said(StandingKind::Active, 0);
        e = update(Some(e), true, 5_000);
        let back = 5_000 + WORKING_TTL_MS;
        let a = attention(Some(&st), None, Some(&e), Some(ActivityState::Waiting), back);
        assert_eq!(a.standing, Some(StandingKind::Active));
        assert!(a.needs_attention, "active runs stay on the list");
    }

    /// A run the notifier has not observed yet has no state and no output to
    /// consume anything: whatever the user said still stands, and the pin is
    /// reported either way.
    #[test]
    fn an_unobserved_run_keeps_its_standing_and_its_pin() {
        let st = said(StandingKind::Settled, 1_000);
        let a = attention(Some(&st), Some(2.5), None, None, 9_000_000);
        assert_eq!(a.standing, Some(StandingKind::Settled));
        assert_eq!(a.pin_rank, Some(2.5));
        assert!(!a.needs_attention, "no observation, nothing to be waiting on");
    }

    /// A pin is about placement, not lifecycle: it is reported whatever the
    /// run is doing and whatever else the user has said about it.
    #[test]
    fn a_pin_is_untouched_by_activity() {
        let mut e = update(None, true, 0);
        e = update(Some(e), true, 900_000);
        for (state, standing) in [
            (ActivityState::Working, None),
            (ActivityState::Waiting, Some(said(StandingKind::Settled, 0))),
            (ActivityState::Idle, None),
        ] {
            let a = attention(standing.as_ref(), Some(1.0), Some(&e), Some(state), 900_000);
            assert_eq!(a.pin_rank, Some(1.0), "{state:?}");
        }
    }

    #[test]
    fn attention_serializes_camel_case_for_the_ui() {
        let st = said(StandingKind::Snoozed { until_ms: 7_000 }, 1_000);
        let e = update(None, true, 0);
        let a = attention(Some(&st), Some(3.0), Some(&e), Some(ActivityState::Waiting), 2_000);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"{"standing":{"kind":"snoozed","untilMs":7000},"pinRank":3.0,"needsAttention":false}"#
        );
        let bare = attention(None, None, Some(&e), Some(ActivityState::Waiting), 2_000);
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"standing":null,"pinRank":null,"needsAttention":true}"#
        );
    }

    #[test]
    fn info_serializes_camel_case_for_the_ui() {
        let e = update(None, true, 1_234);
        let info = classify(&e, true, None, 1_234 + WORKING_TTL_MS);
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(json, r#"{"state":"waiting","since":1234}"#);
    }
}
