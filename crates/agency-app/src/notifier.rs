use agency_core::term::SessionStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Loop terminal events (complete/stalled). Per-attempt Finished/Idle
    /// toasts are always suppressed for looping runs — these are the signal.
    #[serde(default = "d_true")]
    pub loop_events: bool,
    /// Stay quiet about the run the user is already watching. Scoped to that
    /// one run, not the whole app: every other agent notifies even while
    /// Agency is in the foreground, because an agent in another tab is
    /// precisely the one you can't see finish. The alias keeps settings saved
    /// under the old app-wide name (`onlyWhenUnfocused`) loading.
    #[serde(default = "d_true", alias = "onlyWhenUnfocused")]
    pub only_when_watching: bool,
    #[serde(default = "d_idle")]
    pub idle_secs: u64,
}

fn d_true() -> bool {
    true
}
fn d_idle() -> u64 {
    30
}

impl Default for NotifSettings {
    fn default() -> Self {
        NotifSettings {
            agent_finished: true,
            agent_idle: true,
            run_crashed: true,
            merge_attention: true,
            loop_events: true,
            only_when_watching: true,
            idle_secs: 30,
        }
    }
}

pub struct RunSnapshot {
    pub id: String,
    pub project_id: String,
    pub label: String,
    /// Terminals never notify: a shell exiting isn't an agent finishing, and a
    /// quiet shell isn't an agent finishing a turn.
    pub is_terminal: bool,
    /// Looping runs suppress per-attempt Finished/Idle (ten "agent exited"
    /// toasts are noise; the loop's own complete/stalled events are the
    /// signal). True whenever the run has a loop config — including terminal
    /// loops, so the final attempt's exit edge (which lands on the same tick
    /// as the terminal transition) can't slip through as a duplicate toast.
    /// Run-script crashes still notify.
    pub is_loop: bool,
    pub agent: SessionStatus,
    /// One entry per configured run script, keyed by script name. A project
    /// runs several (a dev server, a release build), and each crashes on its
    /// own — so the edge is detected per script, and the toast can name it.
    pub run_scripts: BTreeMap<String, SessionStatus>,
    pub pane_hash: u64,
    /// True when the user has submitted a turn (pressed Enter) in this run since
    /// the last turn-finished notification. Gates the idle notification so we
    /// only nudge after a turn the user actually started — never for a fresh
    /// agent sitting at its opening prompt, and never more than once per turn.
    pub user_input_pending: bool,
}

