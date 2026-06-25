use agency_core::tmux::SessionStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifSettings {
    #[serde(default = "d_true")]
    pub agent_finished: bool,
    #[serde(default = "d_true")]
    pub agent_idle: bool,
    #[serde(default = "d_true")]
    pub run_crashed: bool,
    #[serde(default = "d_true")]
    pub merge_attention: bool,
    #[serde(default = "d_true")]
    pub only_when_unfocused: bool,
    #[serde(default = "d_idle")]
    pub idle_secs: u64,
}

fn d_true() -> bool { true }
fn d_idle() -> u64 { 30 }

impl Default for NotifSettings {
    fn default() -> Self {
        NotifSettings {
            agent_finished: true,
            agent_idle: true,
            run_crashed: true,
            merge_attention: true,
            only_when_unfocused: true,
            idle_secs: 30,
        }
    }
}

pub struct RunSnapshot {
    pub id: String,
    pub label: String,
    pub agent: SessionStatus,
    pub run_script: SessionStatus,
    pub pane_hash: u64,
    /// True when the user has sent input to this run since the last "waiting for
    /// input" notification. Gates the idle notification so we only nudge after a
    /// turn the user actually started — never for a fresh agent sitting at its
    /// opening prompt, and never more than once per turn.
    pub user_input_pending: bool,
}

