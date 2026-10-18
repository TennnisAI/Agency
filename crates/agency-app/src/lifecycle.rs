//! Quit flow: confirm, then either stop every agent and the daemon or leave
//! them running for the next launch to adopt, and exit.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

/// Set to `true` once the user confirms quitting so that the second
/// `RunEvent::ExitRequested` event (triggered by `app.exit(0)`) is not
/// intercepted again, avoiding an infinite dialog loop.
pub static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// What a quit does with the sessions the terminal daemon is hosting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agents {
    /// Kill every session and shut the daemon down.
    Stop,
    /// Leave the daemon and its sessions running; the next launch adopts them
    /// (`AppState::new`'s rehydrate). Closing the window already worked this
    /// way, but quitting did not: twelve agents died to a quit that had nothing
    /// to do with them (AGE-210).
    Leave,
}

/// Whether an `ExitRequested` should be held for the quit confirmation.
///
/// Not once the user has confirmed, which is the exit `app.exit(0)` itself
/// raises. Not for a restart either: Tauri ignores `prevent_exit` for
/// `RESTART_EXIT_CODE`, so asking would only flash a dialog over an app that is
/// already going, and offer to kill the agents the restart is leaving alone.
pub fn holds_exit(code: Option<i32>, confirmed: bool) -> bool {
    !confirmed && code != Some(tauri::RESTART_EXIT_CODE)
}

// The native fallback's buttons. Its answer comes back as the label.
const LEAVE_LABEL: &str = "Leave running";
const STOP_LABEL: &str = "Stop and quit";
const CANCEL_LABEL: &str = "Cancel";

/// The native fallback's answer, as a quit. `None` is a cancel, which is also
/// what anything unrecognised has to mean: a dialog that quits on a stray
/// result is worse than one that asks again.
fn fallback_choice(result: &MessageDialogResult, running: usize) -> Option<Agents> {
    match result {
        MessageDialogResult::Custom(label) if label == LEAVE_LABEL => Some(Agents::Leave),
        MessageDialogResult::Custom(label) if label == STOP_LABEL => Some(Agents::Stop),
        // The idle prompt has a single "Quit", and with nothing running the
        // daemon has nothing to keep alive for.
        MessageDialogResult::Custom(label) if label == "Quit" && running == 0 => Some(Agents::Stop),
        _ => None,
    }
}

/// Ask the user to confirm quitting. The confirmation itself is rendered by
/// the frontend (a `quit-requested` listener shows an app-styled dialog and
/// calls `confirm_quit` or `quit_leaving_agents`), so it matches the rest of
/// the UI instead of the native alert. The native dialog remains only as a
/// fallback for the unlikely case where no main window exists to host the
/// styled one.
pub fn request_quit(app: &tauri::AppHandle) {
    let state = app.state::<crate::state::AppState>();
    let running = state.session_count();

    if let Some(w) = app.get_webview_window("main") {
        // Quit can come from the tray while the window is hidden; surface the
        // window so the confirmation is actually visible.
        let _ = w.show();
        let _ = w.set_focus();
        if w.emit("quit-requested", running).is_ok() {
            return;
        }
    }

    let (message, buttons) = if running > 0 {
        (
            format!(
                "{running} agent session(s) are running. Leave them running and Agency picks \
                 them back up when you reopen it, or stop them now."
            ),
            MessageDialogButtons::YesNoCancelCustom(
                LEAVE_LABEL.into(),
                STOP_LABEL.into(),
                CANCEL_LABEL.into(),
            ),
        )
    } else {
        (
            "Quit Agency?".to_string(),
            MessageDialogButtons::OkCancelCustom("Quit".into(), CANCEL_LABEL.into()),
        )
    };
    let app2 = app.clone();
    app.dialog().message(message).title("Quit Agency").buttons(buttons).show_with_result(
        move |result| {
            if let Some(agents) = fallback_choice(&result, running) {
                quit(&app2, agents);
            }
        },
    );
}

/// The user confirmed: deal with the agents as they chose, then exit.
pub fn quit(app: &tauri::AppHandle, agents: Agents) {
    let state = app.state::<crate::state::AppState>();
    match agents {
        Agents::Stop => state.kill_all_and_shutdown(),
        Agents::Leave => state.release_for_exit(),
    }
    QUIT_CONFIRMED.store(true, Ordering::Relaxed);
    app.exit(0);
}

/// Relaunch, leaving the agents running. An update's restart used to keep them
/// only because `app.restart()` skips the exit handler that asks about them;
/// this makes it the path rather than the accident.
pub fn restart(app: &tauri::AppHandle) -> ! {
    app.state::<crate::state::AppState>().release_for_exit();
    QUIT_CONFIRMED.store(true, Ordering::Relaxed);
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_fallback_maps_each_button_to_its_quit() {
        let custom = |s: &str| MessageDialogResult::Custom(s.to_string());
        assert_eq!(fallback_choice(&custom(LEAVE_LABEL), 3), Some(Agents::Leave));
        assert_eq!(fallback_choice(&custom(STOP_LABEL), 3), Some(Agents::Stop));
        assert_eq!(fallback_choice(&custom(CANCEL_LABEL), 3), None);
        assert_eq!(fallback_choice(&custom("Quit"), 0), Some(Agents::Stop));
    }

    #[test]
    fn anything_unrecognised_is_a_cancel() {
        let custom = |s: &str| MessageDialogResult::Custom(s.to_string());
        assert_eq!(fallback_choice(&MessageDialogResult::Cancel, 3), None);
        assert_eq!(fallback_choice(&MessageDialogResult::Ok, 3), None);
        assert_eq!(fallback_choice(&MessageDialogResult::Yes, 0), None);
        // "Quit" only exists on the idle prompt; with agents running it would
        // have to say which kind of quit it meant.
        assert_eq!(fallback_choice(&custom("Quit"), 3), None);
    }
}
