//! The user's own word on a run: settled, active, snoozed — and pinned.
//!
//! `agency_app::activity` derives working/waiting/idle from what a pane does.
//! That derivation has to guess at one thing it cannot see: whether a run that
//! went quiet still matters to the person. It guessed with a clock, decaying a
//! waiting run to idle after 30 minutes. What is stored here is the record
//! that replaces the guess wherever the user has actually spoken.
//!
//! Deliberately kept beside the derived state rather than folded into it: "I
//! have dealt with this" is a fact about the person, and mixing it into a
//! state derived from pane output would lose the difference between the two.
//! The rules that combine them live in `agency_app::activity`, which is where
//! the pane bookkeeping is.

use serde::{Deserialize, Serialize};

/// What the user last said about a run, and the moment they said it.
///
/// `at_ms` is load-bearing: output *after* it is output the user has not seen,
/// which is what consumes a settle and wakes a snooze early. Without it,
/// "settled" would either be permanent or would have to be cleared by hand.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Standing {
    #[serde(flatten)]
    pub kind: StandingKind,
    /// Epoch ms.
    pub at_ms: i64,
}

impl Standing {
    pub fn new(kind: StandingKind, at_ms: i64) -> Standing {
        Standing { kind, at_ms }
    }
}

/// The three things a person can say about a run.
///
/// Serialized internally tagged (`{"kind":"snoozed","untilMs":…}`) so the same
/// shape crosses the IPC boundary, is stored in the run row, and reads as a
/// discriminated union in TypeScript. Serde rejects any other tag, which is
/// what keeps the command that accepts one of these default-deny.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StandingKind {
    /// "I have dealt with this." Suppressed from the attention list until the
    /// run comes back asking, which un-settles it.
    Settled,
    /// "This one still needs me." Nothing about it is a guess any more, so the
    /// time decay stops applying and the run keeps its badge until the user
    /// says otherwise.
    Active,
    /// "Not now." Suppressed until `until_ms` — or until the agent produces
    /// something new, which makes the run raise its hand early. Snoozing is
    /// never a way to miss something.
    Snoozed {
        /// Epoch ms.
        #[serde(rename = "untilMs")]
        until_ms: i64,
    },
}

impl StandingKind {
    /// Whether this standing keeps the run off the attention list. Only the
    /// two "not now" answers do; `Active` is the opposite instruction.
    pub fn suppresses(self) -> bool {
        matches!(self, StandingKind::Settled | StandingKind::Snoozed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snooze_round_trips_through_one_flat_object() {
        let s = Standing::new(StandingKind::Snoozed { until_ms: 1_700 }, 1_000);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"snoozed","untilMs":1700,"atMs":1000}"#);
        assert_eq!(serde_json::from_str::<Standing>(&json).unwrap(), s);
    }

    #[test]
    fn a_settle_round_trips_without_a_wake_time() {
        let s = Standing::new(StandingKind::Settled, 42);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"settled","atMs":42}"#);
        assert_eq!(serde_json::from_str::<Standing>(&json).unwrap(), s);
    }

    /// The UI sends a bare kind and the backend stamps the clock, so the kind
    /// has to deserialize on its own too.
    #[test]
    fn a_bare_kind_deserializes_and_unknown_ones_are_refused() {
        assert_eq!(
            serde_json::from_str::<StandingKind>(r#"{"kind":"active"}"#).unwrap(),
            StandingKind::Active
        );
        assert!(serde_json::from_str::<StandingKind>(r#"{"kind":"muted"}"#).is_err());
    }

    #[test]
    fn only_the_not_now_answers_suppress() {
        assert!(StandingKind::Settled.suppresses());
        assert!(StandingKind::Snoozed { until_ms: 1 }.suppresses());
        assert!(!StandingKind::Active.suppresses());
    }
}
