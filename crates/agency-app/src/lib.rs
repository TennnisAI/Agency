mod commands;
mod state;

pub use state::{AppState, TaskInfo};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = AppState::new(&data_dir.join("agency.db"))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::add_project,
            commands::remove_project,
            commands::start_task,
            commands::send_input,
            commands::task_status,
            commands::stop_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
