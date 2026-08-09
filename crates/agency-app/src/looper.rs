use agency_core::loops::{LoopConfig, LoopState, LoopStatus};
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
                    } else {
                        st.attempt += 1;
                        actions.push(LoopAction::SpawnAttempt);
                    }
                } else {
                    st.status = LoopStatus::Checking;
                    actions.push(LoopAction::StartCheck);
                }
            }
            SessionStatus::Exited { .. } => {
                st.consecutive_failures += 1;
                if st.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    st.status = LoopStatus::Stalled;
                    actions.push(LoopAction::NotifyStalled);
                } else {
                    // A crashed attempt can't have finished the work: respawn
                    // without checking and without consuming an attempt.
                    actions.push(LoopAction::SpawnAttempt);
                }
            }
            // App/daemon restarted under a live loop: relaunch the attempt,
            // counter unchanged.
            SessionStatus::Gone => actions.push(LoopAction::SpawnAttempt),
        },
        LoopStatus::Checking => {
            if let Some(check) = &snap.check {
                st.last_check_exit = check.exit_code;
                if check.passed() {
                    st.status = LoopStatus::Complete;
                    actions.push(LoopAction::NotifyComplete);
                } else if st.attempt >= cfg.max_attempts {
                    st.status = LoopStatus::Stalled;
                    actions.push(LoopAction::NotifyStalled);
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
        LoopConfig { check_command: check.to_string(), max_attempts: max, check_timeout_secs: 600 }
    }

    fn snap(agent: SessionStatus) -> LoopSnapshot {
        LoopSnapshot { agent, check_in_flight: false, check: None }
    }

    fn snap_check(exit: Option<i32>) -> LoopSnapshot {
        LoopSnapshot {
            agent: SessionStatus::Exited { code: 0 },
            check_in_flight: false,
            check: Some(CheckResult { exit_code: exit }),
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
}
