use agency_core::loops::{LoopConfig, LoopState, LoopStatus, StallReason};
use agency_core::term::SessionStatus;

/// Consecutive nonzero agent exits before the loop stalls. Catches bad CLI
/// flags, auth failures and rate-limit storms without burning the attempt
/// budget on them (crashed attempts don't increment the attempt counter).
pub const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Result of a finished check command. `exit_code == None` means the check
/// was killed on timeout; both timeout and nonzero count as "not done yet".
#[derive(Debug, Clone, Copy)]
pub struct CheckResult {
    pub exit_code: Option<i32>,
}

impl CheckResult {
    pub fn passed(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// What the poller observed about one looping run this tick.
pub struct LoopSnapshot {
    /// Status of the run's agent session (the headless attempt).
    pub agent: SessionStatus,
    /// True while a check command spawned by the driver is still running.
    pub check_in_flight: bool,
    /// A check result drained this tick, if one just finished.
    pub check: Option<CheckResult>,
    /// Total tokens the run has burned, read from the agent's own transcript
    /// (see usage.rs). None when the agent's format is unreadable — never 0,
    /// because a token cap must not trip on a count we cannot see.
    pub tokens: Option<u64>,
}

/// Side effects the driver must perform after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopAction {
    /// Kill any leftover session and start a fresh headless agent attempt.
    SpawnAttempt,
    /// Run the check command in the run's worktree.
    StartCheck,
    NotifyComplete,
    NotifyStalled,
}

/// The first configured cap the loop has exceeded, if any.
///
/// Hard stops are off by default: an unconfigured cap (None) never trips, so
/// a loop can only die for a reason the user chose. Consulted only where the
/// loop would otherwise spawn another attempt — a running attempt is never
/// killed mid-flight, and a finished attempt still gets its check (a passing
/// check completes the loop even over-cap) — so the worst a cap can do is
/// turn "spawn one more attempt" into a stall that names itself.
pub fn exceeded_cap(cfg: &LoopConfig, st: &LoopState, now: i64) -> Option<StallReason> {
    // max(0): a clock stepped backwards must read as "no time elapsed", not
    // as a huge unsigned elapsed that trips the cap.
    let elapsed = now.saturating_sub(st.started_at).max(0) as u64;
    if cfg.max_wall_secs.is_some_and(|cap| elapsed >= cap) {
        return Some(StallReason::WallClock);
    }
    if cfg.max_tokens.is_some_and(|cap| st.tokens_used >= cap) {
        return Some(StallReason::Budget);
    }
    None
}

fn stall(st: &mut LoopState, actions: &mut Vec<LoopAction>, reason: StallReason) {
    st.status = LoopStatus::Stalled;
    st.stall_reason = Some(reason);
    actions.push(LoopAction::NotifyStalled);
}

/// Pure transition function, mirroring `notifier::step`: given the loop's
/// persisted state and a fresh snapshot, return the next state and the side
/// effects to perform. The driver persists the returned state before acting,
/// so a crash between persist and act is recovered by the next tick (an
/// AwaitingAgent state with a Gone session respawns; a Checking state with
/// nothing in flight re-checks).
pub fn step(
    cfg: &LoopConfig,
    prev: &LoopState,
    snap: &LoopSnapshot,
    now: i64,
) -> (LoopState, Vec<LoopAction>) {
    let mut st = prev.clone();
    let mut actions = Vec::new();
    if st.status.is_terminal() {
        return (st, actions);
    }
    // Fold the token observation in before anything is decided, so the budget
    // check below and the persisted figure cannot disagree. None (an agent
    // whose transcript we cannot read) leaves the last value standing.
    if let Some(tokens) = snap.tokens {
        st.tokens_used = tokens;
    }

    match st.status {
        LoopStatus::AwaitingAgent => match snap.agent {
            SessionStatus::Running => {}
            SessionStatus::Exited { code: 0 } => {
                st.consecutive_failures = 0;
                if cfg.check_command.trim().is_empty() {
                    // No verifier: fixed-iterations mode — run exactly
                    // max_attempts attempts, then finish.
                    if st.attempt >= cfg.max_attempts {
                        st.status = LoopStatus::Complete;
                        actions.push(LoopAction::NotifyComplete);
                    } else if let Some(reason) = exceeded_cap(cfg, &st, now) {
                        stall(&mut st, &mut actions, reason);
                    } else {
                        st.attempt += 1;
                        actions.push(LoopAction::SpawnAttempt);
                    }
                } else {
                    // Deliberately not cap-gated: the attempt that just
                    // finished may have done the job, and a passing check
                    // completes the loop even over-cap. Caps only gate
                    // spawning the next attempt.
                    st.status = LoopStatus::Checking;
                    actions.push(LoopAction::StartCheck);
                }
            }
            SessionStatus::Exited { .. } => {
                st.consecutive_failures += 1;
                if st.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    stall(&mut st, &mut actions, StallReason::CrashLoop);
                } else if let Some(reason) = exceeded_cap(cfg, &st, now) {
                    stall(&mut st, &mut actions, reason);
                } else {
                    // A crashed attempt can't have finished the work: respawn
                    // without checking and without consuming an attempt.
                    actions.push(LoopAction::SpawnAttempt);
                }
            }
            // App/daemon restarted under a live loop: relaunch the attempt,
            // counter unchanged. Cap-gated like every other respawn — wall
            // clock keeps counting while the app is closed, and resuming past
            // the cap would overrun it by however long the next attempt runs.
            SessionStatus::Gone => {
                if let Some(reason) = exceeded_cap(cfg, &st, now) {
                    stall(&mut st, &mut actions, reason);
                } else {
                    actions.push(LoopAction::SpawnAttempt);
                }
            }
        },
        LoopStatus::Checking => {
            if let Some(check) = &snap.check {
                st.last_check_exit = check.exit_code;
                if check.passed() {
                    st.status = LoopStatus::Complete;
                    actions.push(LoopAction::NotifyComplete);
                } else if st.attempt >= cfg.max_attempts {
                    stall(&mut st, &mut actions, StallReason::AttemptCap);
                } else if let Some(reason) = exceeded_cap(cfg, &st, now) {
                    stall(&mut st, &mut actions, reason);
                } else {
                    st.attempt += 1;
                    st.status = LoopStatus::AwaitingAgent;
                    actions.push(LoopAction::SpawnAttempt);
                }
            } else if !snap.check_in_flight {
                // Checking persisted but no check running (the check thread
                // died with the app): start it again.
                actions.push(LoopAction::StartCheck);
            }
        }
        LoopStatus::Complete | LoopStatus::Stalled | LoopStatus::Stopped => unreachable!(),
    }

