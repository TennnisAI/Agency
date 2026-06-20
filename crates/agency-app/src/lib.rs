mod commands;
mod state;

pub use state::{AppState, ProviderSettings, RunInfo};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            commands::create_run,
            commands::list_runs,
            commands::run_preview,
            commands::attach_run,
            commands::detach_run,
            commands::run_input,
            commands::run_status,
            commands::discard_run,
            commands::rerun,
            commands::git_status,
            commands::git_diff,
            commands::git_log,
            commands::git_stage,
            commands::git_unstage,
            commands::git_commit,
            commands::git_push,
            commands::list_profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::get_settings,
            commands::save_settings,
            commands::merge_task,
            commands::abort_merge_task,
            commands::resolve_merge,
            commands::resolver_input,
            commands::resolver_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
