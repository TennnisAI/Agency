use serde::{Deserialize, Serialize};

/// Immutable recipe for a looping run, written once at creation. The loop
/// re-invokes the agent headless with the run's prompt until `check_command`
/// exits 0 or `max_attempts` is spent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopConfig {
    /// Run in the worktree via the user's shell between attempts; exit 0 ends
    /// the loop as complete. Empty = no verifier: run exactly `max_attempts`
    /// attempts ("fixed iterations" mode).
    pub check_command: String,
    pub max_attempts: u32,
    #[serde(default = "d_check_timeout")]
    pub check_timeout_secs: u64,
}

fn d_check_timeout() -> u64 {
    600
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoopStatus {
    /// A headless agent attempt is (or should be) running.
    AwaitingAgent,
    /// The check command is running in the worktree.
    Checking,
    /// Check passed (or fixed iterations finished) — terminal.
    Complete,
    /// Attempt cap hit or repeated agent crashes — terminal.
    Stalled,
    /// User pressed Stop — terminal.
    Stopped,
}

impl LoopStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, LoopStatus::Complete | LoopStatus::Stalled | LoopStatus::Stopped)
    }
}

/// Mutable progress of a looping run, persisted on every transition so a
/// restarted app resumes the loop instead of orphaning it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopState {
    pub status: LoopStatus,
    /// 1-based attempt counter (current attempt while running, last attempt
    /// once terminal).
    pub attempt: u32,
    /// Consecutive nonzero agent exits; crash-loop guard stalls at 3 so a bad
    /// flag or auth failure doesn't burn the whole attempt budget.
    pub consecutive_failures: u32,
    pub last_check_exit: Option<i32>,
    pub updated_at: i64,
}

impl LoopState {
    pub fn new(now: i64) -> LoopState {
        LoopState {
            status: LoopStatus::AwaitingAgent,
            attempt: 1,
            consecutive_failures: 0,
            last_check_exit: None,
            updated_at: now,
        }
    }
}