#[derive(Clone)]
pub struct RunWatch {
    pub agent: SessionStatus,
    pub run_script: SessionStatus,
    pub pane_hash: u64,
    pub quiet_since_tick: u64,
    pub idle_fired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyKind {
    Finished,
    RunCrashed,
    Idle,
}

/// Pure edge detector. Given the previous watch state (if any) and a fresh
/// snapshot at `now_tick`, return the updated watch and any notification events.
pub fn step(
    prev: Option<&RunWatch>,
    snap: &RunSnapshot,
    now_tick: u64,
    poll_secs: u64,
    idle_secs: u64,
) -> (RunWatch, Vec<NotifyKind>) {
    let mut events = Vec::new();

    // Pane-change tracking for idle.
    let pane_changed = prev.map(|p| p.pane_hash != snap.pane_hash).unwrap_or(true);
    let quiet_since_tick = if pane_changed { now_tick } else { prev.unwrap().quiet_since_tick };

    if let Some(p) = prev {
        // Agent finished: running -> exited.
        if matches!(p.agent, SessionStatus::Running) {
            if let SessionStatus::Exited { .. } = snap.agent {
                events.push(NotifyKind::Finished);
            }
        }
        // Run script crashed: running -> exited non-zero.
        if matches!(p.run_script, SessionStatus::Running) {
            if let SessionStatus::Exited { code } = snap.run_script {
                if code != 0 {
                    events.push(NotifyKind::RunCrashed);
                }
            }
        }
    }

    // Idle latch.
    let agent_running = matches!(snap.agent, SessionStatus::Running);
    let mut idle_fired = prev.map(|p| p.idle_fired).unwrap_or(false);
    if !agent_running || pane_changed {
        idle_fired = false;
    }
    // Idle only fires once the user has given this run input — a fresh agent
    // sitting at its opening prompt has `user_input_pending == false`, so it is
    // never flagged as "waiting for input". The flag is cleared by the caller
    // when the notification fires, so each turn nudges at most once.
    if agent_running && !idle_fired && snap.user_input_pending {
        let quiet_ticks = now_tick.saturating_sub(quiet_since_tick);
        if quiet_ticks.saturating_mul(poll_secs) >= idle_secs {
            events.push(NotifyKind::Idle);
            idle_fired = true;
        }
    }

    let watch = RunWatch {
        agent: snap.agent.clone(),
        run_script: snap.run_script.clone(),
        pane_hash: snap.pane_hash,
        quiet_since_tick,
        idle_fired,
    };
    (watch, events)
}

/// Notification (title, body) for an event about the run labelled `label`.
pub fn message(kind: &NotifyKind, label: &str) -> (String, String) {
    match kind {
        NotifyKind::Finished => ("Agent finished".to_string(), format!("{label} — done")),
        NotifyKind::RunCrashed => ("Run script crashed".to_string(), format!("{label} — dev server exited")),
        NotifyKind::Idle => ("Agent needs you".to_string(), format!("{label} — waiting for input")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(agent: SessionStatus, run_script: SessionStatus, pane_hash: u64) -> RunSnapshot {
        // Default to "input given" so the idle-path tests exercise the timing;
        // the gating itself is covered by `idle_requires_user_input`.
        snap_input(agent, run_script, pane_hash, true)
    }
    fn snap_input(agent: SessionStatus, run_script: SessionStatus, pane_hash: u64, user_input_pending: bool) -> RunSnapshot {
        RunSnapshot { id: "x".into(), label: "claude: fix".into(), agent, run_script, pane_hash, user_input_pending }
    }
    fn running() -> SessionStatus { SessionStatus::Running }
    fn exited(code: i32) -> SessionStatus { SessionStatus::Exited { code } }

    #[test]
    fn settings_default_is_all_on_idle_30() {
        let s = NotifSettings::default();
        assert!(s.agent_finished && s.agent_idle && s.run_crashed && s.merge_attention && s.only_when_unfocused);
        assert_eq!(s.idle_secs, 30);
    }

    #[test]
    fn empty_json_deserializes_to_defaults() {
        let s: NotifSettings = serde_json::from_str("{}").unwrap();
        assert!(s.agent_finished);
        assert_eq!(s.idle_secs, 30);
    }

    #[test]
    fn first_observation_emits_nothing() {
        let (_w, events) = step(None, &snap(running(), SessionStatus::Gone, 1), 0, 2, 30);
        assert!(events.is_empty());
    }

    #[test]
    fn agent_running_to_exited_emits_finished() {
        let (prev, _) = step(None, &snap(running(), SessionStatus::Gone, 1), 0, 2, 30);
        let (_w, events) = step(Some(&prev), &snap(exited(0), SessionStatus::Gone, 1), 1, 2, 30);
        assert_eq!(events, vec![NotifyKind::Finished]);
    }

    #[test]
    fn run_script_nonzero_exit_emits_crash_but_zero_does_not() {
        let (p1, _) = step(None, &snap(running(), running(), 1), 0, 2, 30);
        let (_w, events) = step(Some(&p1), &snap(running(), exited(1), 1), 1, 2, 30);
        assert_eq!(events, vec![NotifyKind::RunCrashed]);

        let (p2, _) = step(None, &snap(running(), running(), 1), 0, 2, 30);
        let (_w2, events2) = step(Some(&p2), &snap(running(), exited(0), 1), 1, 2, 30);
        assert!(events2.is_empty());
    }

    #[test]
    fn idle_fires_once_after_threshold_then_resets_on_activity() {
        // tick 0: first obs, pane_hash 1, quiet_since 0
        let (mut w, _) = step(None, &snap(running(), SessionStatus::Gone, 1), 0, 2, 30);
        // ticks 1..=14 same pane → at tick 15, 15*2=30 >= 30 → Idle
        let mut fired_at = None;
        for t in 1..=15 {
            let (nw, ev) = step(Some(&w), &snap(running(), SessionStatus::Gone, 1), t, 2, 30);
            w = nw;
            if ev.contains(&NotifyKind::Idle) { fired_at = Some(t); }
        }
        assert_eq!(fired_at, Some(15));
        // already idle → no repeat next tick
        let (w2, ev2) = step(Some(&w), &snap(running(), SessionStatus::Gone, 1), 16, 2, 30);
        assert!(ev2.is_empty());
        // pane changes → idle latch clears, quiet resets
        let (w3, ev3) = step(Some(&w2), &snap(running(), SessionStatus::Gone, 999), 17, 2, 30);
        assert!(ev3.is_empty());
        assert!(!w3.idle_fired);
        assert_eq!(w3.quiet_since_tick, 17);
    }

    #[test]
    fn idle_requires_user_input() {
        // A running agent that has never received input (fresh prompt) must not
        // fire idle no matter how long its pane stays quiet.
        let (mut w, _) = step(None, &snap_input(running(), SessionStatus::Gone, 1, false), 0, 2, 30);
        for t in 1..=30 {
            let (nw, ev) = step(Some(&w), &snap_input(running(), SessionStatus::Gone, 1, false), t, 2, 30);
            w = nw;
            assert!(!ev.contains(&NotifyKind::Idle), "fired idle without input at tick {t}");
        }
    }

    #[test]
    fn exited_agent_does_not_go_idle() {
        let (p, _) = step(None, &snap(exited(0), SessionStatus::Gone, 1), 0, 2, 30);
        let (_w, ev) = step(Some(&p), &snap(exited(0), SessionStatus::Gone, 1), 100, 2, 30);
        assert!(!ev.contains(&NotifyKind::Idle));
    }

    #[test]
    fn message_uses_label() {
        let (title, body) = message(&NotifyKind::Finished, "claude: fix login");
        assert!(!title.is_empty());
        assert!(body.contains("claude: fix login"));
    }
}