#[derive(Clone)]
pub struct RunWatch {
    pub agent: SessionStatus,
    pub run_scripts: BTreeMap<String, SessionStatus>,
    pub pane_hash: u64,
    pub quiet_since_tick: u64,
    pub idle_fired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyKind {
    Finished,
    /// The named run script went from running to a non-zero exit.
    RunCrashed(String),
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
        // Agent finished: running -> exited. Loop attempts exit by design;
        // their edges are reported by the loop driver instead.
        if matches!(p.agent, SessionStatus::Running) && !snap.is_loop {
            if let SessionStatus::Exited { .. } = snap.agent {
                events.push(NotifyKind::Finished);
            }
        }
        // Run script crashed: running -> exited non-zero, per script. A script
        // that has since been renamed or deleted simply has no new status, so
        // it drops out rather than reporting a phantom crash.
        for (name, was) in &p.run_scripts {
            if !matches!(was, SessionStatus::Running) {
                continue;
            }
            if let Some(SessionStatus::Exited { code }) = snap.run_scripts.get(name) {
                if *code != 0 {
                    events.push(NotifyKind::RunCrashed(name.clone()));
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
    if agent_running && !idle_fired && snap.user_input_pending && !snap.is_loop {
        let quiet_ticks = now_tick.saturating_sub(quiet_since_tick);
        if quiet_ticks.saturating_mul(poll_secs) >= idle_secs {
            events.push(NotifyKind::Idle);
            idle_fired = true;
        }
    }

    let watch = RunWatch {
        agent: snap.agent.clone(),
        run_scripts: snap.run_scripts.clone(),
        pane_hash: snap.pane_hash,
        quiet_since_tick,
        idle_fired,
    };
    // Terminals are tracked (so a later promotion to notifying would have
    // history) but never produce events.
    if snap.is_terminal {
        return (watch, Vec::new());
    }
    (watch, events)
}

/// Whether to hold back a notification about `run_id`.
///
/// The only run we stay quiet about is the one the user is watching right now:
/// its pane is on screen in a focused window, so a toast would just repeat
/// what they can already see. Everything else notifies, app focused or not — a
/// run in another tab is exactly the one whose turn ending you'd otherwise
/// miss, and a run left selected while the user is off in another app is not
/// being watched at all.
///
/// `focused` is the window's focus state, `active` the run the UI has open.
pub fn suppressed(
    settings: &NotifSettings,
    focused: bool,
    active: Option<&str>,
    run_id: &str,
) -> bool {
    settings.only_when_watching && focused && active == Some(run_id)
}

/// How long a notification stays worth opening. Long enough to cover reading
/// the banner and clicking it; short enough that a return to the app an hour
/// later doesn't teleport the user to a stale run.
pub const OPEN_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// What is asking to open the run a notification was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenTrigger {
    /// The user clicked the notification itself. macOS tells us (the delegate
    /// hook in `notif_macos`), and it is an unambiguous "take me there".
    Click,
    /// Agency became the focused app again — a guess, not a request.
    Focus,
}

/// Whether a notification's run should be opened now.
///
/// AGE-166: a focus change is not consent. The webview blurs for a native
/// menu, a panel, a banner — all without the user leaving the app — so
/// treating the focus that comes back as "the user clicked the notification"
/// jumped them to another agent on a click anywhere in the app. A click on the
/// notification always opens; regaining focus only opens what was posted while
/// Agency really was in the background, which is the case where returning to
/// the app *is* the user acting on the banner.
pub fn opens_notified_run(
    trigger: OpenTrigger,
    from_background: bool,
    age: std::time::Duration,
) -> bool {
    age < OPEN_TTL && (trigger == OpenTrigger::Click || from_background)
}

/// Notification (title, body) for an event about the run labelled `label`.
pub fn message(kind: &NotifyKind, label: &str) -> (String, String) {
    match kind {
        NotifyKind::Finished => ("Agent exited".to_string(), format!("{label} — done")),
        NotifyKind::RunCrashed(script) => {
            ("Run script crashed".to_string(), format!("{label} — \"{script}\" exited"))
        }
        NotifyKind::Idle => {
            ("Agent finished a turn".to_string(), format!("{label} — ready for you"))
        }
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
    fn snap_input(
        agent: SessionStatus,
        run_script: SessionStatus,
        pane_hash: u64,
        user_input_pending: bool,
    ) -> RunSnapshot {
        RunSnapshot {
            id: "x".into(),
            project_id: "proj".into(),
            label: "claude: fix".into(),
            is_terminal: false,
            is_loop: false,
            agent,
            run_scripts: one(run_script),
            pane_hash,
            user_input_pending,
        }
    }
    /// A one-script project, under the name the tests refer to.
    fn one(status: SessionStatus) -> BTreeMap<String, SessionStatus> {
        BTreeMap::from([("dev".to_string(), status)])
    }
    fn running() -> SessionStatus {
        SessionStatus::Running
    }
    fn exited(code: i32) -> SessionStatus {
        SessionStatus::Exited { code }
    }

    #[test]
    fn settings_default_is_all_on_idle_30() {
        let s = NotifSettings::default();
        assert!(
            s.agent_finished
                && s.agent_idle
                && s.run_crashed
                && s.merge_attention
                && s.only_when_watching
        );
        assert_eq!(s.idle_secs, 30);
    }

    #[test]
    fn empty_json_deserializes_to_defaults() {
        let s: NotifSettings = serde_json::from_str("{}").unwrap();
        assert!(s.agent_finished);
        assert_eq!(s.idle_secs, 30);
    }

    #[test]
    fn settings_saved_under_the_old_focus_key_still_load() {
        let s: NotifSettings =
            serde_json::from_str(r#"{"onlyWhenUnfocused":false,"idleSecs":45}"#).unwrap();
        assert!(!s.only_when_watching, "old app-wide toggle must carry over");
        assert_eq!(s.idle_secs, 45);
    }

    #[test]
    fn only_the_watched_run_is_suppressed() {
        let s = NotifSettings::default();
        let watched = |focused, active: Option<&str>| suppressed(&s, focused, active, "run-a");
        // Focused window, this run open: the user is looking at it.
        assert!(watched(true, Some("run-a")));
        // Focused window, some other run open — the whole point of AGE-14.
        assert!(!watched(true, Some("run-b")));
        // Focused window, no run open (grid view): nothing is being watched.
        assert!(!watched(true, None));
        // App in the background: even the selected run is unwatched.
        assert!(!watched(false, Some("run-a")));
    }

    #[test]
    fn toggle_off_notifies_even_for_the_watched_run() {
        let s = NotifSettings { only_when_watching: false, ..Default::default() };
        assert!(!suppressed(&s, true, Some("run-a"), "run-a"));
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
        assert_eq!(events, vec![NotifyKind::RunCrashed("dev".into())]);

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
            if ev.contains(&NotifyKind::Idle) {
                fired_at = Some(t);
            }
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
        let (mut w, _) =
            step(None, &snap_input(running(), SessionStatus::Gone, 1, false), 0, 2, 30);
        for t in 1..=30 {
            let (nw, ev) =
                step(Some(&w), &snap_input(running(), SessionStatus::Gone, 1, false), t, 2, 30);
            w = nw;
            assert!(!ev.contains(&NotifyKind::Idle), "fired idle without input at tick {t}");
        }
    }

    #[test]
    fn terminals_never_notify() {
        let term = |agent: SessionStatus, hash: u64| RunSnapshot {
            id: "t".into(),
            project_id: "proj".into(),
            label: "terminal".into(),
            is_terminal: true,
            is_loop: false,
            agent,
            run_scripts: one(SessionStatus::Gone),
            pane_hash: hash,
            user_input_pending: true,
        };
        // Exit edge: running -> exited must stay silent for terminals.
        let (w, _) = step(None, &term(running(), 1), 0, 2, 30);
        let (w, ev) = step(Some(&w), &term(exited(0), 1), 1, 2, 30);
        assert!(ev.is_empty(), "terminal exit must not notify");
        // Idle: a long-quiet terminal with input pending must stay silent too.
        let (mut w, _) = step(Some(&w), &term(running(), 2), 2, 2, 30);
        for t in 3..=40 {
            let (nw, ev) = step(Some(&w), &term(running(), 2), t, 2, 30);
            w = nw;
            assert!(ev.is_empty(), "terminal idle must not notify (tick {t})");
        }
    }

    #[test]
    fn looping_runs_suppress_finished_and_idle_but_not_run_crash() {
        let lsnap = |agent: SessionStatus, run_script: SessionStatus, hash: u64| RunSnapshot {
            id: "l".into(),
            project_id: "proj".into(),
            label: "claude: loop".into(),
            is_terminal: false,
            is_loop: true,
            agent,
            run_scripts: one(run_script),
            pane_hash: hash,
            user_input_pending: true,
        };
        // Attempt exit (running -> exited) must not toast.
        let (w, _) = step(None, &lsnap(running(), running(), 1), 0, 2, 30);
        let (w, ev) = step(Some(&w), &lsnap(exited(0), running(), 1), 1, 2, 30);
        assert!(ev.is_empty(), "loop attempt exit must not notify");
        // Long-quiet loop must not fire idle.
        let (mut w, _) = step(Some(&w), &lsnap(running(), running(), 2), 2, 2, 30);
        for t in 3..=40 {
            let (nw, ev) = step(Some(&w), &lsnap(running(), running(), 2), t, 2, 30);
            w = nw;
            assert!(!ev.contains(&NotifyKind::Idle), "loop idle must not notify (tick {t})");
        }
        // A crashing run script still notifies.
        let (_w, ev) = step(Some(&w), &lsnap(running(), exited(1), 2), 41, 2, 30);
        assert_eq!(ev, vec![NotifyKind::RunCrashed("dev".into())]);
    }

    #[test]
    fn exited_agent_does_not_go_idle() {
        let (p, _) = step(None, &snap(exited(0), SessionStatus::Gone, 1), 0, 2, 30);
        let (_w, ev) = step(Some(&p), &snap(exited(0), SessionStatus::Gone, 1), 100, 2, 30);
        assert!(!ev.contains(&NotifyKind::Idle));
    }

    #[test]
    fn each_script_crashes_on_its_own() {
        let two = |dev: SessionStatus, build: SessionStatus| RunSnapshot {
            id: "x".into(),
            project_id: "proj".into(),
            label: "claude: fix".into(),
            is_terminal: false,
            is_loop: false,
            agent: running(),
            run_scripts: BTreeMap::from([("dev".into(), dev), ("build".into(), build)]),
            pane_hash: 1,
            user_input_pending: true,
        };
        let (w, _) = step(None, &two(running(), running()), 0, 2, 30);
        // The build fails while the dev server keeps serving: one toast, named.
        let (w, ev) = step(Some(&w), &two(running(), exited(2)), 1, 2, 30);
        assert_eq!(ev, vec![NotifyKind::RunCrashed("build".into())]);
        // Still exited on the next tick — the edge already fired, so it stays quiet.
        let (w, ev) = step(Some(&w), &two(running(), exited(2)), 2, 2, 30);
        assert!(ev.is_empty());
        // A script deleted from the config drops out instead of crashing.
        let gone = RunSnapshot {
            run_scripts: BTreeMap::from([("dev".into(), running())]),
            ..two(running(), running())
        };
        let (_w, ev) = step(Some(&w), &gone, 3, 2, 30);
        assert!(ev.is_empty());
    }

    #[test]
    fn message_names_the_script_that_crashed() {
        let (_title, body) = message(&NotifyKind::RunCrashed("build mac".into()), "claude: fix");
        assert!(body.contains("build mac"));
    }

    #[test]
    fn message_uses_label() {
        let (title, body) = message(&NotifyKind::Finished, "claude: fix login");
        assert!(!title.is_empty());
        assert!(body.contains("claude: fix login"));
    }

    /// AGE-166: the app jumped to the notified agent on a click anywhere in
    /// the window. The click landed on a webview that had blurred while the
    /// user stayed in the app, and the focus coming back was read as them
    /// answering the banner.
    #[test]
    fn regaining_focus_only_opens_what_was_notified_from_the_background() {
        let fresh = std::time::Duration::from_secs(1);
        assert!(!opens_notified_run(OpenTrigger::Focus, false, fresh), "the AGE-166 jump");
        assert!(opens_notified_run(OpenTrigger::Focus, true, fresh), "a real return from away");
        // A click is the user pointing at the run, in front or not.
        assert!(opens_notified_run(OpenTrigger::Click, false, fresh));
        assert!(opens_notified_run(OpenTrigger::Click, true, fresh));
    }

    #[test]
    fn a_stale_notification_opens_nothing() {
        let stale = OPEN_TTL + std::time::Duration::from_secs(1);
        assert!(!opens_notified_run(OpenTrigger::Click, true, stale));
        assert!(!opens_notified_run(OpenTrigger::Focus, true, stale));
    }
}
