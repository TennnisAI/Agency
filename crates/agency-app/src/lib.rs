mod commands;
mod notifier;
mod state;

pub use state::{AppState, ProviderSettings, RunInfo};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            use tauri::Manager;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = AppState::new(&data_dir.join("agency.db"))?;
            app.manage(state);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                use std::collections::{HashMap, HashSet};
                use tauri::Manager;
                use tauri_plugin_notification::NotificationExt;

                let poll_secs: u64 = 2;
                let mut watches: HashMap<String, crate::notifier::RunWatch> = HashMap::new();
                let mut tick: u64 = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(poll_secs));
                    tick += 1;
                    let state = handle.state::<AppState>();
                    let settings = state.notif_settings().unwrap_or_default();
                    let (focused, active) = state.ui_snapshot();
                    let snaps = match state.watch_snapshot() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut seen: HashSet<String> = HashSet::new();
                    for snap in &snaps {
                        seen.insert(snap.id.clone());
                        let (watch, events) =
                            crate::notifier::step(watches.get(&snap.id), snap, tick, poll_secs, settings.idle_secs);
                        for ev in &events {
                            let enabled = match ev {
                                crate::notifier::NotifyKind::Finished => settings.agent_finished,
                                crate::notifier::NotifyKind::RunCrashed => settings.run_crashed,
                                crate::notifier::NotifyKind::Idle => settings.agent_idle,
                            };
                            let suppressed = (settings.only_when_unfocused && focused)
                                || active.as_deref() == Some(snap.id.as_str());
                            if enabled && !suppressed {
                                let (title, body) = crate::notifier::message(ev, &snap.label);
                                let _ = handle.notification().builder().title(title).body(body).show();
                            }
                        }
                        watches.insert(snap.id.clone(), watch);
                    }
                    watches.retain(|id, _| seen.contains(id));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::add_project,
            commands::close_project,
            commands::delete_project,
            commands::create_run,
            commands::create_terminal,
            commands::set_run_title,
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
            commands::archive_run,
            commands::restore_run,
            commands::list_archived_runs,
            commands::set_ui_state,
            commands::get_notif_settings,
            commands::save_notif_settings,
            commands::add_review_comment,
            commands::list_review_comments,
            commands::delete_review_comment,
            commands::send_review_comments,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
