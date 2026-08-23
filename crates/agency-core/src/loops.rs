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
    /// Wall-clock cap across the whole loop, measured from `started_at`.
    /// None = off, and off is the default: a cap the user did not set must
    /// never trip. Checked only where the loop would spawn another attempt,
    /// so it never kills a running attempt mid-flight.
    #[serde(default)]
    pub max_wall_secs: Option<u64>,
    /// Token budget across the whole loop, compared against the transcript
    /// reader's total (the same figure the UI displays). None = off. An agent
    /// whose transcript we cannot read reports no tokens, so this cap never
    /// trips for it — "0 tokens" and "cannot see tokens" are different claims.
    #[serde(default)]
    pub max_tokens: Option<u64>,
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

/// Why a loop stalled. Not cosmetic: "hit the attempt cap", "crashed three
/// times in a row", "ran out of clock" and "ran out of budget" all used to
/// render as one *Stalled*, which told the user nothing about whether to
/// raise a cap, fix a flag, or rewrite the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StallReason {
    /// The check kept failing until `max_attempts` was spent.
    AttemptCap,
    /// The agent exited nonzero repeatedly (see the crash-loop guard).
    CrashLoop,
    /// `max_wall_secs` elapsed before the loop finished.
    WallClock,
    /// The observed token total reached `max_tokens`.
    Budget,
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
    /// Epoch seconds the loop was created; the wall-clock cap measures from
    /// here. Defaults to 0 for states persisted before the caps existed —
    /// harmless, because those configs cannot carry a cap either.
    #[serde(default)]
    pub started_at: i64,
    /// Last token total the driver observed for the run, folded in on every
    /// step so the persisted record and the budget check cannot disagree.
    /// Stays 0 for agents whose transcript we cannot read.
    #[serde(default)]
    pub tokens_used: u64,
    /// Set on every transition into Stalled. None on states stalled before
    /// reasons existed, and on driver-side stalls (attempt spawn failure).
    #[serde(default)]
    pub stall_reason: Option<StallReason>,
    pub updated_at: i64,
}

impl LoopState {
    pub fn new(now: i64) -> LoopState {
        LoopState {
            status: LoopStatus::AwaitingAgent,
            attempt: 1,
            consecutive_failures: 0,
            last_check_exit: None,
            started_at: now,
            tokens_used: 0,
            stall_reason: None,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows written before the caps existed lack every new field; they must
    /// load with the caps off and the counters zeroed, or the migration adds
    /// a guard the user never asked for.
    #[test]
    fn json_from_before_the_caps_still_loads() {
        let cfg: LoopConfig =
            serde_json::from_str(r#"{"checkCommand":"cargo test","maxAttempts":5}"#).unwrap();
        assert_eq!(cfg.max_wall_secs, None);
        assert_eq!(cfg.max_tokens, None);
        let st: LoopState = serde_json::from_str(
            r#"{"status":"stalled","attempt":5,"consecutiveFailures":0,"lastCheckExit":1,"updatedAt":9}"#,
        )
        .unwrap();
        assert_eq!(st.started_at, 0);
        assert_eq!(st.tokens_used, 0);
        assert_eq!(st.stall_reason, None);
    }

    /// The reason names are a wire contract with the frontend (api.ts
    /// StallReason), so pin the serde spelling.
    #[test]
    fn stall_reasons_serialize_camel_case() {
        let mut st = LoopState::new(0);
        st.stall_reason = Some(StallReason::WallClock);
        let json = serde_json::to_string(&st).unwrap();
        assert!(json.contains(r#""stallReason":"wallClock""#), "{json}");
        assert!(json.contains(r#""tokensUsed":0"#), "{json}");
        assert!(json.contains(r#""startedAt":0"#), "{json}");
    }
}
