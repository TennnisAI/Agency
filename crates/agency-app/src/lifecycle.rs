//! Quit flow: confirm, then either stop every agent and the daemon or leave
//! them running for the next launch to adopt, and exit.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

/// Set to `true` once the user confirms quitting so that the second
/// `RunEvent::ExitRequested` event (triggered by `app.exit(0)`) is not
/// intercepted again, avoiding an infinite dialog loop.
pub static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// Claimed by the first answer to the quit prompt, so that any later one is
/// ignored. `app.exit(0)` only asks the event loop to stop, and the styled
/// dialog stayed clickable meanwhile: a "Stop and quit" landing after "Leave
/// running" killed the agents the first click had kept.
static ANSWERED: AtomicBool = AtomicBool::new(false);

/// What a quit does with the sessions the terminal daemon is hosting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agents {
    /// Kill every session and shut the daemon down.
    Stop,
    /// Leave the daemon and its sessions running; the next launch adopts them
    /// (`AppState::new`'s rehydrate). Closing the window already worked this
    /// way, but quitting did not: twelve agents died to a quit that had nothing
    /// to do with them (AGE-210).
    ///
    /// Adoption is not guaranteed, so the dialogs do not promise it. A next
    /// launch on a release that bumps `PROTOCOL_VERSION`, installed by hand
    /// rather than through the update panel that warns about it, replaces the
    /// daemon and ends its sessions (`TermClient::connect_or_spawn`); they then
    /// resume as they would after a stop.
    Leave,
}

/// Whether an `ExitRequested` should be held for the quit confirmation.
///
/// Not once the user has confirmed, which is the exit `app.exit(0)` itself
/// raises. Not for a restart either: Tauri ignores `prevent_exit` for
/// `RESTART_EXIT_CODE`, so asking would only flash a dialog over an app that is
/// already going, and offer to kill the agents the restart is leaving alone.
///
/// A guard, not a fix for something observed: `restart_app` is a sync command,
/// so it runs on the main thread, where `app.restart()` skips `ExitRequested`
/// entirely, and `lifecycle::restart` sets `QUIT_CONFIRMED` first anyway. A
/// restart raised from any other thread would reach this handler.
pub fn holds_exit(code: Option<i32>, confirmed: bool) -> bool {
    !confirmed && code != Some(tauri::RESTART_EXIT_CODE)
}

const TITLE: &str = "Quit Agency?";
const LEAVE_LABEL: &str = "Leave running";
const STOP_LABEL: &str = "Stop and quit";
const QUIT_LABEL: &str = "Quit";
const CANCEL_LABEL: &str = "Cancel";

/// One of the quit prompt's buttons, and the quit it stands for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Choice {
    pub label: &'static str,
    pub agents: Agents,
}

/// Everything the quit prompt says and offers, for the styled dialog (sent as
/// the `quit-requested` payload) and the native fallback alike. Both used to
/// carry their own copy of the words and their own button-to-quit mapping,
/// which a test on each side pinned separately and nothing compared.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Prompt {
    pub title: &'static str,
    pub body: String,
    pub confirm: Choice,
    pub alt: Option<Choice>,
}

/// The quit prompt with `running` sessions in the daemon, `None` when the
/// count could not be read.
///
/// With anything running, or possibly running, leaving it is the confirm and
/// stopping it the alternative, so the safe answer is on the button that is
/// not red. With nothing running the one Quit leaves the daemon too. The count
/// is read when the prompt opens, and a scheduled run or a loop's next attempt
/// can start while it is up: a Quit that stopped the daemon would kill an agent
/// the prompt had just said nothing would interrupt. Left alone, a daemon with
/// no clients and nothing running exits by itself (`IDLE_GRACE` in
/// `term/server.rs`).
pub fn prompt(running: Option<usize>) -> Prompt {
    let leave = Choice { label: LEAVE_LABEL, agents: Agents::Leave };
    let stop = Choice { label: STOP_LABEL, agents: Agents::Stop };
    match running {
        Some(0) => Prompt {
            title: TITLE,
            body: body(running),
            confirm: Choice { label: QUIT_LABEL, agents: Agents::Leave },
            alt: None,
        },
        _ => Prompt { title: TITLE, body: body(running), confirm: leave, alt: Some(stop) },
    }
}

