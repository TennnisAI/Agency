/// Request that the application quit.
///
/// Currently exits immediately (exit code 0).  Task 13 will replace this stub
/// with a confirmation flow before terminating.
pub fn request_quit(app: &tauri::AppHandle) {
    app.exit(0);
}
