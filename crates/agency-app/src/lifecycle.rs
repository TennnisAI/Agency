//! Quit flow: confirm, stop all agents, shut the daemon down, exit.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

/// Set to `true` once the user confirms quitting so that the second
/// `RunEvent::ExitRequested` event (triggered by `app.exit(0)`) is not
/// intercepted again, avoiding an infinite dialog loop.
pub static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// Ask the user to confirm quitting. The confirmation itself is rendered by
/// the frontend (a `quit-requested` listener shows an app-styled dialog and
/// calls the `confirm_quit` command), so it matches the rest of the UI instead
/// of the native alert. The native dialog remains only as a fallback for the
/// unlikely case where no main window exists to host the styled one.
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

    let message = if running > 0 {
        format!("Quitting will stop {running} running agent(s). Continue?")
    } else {
        "Quit Agency?".to_string()
    };
    let app2 = app.clone();
    app.dialog()
        .message(message)
        .title("Quit Agency")
        .buttons(MessageDialogButtons::OkCancelCustom("Quit".into(), "Cancel".into()))
        .show(move |confirmed| {
            if confirmed {
                confirm_quit(&app2);
            }
        });
}

/// The user confirmed: kill every daemon session, shut the daemon down, exit.
pub fn confirm_quit(app: &tauri::AppHandle) {
    let state = app.state::<crate::state::AppState>();
    state.kill_all_and_shutdown();
    QUIT_CONFIRMED.store(true, Ordering::Relaxed);
    app.exit(0);
}