/// "Carry on" alone would overpromise. The daemon keeps the sessions, but the
/// app is what pings when a turn finishes, delivers a queued message, advances
/// a loop and serves each run's preview and editor tools, and none of that
/// runs while it is closed. It used to read "1 agent session(s) are running"
/// and say the next launch "picks them back up", which a release that changes
/// the daemon's protocol does not do.
fn body(running: Option<usize>) -> String {
    let (opening, them, they) = match running {
        Some(0) => return "All agents are idle; nothing will be interrupted.".to_string(),
        Some(1) => ("1 session is running.".to_string(), "it", "it carries"),
        Some(n) => (format!("{n} sessions are running."), "them", "they carry"),
        None => (
            "Agency can't tell how many sessions are running.".to_string(),
            "them",
            "any that are carry",
        ),
    };
    format!(
        "{opening} Leave {them} running and {they} on while Agency is closed, but notifications, \
         queued messages, loops and Agency's own tools wait until you reopen Agency. Stop \
         {them} and agents resume where they left off next time."
    )
}

/// The native fallback's answer, as a quit. `None` is a cancel, which is also
/// what anything unrecognised has to mean: a dialog that quits on a stray
/// result is worse than one that asks again.
fn fallback_choice(result: &MessageDialogResult, prompt: &Prompt) -> Option<Agents> {
    let MessageDialogResult::Custom(label) = result else { return None };
    [Some(&prompt.confirm), prompt.alt.as_ref()]
        .into_iter()
        .flatten()
        .find(|c| c.label == label)
        .map(|c| c.agents)
}

/// Ask the user to confirm quitting. The confirmation itself is rendered by
/// the frontend (a `quit-requested` listener shows an app-styled dialog and
/// calls `quit` with the answer), so it matches the rest of the UI instead of
/// the native alert. The native dialog remains only as a fallback for the
/// unlikely case where no main window exists to host the styled one.
pub fn request_quit(app: &tauri::AppHandle) {
    let state = app.state::<crate::state::AppState>();
    let prompt = prompt(state.running_session_count());

    if let Some(w) = app.get_webview_window("main") {
        // Quit can come from the tray while the window is hidden; surface the
        // window so the confirmation is actually visible.
        let _ = w.show();
        let _ = w.set_focus();
        if w.emit("quit-requested", &prompt).is_ok() {
            return;
        }
    }

    let buttons = match &prompt.alt {
        Some(alt) => MessageDialogButtons::YesNoCancelCustom(
            prompt.confirm.label.into(),
            alt.label.into(),
            CANCEL_LABEL.into(),
        ),
        None => {
            MessageDialogButtons::OkCancelCustom(prompt.confirm.label.into(), CANCEL_LABEL.into())
        }
    };
    let app2 = app.clone();
    app.dialog()
        .message(prompt.body.clone())
        .title(prompt.title)
        .buttons(buttons)
        .show_with_result(move |result| {
            if let Some(agents) = fallback_choice(&result, &prompt) {
                quit(&app2, agents);
            }
        });
}

/// The user confirmed: deal with the agents as they chose, then exit. Only the
/// first answer counts (see `ANSWERED`).
pub fn quit(app: &tauri::AppHandle, agents: Agents) {
    if ANSWERED.swap(true, Ordering::SeqCst) {
        return;
    }
    app.state::<crate::state::AppState>().prepare_exit(agents);
    QUIT_CONFIRMED.store(true, Ordering::Relaxed);
    app.exit(0);
}

