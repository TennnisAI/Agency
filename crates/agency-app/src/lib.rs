mod commands;
mod state;

pub use state::{AppState, ProviderSettings, RunInfo};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
            commands::close_project,
            commands::delete_project,
            commands::create_run,
            commands::list_runs,
            commands::run_preview,
            commands::attach_run,
            commands::detach_run,
            commands::run_input,
            commands::resize_run,
            commands::run_status,
            commands::discard_run,
            commands::stop_run,
            commands::rerun,
            commands::git_status,
            commands::git_diff,
            commands::git_stage,
            commands::git_unstage,
            commands::git_stage_all,
            commands::git_unstage_all,
            commands::git_discard,
            commands::git_discard_all,
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
            commands::resolver_resize,
            commands::git_parse_diff,
            commands::git_stage_hunk,
            commands::git_unstage_hunk,
            commands::git_log_graph,
            commands::git_branch_info,
            commands::inspect_repo,
            commands::init_repo,
            commands::commit_repo,
            commands::git_commit_files,
            commands::git_commit_diff,
            commands::git_commit_amend,
            commands::git_stage_lines,
            commands::git_unstage_lines,
            commands::git_revert_lines,
            commands::run_script_configured,
            commands::start_run_script,
            commands::stop_run_script,
            commands::run_script_status,
            commands::run_script_preview,
            commands::attach_run_script,
            commands::detach_run_script,
            commands::run_script_input,
            commands::resize_run_script,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
