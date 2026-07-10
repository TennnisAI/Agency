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

/// How long the pane may stay unchanged before a run stops counting as
/// working. Agents redraw constantly while working (spinners, streaming
/// output), so anything quieter than this is sitting at a prompt. Comfortably
/// above the 2s poll so a single slow tick can't flap the state.
pub const WORKING_TTL_MS: i64 = 10_000;

/// How long a quiet run stays "waiting" before decaying to plain idle. Past
/// this the user has clearly deprioritized the run — an urgent badge and a
/// climbing timer stop being signal.
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
pub fn classify(entry: &ActivityEntry, turn_driven: bool, now_ms: i64) -> ActivityInfo {
    let quiet_ms = now_ms - entry.last_change_ms;
    if quiet_ms < WORKING_TTL_MS {
        ActivityInfo { state: ActivityState::Working, since: entry.busy_since_ms }
    } else if turn_driven && quiet_ms < WAITING_MAX_MS {
        ActivityInfo { state: ActivityState::Waiting, since: entry.last_change_ms }
    } else {
        ActivityInfo { state: ActivityState::Idle, since: entry.last_change_ms }
    }
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
        assert_eq!(state(&e, true, quiet), ActivityState::Waiting, "turn driven: waiting");
        assert_eq!(state(&e, false, quiet), ActivityState::Idle, "never prompted: just idle");
        assert_eq!(classify(&e, true, quiet).since, 2_000, "waiting since = last pane change");
    }

    #[test]
    fn waiting_decays_to_idle_after_the_cap() {
        let mut e = update(None, true, 0);
        let just_under = WAITING_MAX_MS - 1;
        e = update(Some(e), false, just_under);
        assert_eq!(state(&e, true, just_under), ActivityState::Waiting);
        assert_eq!(state(&e, true, WAITING_MAX_MS), ActivityState::Idle, "cap reached: idle");
        assert_eq!(classify(&e, true, WAITING_MAX_MS).since, 0);
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

    #[test]
    fn info_serializes_camel_case_for_the_ui() {
        let e = update(None, true, 1_234);
        let info = classify(&e, true, 1_234 + WORKING_TTL_MS);
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(json, r#"{"state":"waiting","since":1234}"#);
    }
}