    if st != *prev {
        st.updated_at = now;
    }
    (st, actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(check: &str, max: u32) -> LoopConfig {
        LoopConfig {
            check_command: check.to_string(),
            max_attempts: max,
            check_timeout_secs: 600,
            max_wall_secs: None,
            max_tokens: None,
        }
    }

    fn cfg_caps(check: &str, wall: Option<u64>, tokens: Option<u64>) -> LoopConfig {
        LoopConfig { max_wall_secs: wall, max_tokens: tokens, ..cfg(check, 5) }
    }

    fn snap(agent: SessionStatus) -> LoopSnapshot {
        LoopSnapshot { agent, check_in_flight: false, check: None, tokens: None }
    }

    fn snap_check(exit: Option<i32>) -> LoopSnapshot {
        LoopSnapshot {
            agent: SessionStatus::Exited { code: 0 },
            check_in_flight: false,
            check: Some(CheckResult { exit_code: exit }),
            tokens: None,
        }
    }

    fn running() -> SessionStatus {
        SessionStatus::Running
    }
    fn exited(code: i32) -> SessionStatus {
        SessionStatus::Exited { code }
    }

    #[test]
    fn running_agent_is_a_no_op() {
        let st = LoopState::new(0);
        let (next, actions) = step(&cfg("true", 5), &st, &snap(running()), 1);
        assert_eq!(next, st);
        assert!(actions.is_empty());
    }

    #[test]
    fn clean_exit_starts_check() {
        let st = LoopState::new(0);
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap(exited(0)), 1);
        assert_eq!(next.status, LoopStatus::Checking);
        assert_eq!(next.attempt, 1);
        assert_eq!(actions, vec![LoopAction::StartCheck]);
        assert_eq!(next.updated_at, 1);
    }