/// Relaunch, leaving the agents running. An update's restart used to keep them
/// only because `app.restart()` skips the exit handler that asks about them;
/// this makes it the path rather than the accident.
pub fn restart(app: &tauri::AppHandle) -> ! {
    app.state::<crate::state::AppState>().prepare_exit(Agents::Leave);
    QUIT_CONFIRMED.store(true, Ordering::Relaxed);
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(label: &str) -> MessageDialogResult {
        MessageDialogResult::Custom(label.to_string())
    }

    #[test]
    fn an_unconfirmed_quit_is_held_for_the_prompt() {
        assert!(holds_exit(None, false));
        assert!(holds_exit(Some(0), false));
    }

    #[test]
    fn a_confirmed_quit_goes_through() {
        assert!(!holds_exit(None, true));
        assert!(!holds_exit(Some(0), true));
    }

    #[test]
    fn a_restart_is_never_held() {
        assert!(!holds_exit(Some(tauri::RESTART_EXIT_CODE), false));
    }

    #[test]
    fn with_agents_running_the_confirm_leaves_them_and_the_red_button_stops_them() {
        let p = prompt(Some(3));
        assert_eq!(p.confirm, Choice { label: "Leave running", agents: Agents::Leave });
        assert_eq!(p.alt, Some(Choice { label: "Stop and quit", agents: Agents::Stop }));
    }

    #[test]
    fn a_count_that_could_not_be_read_still_offers_both() {
        // A daemon that did not answer used to count as idle, whose one button
        // stopped everything.
        let p = prompt(None);
        assert_eq!(p.confirm.agents, Agents::Leave);
        assert_eq!(p.alt.map(|c| c.agents), Some(Agents::Stop));
        assert!(p.body.starts_with("Agency can't tell how many sessions are running."));
        assert!(p.body.contains("Leave them running and any that are carry on"), "{}", p.body);
    }

    #[test]
    fn the_idle_quit_leaves_the_daemon_to_exit_by_itself() {
        // An agent that starts while the idle prompt is up must not die to it.
        let p = prompt(Some(0));
        assert_eq!(p.confirm, Choice { label: "Quit", agents: Agents::Leave });
        assert_eq!(p.alt, None);
        assert_eq!(p.body, "All agents are idle; nothing will be interrupted.");
    }

    #[test]
    fn the_fallback_maps_each_button_to_its_quit() {
        let busy = prompt(Some(3));
        assert_eq!(fallback_choice(&custom(LEAVE_LABEL), &busy), Some(Agents::Leave));
        assert_eq!(fallback_choice(&custom(STOP_LABEL), &busy), Some(Agents::Stop));
        assert_eq!(fallback_choice(&custom(CANCEL_LABEL), &busy), None);
        assert_eq!(fallback_choice(&custom(QUIT_LABEL), &prompt(Some(0))), Some(Agents::Leave));
    }

    #[test]
    fn anything_unrecognised_is_a_cancel() {
        let busy = prompt(Some(3));
        assert_eq!(fallback_choice(&MessageDialogResult::Cancel, &busy), None);
        assert_eq!(fallback_choice(&MessageDialogResult::Ok, &busy), None);
        assert_eq!(fallback_choice(&MessageDialogResult::Yes, &prompt(Some(0))), None);
        // "Quit" only exists on the idle prompt; with agents running it would
        // have to say which kind of quit it meant.
        assert_eq!(fallback_choice(&custom(QUIT_LABEL), &busy), None);
        assert_eq!(fallback_choice(&custom(STOP_LABEL), &prompt(Some(0))), None);
    }

    #[test]
    fn the_body_offers_both_ways_out_in_the_singular() {
        assert_eq!(
            body(Some(1)),
            "1 session is running. Leave it running and it carries on while Agency is closed, \
             but notifications, queued messages, loops and Agency's own tools wait until you \
             reopen Agency. Stop it and agents resume where they left off next time."
        );
    }

    #[test]
    fn the_body_offers_both_ways_out_in_the_plural() {
        let many = body(Some(12));
        assert!(many.starts_with("12 sessions are running. Leave them running and they carry"));
        assert!(many.contains("Stop them and agents resume"));
        assert!(!many.contains("(s)"), "{many}");
    }

    #[test]
    fn the_body_says_what_waits_for_the_app_while_it_is_closed() {
        // The app, not the daemon, pings on a finished turn, delivers queued
        // messages, advances loops and serves the preview tools: saying "carry
        // on" alone would promise all of them.
        let b = body(Some(3));
        for paused in ["notifications", "queued messages", "loops", "Agency's own tools"] {
            assert!(b.contains(paused), "{paused}: {b}");
        }
        assert!(b.contains("until you reopen Agency"), "{b}");
    }

    #[test]
    fn the_body_promises_no_adoption() {
        for running in [None, Some(1), Some(4)] {
            let b = body(running);
            assert!(!b.contains("picks them back up"), "{b}");
            assert!(!b.contains("next launch"), "{b}");
        }
    }

    #[test]
    fn no_copy_has_an_em_dash() {
        for running in [None, Some(0), Some(1), Some(7)] {
            let p = prompt(running);
            assert!(!p.body.contains('—'), "{}", p.body);
            assert!(!p.title.contains('—'));
        }
    }
}
