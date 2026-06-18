mod commands;
mod state;

pub use state::{AppState, TaskInfo};

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
