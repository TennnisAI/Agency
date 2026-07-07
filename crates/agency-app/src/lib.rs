mod commands;
mod lifecycle;
mod looper;
mod notifier;
mod pathenv;
mod resume_probe;
mod state;
mod tray;

pub use state::{AppState, ProviderSettings, RunInfo};

pub fn run() {
    // Repair PATH first: a Finder-launched .app inherits launchd's minimal PATH,
    // which omits Homebrew etc., so spawning the daemon/agent CLIs fails with ENOENT.
    pathenv::repair();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Don't quit — retreat to the menu bar. Quit happens only via the
                // tray "Quit Agency" item (Task 13).
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            use tauri::Manager;
            use tauri::tray::TrayIconBuilder;

            // Build the tray icon and move the handle into managed state so it
            // is not dropped at the end of this setup closure. In Tauri 2 the
            // underlying icon is reference-counted and is removed from the menu
            // bar when the last handle is dropped — keeping it in managed state
            // ties its lifetime to the app itself. The menu itself is owned by
            // tray::refresh, which the watcher thread re-runs as run status
            // changes; the initial refresh below populates the static items.
            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(|app, event| crate::tray::on_menu_event(app, event))
                .build(app)?;
            app.manage(tray);
            crate::tray::refresh(app.handle(), &[])?;

            // macOS attributes notifications to a bundle; an unbundled dev run
            // defaults to Terminal's identity (hence its icon on every
            // notification). Point notify-rust at the Agency bundle so they
            // carry our logo. Best-effort: fails harmlessly when no Agency.app
            // is installed to attribute to.
            #[cfg(target_os = "macos")]
            {
                let _ = notify_rust::set_application(&app.config().identifier);
            }

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = AppState::new(&data_dir.join("agency.db"), &data_dir)?;
            app.manage(state);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                use std::collections::{HashMap, HashSet};
                use tauri::Manager;
                use tauri_plugin_notification::NotificationExt;

                let poll_secs: u64 = 2;
                let mut watches: HashMap<String, crate::notifier::RunWatch> = HashMap::new();
                let mut tick: u64 = 0;
                let mut tray_items: Vec<crate::tray::TrayRun> = Vec::new();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(poll_secs));
                    tick += 1;
                    let state = handle.state::<AppState>();

                    // Keep the tray menu in step with live runs; rebuild only on
                    // change (menu APIs must run on the main thread).
                    if let Ok(items) = state.tray_runs() {
                        if items != tray_items {
                            tray_items = items.clone();
                            let handle2 = handle.clone();
                            let _ = handle.run_on_main_thread(move || {
                                let _ = crate::tray::refresh(&handle2, &items);
                            });
                        }
                    }

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
                            // Consume the idle gate at the edge (whether or not the
                            // notification is shown) so the run won't nudge again
                            // until the user drives another turn.
                            if matches!(ev, crate::notifier::NotifyKind::Idle) {
                                state.clear_input_seen(&snap.id);
                            }
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
                                // Clicking the notification activates the app; the
                                // focus edge in set_ui_state deep-links to this run.
                                // Only armed while unfocused — a notification seen
                                // while already in the app shouldn't cause a jump
                                // on some later blur/refocus.
                                if !focused {
                                    state.note_notification(&snap.project_id, &snap.id);
                                }
                            }
                        }
                        watches.insert(snap.id.clone(), watch);
                    }
                    watches.retain(|id, _| seen.contains(id));
                }
            });

            // Loop driver: advance active loops (respawn attempts, run
            // checks) and toast their terminal transitions, with the same
            // suppression rules as run notifications. Its own thread — a
            // respawn does blocking git work (WIP commit) that must not
            // stall the notifier/tray tick above. Idle cost is one atomic
            // load per tick while no loops exist.
            let loop_handle = app.handle().clone();
            std::thread::spawn(move || {
                use tauri::Manager;
                use tauri_plugin_notification::NotificationExt;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let state = loop_handle.state::<AppState>();
                    let notices = match state.drive_loops() {
                        Ok(n) => n,
                        Err(e) => {
                            log::warn!("drive_loops: {e}");
                            continue;
                        }
                    };
                    if notices.is_empty() {
                        continue;
                    }
                    let settings = state.notif_settings().unwrap_or_default();
                    let (focused, active) = state.ui_snapshot();
                    for n in notices {
                        let suppressed = !settings.loop_events
                            || (settings.only_when_unfocused && focused)
                            || active.as_deref() == Some(n.run_id.as_str());
                        if suppressed {
                            continue;
                        }
                        let (title, body) = if n.done {
                            ("Loop complete".to_string(),
                             format!("{} — checks passed on attempt {}", n.label, n.attempt))
                        } else {
                            ("Loop stalled".to_string(),
                             format!("{} — stopped after attempt {}, checks still failing", n.label, n.attempt))
                        };
                        let _ = loop_handle.notification().builder().title(title).body(body).show();
                        if !focused {
                            state.note_notification(&n.project_id, &n.run_id);
                        }
                    }
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
            commands::create_loop,
            commands::stop_loop,
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
            commands::ensure_run_active,
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
            commands::git_set_remote,
            commands::run_branches,
            commands::list_profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::get_settings,
            commands::save_settings,
            commands::merge_preview,
            commands::merge_task,
            commands::abort_merge_task,
            commands::gh_readiness,
            commands::create_pr,
            commands::pr_status,
            commands::send_check_feedback,
            commands::list_mcp_servers,
            commands::save_mcp_servers,
            commands::get_knowledge_config,
            commands::save_knowledge_config,
            commands::create_race,
            commands::list_gh_issues,
            commands::list_gh_prs,
            commands::create_run_from_issue,
            commands::create_run_from_pr,
            commands::resolve_merge,
            commands::resolver_input,
            commands::resolver_status,
            commands::resolver_resize,
            commands::git_parse_diff,
            commands::git_stage_hunk,
            commands::git_unstage_hunk,
            commands::git_log_graph,
            commands::git_branch_info,
            commands::list_project_branches,
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
            commands::start_run_session,
            commands::list_run_sessions,
            commands::close_run_session,
            commands::start_shell,
            commands::stop_shell,
            commands::shell_status,
            commands::shell_preview,
            commands::attach_shell,
            commands::detach_shell,
            commands::shell_input,
            commands::resize_shell,
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
            commands::list_dir,
            commands::read_file,
            commands::write_file,
            commands::read_file_base64,
            commands::confirm_quit,
            commands::agent_installed,
            commands::create_install_terminal,
        ])
        .build(tauri::generate_context!())
        .expect("error while running Agency")
        .run(|app, event| {
            // Intercept OS-level quit (Cmd+Q, dock menu, etc.) so it routes
            // through our confirmation dialog instead of exiting immediately.
            // Once the user confirms, QUIT_CONFIRMED is set to true and we let
            // the subsequent exit triggered by app.exit(0) proceed unimpeded.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !crate::lifecycle::QUIT_CONFIRMED.load(std::sync::atomic::Ordering::Relaxed) {
                    api.prevent_exit();
                    crate::lifecycle::request_quit(app);
                }
            }
        });
}
