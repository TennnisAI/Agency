//! Quit flow: confirm, stop all agents, shut the daemon down, exit.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

/// Set to `true` once the user clicks "Quit" in the confirmation dialog so that
/// the second `RunEvent::ExitRequested` event (triggered by `app.exit(0)`) is
/// not intercepted again, avoiding an infinite dialog loop.
pub static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// Show a confirmation dialog warning that running agents will be stopped.
/// On confirm: kills every daemon session, shuts down the daemon, then exits.
/// On cancel: does nothing (the app stays alive).
pub fn request_quit(app: &tauri::AppHandle) {
    let state = app.state::<crate::state::AppState>();
    let running = state.session_count();

    let message = if running > 0 {
        format!("Quitting will stop {running} running agent(s). Continue?")
    } else {
        "Quit Agency?".to_string()
    };

    let app2 = app.clone();
    app.dialog()
        .message(message)
        .title("Quit Agency")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit".into(),
            "Cancel".into(),
        ))
        .show(move |confirmed| {
            if confirmed {
                let state = app2.state::<crate::state::AppState>();
                state.kill_all_and_shutdown();
                QUIT_CONFIRMED.store(true, Ordering::Relaxed);
                app2.exit(0);
            }
        });
}