    #[test]
    fn passing_check_completes() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap_check(Some(0)), 2);
        assert_eq!(next.status, LoopStatus::Complete);
        assert_eq!(actions, vec![LoopAction::NotifyComplete]);
    }

    #[test]
    fn failing_check_respawns_and_increments_attempt() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap_check(Some(1)), 2);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(next.attempt, 2);
        assert_eq!(next.last_check_exit, Some(1));
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn check_timeout_counts_as_failure() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap_check(None), 2);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(next.attempt, 2);
        assert_eq!(next.last_check_exit, None);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn failing_check_at_attempt_cap_stalls() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        st.attempt = 5;
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap_check(Some(1)), 2);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.attempt, 5);
        assert_eq!(next.stall_reason, Some(StallReason::AttemptCap));
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    #[test]
    fn agent_crash_respawns_without_consuming_attempt() {
        let st = LoopState::new(0);
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap(exited(1)), 1);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(next.attempt, 1);
        assert_eq!(next.consecutive_failures, 1);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn three_consecutive_crashes_stall() {
        let mut st = LoopState::new(0);
        for i in 0..2 {
            let (next, actions) = step(&cfg("cargo test", 5), &st, &snap(exited(1)), i);
            assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
            st = next;
        }
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap(exited(1)), 3);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.consecutive_failures, 3);
        assert_eq!(next.stall_reason, Some(StallReason::CrashLoop));
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    #[test]
    fn clean_exit_resets_crash_counter() {
        let mut st = LoopState::new(0);
        st.consecutive_failures = 2;
        let (next, _) = step(&cfg("cargo test", 5), &st, &snap(exited(0)), 1);
        assert_eq!(next.consecutive_failures, 0);
        assert_eq!(next.status, LoopStatus::Checking);
    }

    #[test]
    fn gone_session_respawns_with_attempt_unchanged() {
        let mut st = LoopState::new(0);
        st.attempt = 4;
        let (next, actions) = step(&cfg("cargo test", 5), &st, &snap(SessionStatus::Gone), 1);
        assert_eq!(next.attempt, 4);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn empty_check_runs_fixed_iterations_then_completes() {
        let c = cfg("", 3);
        let mut st = LoopState::new(0);
        // Attempts 1 and 2 finish -> respawn with incremented counter.
        for expected_next in [2, 3] {
            let (next, actions) = step(&c, &st, &snap(exited(0)), 1);
            assert_eq!(next.attempt, expected_next);
            assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
            st = next;
        }
        // Attempt 3 (the cap) finishes -> complete.
        let (next, actions) = step(&c, &st, &snap(exited(0)), 2);
        assert_eq!(next.status, LoopStatus::Complete);
        assert_eq!(actions, vec![LoopAction::NotifyComplete]);
    }

    #[test]
    fn checking_with_nothing_in_flight_restarts_check() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let s = LoopSnapshot {
            agent: SessionStatus::Exited { code: 0 },
            check_in_flight: false,
            check: None,
            tokens: None,
        };
        let (next, actions) = step(&cfg("cargo test", 5), &st, &s, 1);
        assert_eq!(next.status, LoopStatus::Checking);
        assert_eq!(actions, vec![LoopAction::StartCheck]);
    }

    #[test]
    fn checking_with_check_in_flight_waits() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let s = LoopSnapshot {
            agent: SessionStatus::Exited { code: 0 },
            check_in_flight: true,
            check: None,
            tokens: None,
        };
        let (next, actions) = step(&cfg("cargo test", 5), &st, &s, 1);
        assert_eq!(next, st);
        assert!(actions.is_empty());
    }

    #[test]
    fn terminal_states_are_inert() {
        for status in [LoopStatus::Complete, LoopStatus::Stalled, LoopStatus::Stopped] {
            let mut st = LoopState::new(0);
            st.status = status;
            let (next, actions) = step(&cfg("cargo test", 5), &st, &snap(exited(0)), 9);
            assert_eq!(next, st);
            assert!(actions.is_empty(), "{status:?} must not act");
        }
    }

    // ── wall-clock and token caps (AGE-110) ────────────────────────────────

    /// Rule 1 of the design: a cap the user did not set must never trip, no
    /// matter how long the loop has run or how much it has spent.
    #[test]
    fn unconfigured_caps_never_trip() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let mut s = snap_check(Some(1));
        s.tokens = Some(u64::MAX);
        let (next, actions) = step(&cfg("cargo test", 5), &st, &s, 100_000_000);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn wall_cap_stalls_instead_of_respawning_after_a_failed_check() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", Some(3600), None);
        let (next, actions) = step(&c, &st, &snap_check(Some(1)), 3600);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.stall_reason, Some(StallReason::WallClock));
        assert_eq!(next.attempt, 1, "a stall must not consume an attempt");
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    #[test]
    fn under_the_wall_cap_the_loop_respawns() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", Some(3600), None);
        let (next, actions) = step(&c, &st, &snap_check(Some(1)), 3599);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn wall_cap_stalls_fixed_iterations_between_attempts() {
        let c = cfg_caps("", Some(600), None);
        let st = LoopState::new(0);
        let (next, actions) = step(&c, &st, &snap(exited(0)), 600);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.stall_reason, Some(StallReason::WallClock));
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    /// Caps gate spawning, not finishing: work that is done is done, and
    /// reporting it as stalled would be the false positive rule 2 forbids.
    #[test]
    fn passing_check_completes_even_over_every_cap() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", Some(1), Some(1));
        let mut s = snap_check(Some(0));
        s.tokens = Some(1_000_000);
        let (next, actions) = step(&c, &st, &s, 100_000);
        assert_eq!(next.status, LoopStatus::Complete);
        assert_eq!(actions, vec![LoopAction::NotifyComplete]);
    }

    #[test]
    fn fixed_iterations_final_attempt_completes_over_cap() {
        let c = LoopConfig { max_attempts: 3, ..cfg_caps("", Some(1), None) };
        let mut st = LoopState::new(0);
        st.attempt = 3;
        let (next, actions) = step(&c, &st, &snap(exited(0)), 100_000);
        assert_eq!(next.status, LoopStatus::Complete);
        assert_eq!(actions, vec![LoopAction::NotifyComplete]);
    }

    /// An attempt that is still running is never killed by a cap; the cap
    /// takes effect at the next attempt boundary.
    #[test]
    fn a_running_attempt_is_never_killed_by_a_cap() {
        let st = LoopState::new(0);
        let c = cfg_caps("cargo test", Some(60), Some(100));
        let mut s = snap(running());
        s.tokens = Some(1_000_000);
        let (next, actions) = step(&c, &st, &s, 100_000);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert!(actions.is_empty());
    }

    #[test]
    fn token_cap_stalls_on_the_observed_total() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", None, Some(1_000));
        let mut s = snap_check(Some(1));
        s.tokens = Some(1_500);
        let (next, actions) = step(&c, &st, &s, 2);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.stall_reason, Some(StallReason::Budget));
        assert_eq!(next.tokens_used, 1_500, "the stalled record keeps the figure that tripped it");
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    /// An agent whose transcript we cannot read reports None, never 0, and a
    /// cap must not trip on a count we cannot see (see usage.rs on why the
    /// two are different claims).
    #[test]
    fn unreadable_tokens_never_trip_the_token_cap() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", None, Some(1_000));
        let (next, actions) = step(&c, &st, &snap_check(Some(1)), 2);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(next.tokens_used, 0);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    /// Tokens flow into the state on every step, so the persisted record
    /// tracks spend while the attempt runs, not just at boundaries.
    #[test]
    fn token_observations_fold_into_the_state_each_tick() {
        let st = LoopState::new(0);
        let mut s = snap(running());
        s.tokens = Some(42);
        let (next, actions) = step(&cfg("cargo test", 5), &st, &s, 7);
        assert_eq!(next.tokens_used, 42);
        assert_eq!(next.updated_at, 7);
        assert!(actions.is_empty());
    }

    /// A crash respawn is cap-gated too: it does not consume an attempt, so
    /// without the gate a loop over its cap could keep burning through the
    /// crash-retry budget.
    #[test]
    fn crash_respawn_is_gated_by_the_caps() {
        let st = LoopState::new(0);
        let c = cfg_caps("cargo test", Some(60), None);
        let (next, actions) = step(&c, &st, &snap(exited(1)), 60);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.stall_reason, Some(StallReason::WallClock));
        assert_eq!(next.consecutive_failures, 1);
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    /// The crash-loop guard outranks the caps: "the agent keeps failing" is
    /// the more actionable diagnosis when both hold.
    #[test]
    fn crash_loop_reason_wins_over_an_exceeded_cap() {
        let mut st = LoopState::new(0);
        st.consecutive_failures = 2;
        let c = cfg_caps("cargo test", Some(60), None);
        let (next, _) = step(&c, &st, &snap(exited(1)), 60);
        assert_eq!(next.stall_reason, Some(StallReason::CrashLoop));
    }

    /// Same ranking for the attempt cap: it is checked first, so a loop that
    /// spent its attempts while also over the clock names the attempt cap.
    #[test]
    fn attempt_cap_reason_wins_over_an_exceeded_cap() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        st.attempt = 5;
        let c = cfg_caps("cargo test", Some(60), None);
        let (next, _) = step(&c, &st, &snap_check(Some(1)), 60);
        assert_eq!(next.stall_reason, Some(StallReason::AttemptCap));
    }

    /// Restart-respawn (Gone) is cap-gated: the wall clock kept counting
    /// while the app was closed, and resuming past the cap would overrun it
    /// by however long the next attempt runs.
    #[test]
    fn gone_respawn_is_gated_by_the_caps() {
        let st = LoopState::new(0);
        let c = cfg_caps("cargo test", Some(60), None);
        let (next, actions) = step(&c, &st, &snap(SessionStatus::Gone), 61);
        assert_eq!(next.status, LoopStatus::Stalled);
        assert_eq!(next.stall_reason, Some(StallReason::WallClock));
        assert_eq!(actions, vec![LoopAction::NotifyStalled]);
    }

    /// Both caps exceeded at once: wall clock is checked first, and the
    /// order is pinned so the rendered reason cannot flap between ticks.
    #[test]
    fn wall_clock_reason_outranks_budget() {
        let mut st = LoopState::new(0);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", Some(60), Some(100));
        let mut s = snap_check(Some(1));
        s.tokens = Some(200);
        let (next, _) = step(&c, &st, &s, 60);
        assert_eq!(next.stall_reason, Some(StallReason::WallClock));
    }

    /// A clock stepped backwards (started_at in the future) reads as zero
    /// elapsed, not as an enormous unsigned value that trips the cap.
    #[test]
    fn a_backwards_clock_never_trips_the_wall_cap() {
        let mut st = LoopState::new(100);
        st.status = LoopStatus::Checking;
        let c = cfg_caps("cargo test", Some(60), None);
        let (next, actions) = step(&c, &st, &snap_check(Some(1)), 50);
        assert_eq!(next.status, LoopStatus::AwaitingAgent);
        assert_eq!(actions, vec![LoopAction::SpawnAttempt]);
    }

    #[test]
    fn new_state_starts_the_wall_clock_at_creation() {
        assert_eq!(LoopState::new(7).started_at, 7);
    }
}
