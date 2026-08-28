mod activity;
mod agent_catalog;
mod agent_diag;
mod clipboard;
mod commands;
mod datadir;
mod gates;
mod lifecycle;
mod looper;
mod menu;
mod model_probe;
#[cfg(target_os = "macos")]
mod notif_macos;
mod notifier;
mod pathenv;
mod preview_shot;
mod resume_probe;
mod sendq;
mod state;
mod tray;
mod update;
#[cfg(target_os = "macos")]
mod webview_menu;

// The queue's two after-the-fact reports are part of the public surface: they
// come back out of `drain_send_queues` for the caller to show.
pub use sendq::{Notice, NoticeKind};
// The derived state a run is in, so the attention tests can assert on it.
pub use activity::ActivityState;
pub use state::{AppState, KnowledgeConfigDto, ProviderSettings, RunInfo};

/// Log every panic (the default hook only writes to stderr, which a bundled
/// app loses) so post-mortems in the log file show why a thread died.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "<non-string panic payload>"
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".into());
        log::error!("panic in thread '{name}' at {location}: {msg}");
    }));
}

fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{Target, TargetKind};
    let builder = tauri_plugin_log::Builder::new()
        .level(log::LevelFilter::Info)
        .clear_targets()
        // OS log dir (macOS: ~/Library/Logs/<bundle-id>), rotated so it
        // cannot grow without bound. The log dir comes from the bundle
        // identifier, which a dev build shares, so dev writes to a file of its
        // own: otherwise the two interleave and either one's rotation discards
        // the other's history (KeepOne).
        .target(Target::new(TargetKind::LogDir {
            file_name: tauri::is_dev().then(|| "Agency-dev".to_string()),
        }))
        .max_file_size(5 * 1024 * 1024)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne);
    #[cfg(debug_assertions)]
    let builder = builder.target(Target::new(TargetKind::Stdout));
    builder.build()
}

pub fn run() {
    // Repair PATH first: a Finder-launched .app inherits launchd's minimal PATH,
    // which omits Homebrew etc., so spawning the daemon/agent CLIs fails with ENOENT.
    pathenv::repair();
    install_panic_hook();
    tauri::Builder::default()
        .plugin(log_plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .menu(|app| menu::build(app))
        .on_menu_event(|app, event| menu::on_event(app, event))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Don't quit — retreat to the menu bar. Quit happens only via the
                // tray "Quit Agency" item (Task 13).
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            if let Err(e) = setup_app(app) {
                // A Finder-launched app that fails setup would otherwise just
                // bounce and vanish; say why, then exit cleanly.
                log::error!("startup failed: {e}");
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Agency failed to start")
                    .set_description(format!("{e}"))
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
                std::process::exit(1);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            clipboard::read_clipboard_text,
            commands::list_projects,
            commands::add_project,
            commands::get_workspace,
            commands::default_workspace_location,
            commands::create_workspace,
            commands::move_workspace,
            commands::close_project,
            commands::delete_project,
            commands::set_project_color,
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
            commands::git_sync,
            commands::git_fetch,
            commands::git_auto_fetch,
            commands::git_pull,
            commands::git_set_remote,
            commands::git_checkout_branch,
            commands::git_create_branch,
            commands::git_delete_branch,
            commands::git_list_branches,
            commands::git_pull_rebase,
            commands::git_push_force,
            commands::git_undo_last_commit,
            commands::git_reset_to,
            commands::git_revert_commit,
            commands::git_cherry_pick,
            commands::git_stash_list,
            commands::git_stash_push,
            commands::git_stash_apply,
            commands::git_stash_pop,
            commands::git_stash_drop,
            commands::run_branches,
            commands::list_profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::agent_onboarding_needed,
            commands::list_agent_catalog,
            commands::list_agent_models,
            commands::probe_agent_models,
            commands::enable_agent_profiles,
            commands::complete_agent_onboarding,
            commands::get_settings,
            commands::save_settings,
            commands::merge_preview,
            commands::merge_task,
            commands::merge_status,
            commands::finish_merge_task,
            commands::abort_merge_task,
            commands::gh_readiness,
            commands::gh_auth_readiness,
            commands::create_pr,
            commands::pr_status,
            commands::send_check_feedback,
            commands::pr_detail,
            commands::gh_current_login,
            commands::pr_merge_methods,
            commands::merge_pr,
            commands::pr_diff,
            commands::pr_review_threads,
            commands::submit_pr_review,
            commands::reply_pr_comment,
            commands::edit_pr,
            commands::edit_pr_comment,
            commands::resolve_pr_thread,
            commands::unresolve_pr_thread,
            commands::pr_number_for_run,
            commands::create_pr_from_branch,
            commands::list_mcp_servers,
            commands::save_mcp_servers,
            commands::get_knowledge_config,
            commands::save_knowledge_config,
            commands::set_knowledge_backend,
            commands::build_knowledge_graph,
            commands::knowledge_graph_view,
            commands::install_knowledge_tooling,
            commands::get_files_config,
            commands::save_files_config,
            commands::import_mcp_json,
            commands::authenticate_mcp_server,
            commands::deauthenticate_mcp_server,
            commands::create_race,
            commands::list_gh_issues,
            commands::list_gh_prs,
            commands::create_run_from_issue,
            commands::create_run_from_pr,
            commands::create_pr_review_run,
            commands::list_issues,
            commands::create_issue,
            commands::update_issue,
            commands::delete_issue,
            commands::add_issue_comment,
            commands::update_issue_comment,
            commands::delete_issue_comment,
            commands::start_issue_run,
            commands::start_issue_race,
            commands::start_issue_loop,
            commands::send_merge_conflict,
            commands::git_parse_diff,
            commands::git_blob_sides,
            commands::git_stage_hunk,
            commands::git_unstage_hunk,
            commands::git_log_graph,
            commands::git_branch_info,
            commands::list_project_branches,
            commands::inspect_repo,
            commands::init_repo,
            commands::clone_repo,
            commands::cancel_clone,
            commands::cancel_push,
            commands::commit_repo,
            commands::cancel_repo_setup,
            commands::scan_large_files,
            commands::git_commit_files,
            commands::git_commit_diff,
            commands::git_commit_amend,
            commands::git_stage_lines,
            commands::git_unstage_lines,
            commands::git_revert_lines,
            commands::run_script_config,
            commands::preview_targets,
            commands::set_preview_rect,
            commands::save_run_scripts,
            commands::start_run_script,
            commands::stop_run_script,
            commands::run_scripts_status,
            commands::run_scripts_live,
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
            commands::run_cleanup,
            commands::read_run_record,
            commands::read_run_conversation,
            commands::discard_archived_runs,
            commands::set_ui_state,
            commands::set_menu_context,
            commands::get_notif_settings,
            commands::save_notif_settings,
            commands::check_for_update,
            commands::get_update_check_enabled,
            commands::set_update_check_enabled,
            commands::add_review_comment,
            commands::list_review_comments,
            commands::delete_review_comment,
            commands::send_review_comments,
            commands::list_queued_messages,
            commands::cancel_queued_message,
            commands::list_dir,
            commands::read_file,
            commands::write_file,
            commands::create_file,
            commands::create_dir,
            commands::rename_path,
            commands::copy_path,
            commands::trash_path,
            commands::add_to_gitignore,
            commands::abs_path,
            commands::reveal_path,
            commands::resolve_term_paths,
            commands::open_term_path,
            commands::rename_run,
            commands::rename_run_branch,
            commands::set_run_standing,
            commands::pin_run,
            commands::read_file_base64,
            commands::detect_docs_dir,
            commands::read_docs_corpus,
            commands::docs_corpus_stats,
            commands::read_docs_files,
            commands::ensure_workspace_guide,
            commands::scan_tasks,
            commands::toggle_task,
            commands::search_files,
            commands::list_files,
            commands::write_file_base64,
            commands::import_file,
            commands::confirm_quit,
            commands::agent_installed,
            commands::agent_cli_info,
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

fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    // `pathenv::repair` runs before the log plugin exists, so it records its
    // outcome instead of logging it. Write it down here: an agent CLI the app
    // cannot see is almost always a PATH the probe never composed, and without
    // this line the only way to tell is a `ps eww` of the running app.
    log::info!("{}", pathenv::report());

    // Build the tray icon and move the handle into managed state so it
    // is not dropped at the end of setup. In Tauri 2 the underlying icon
    // is reference-counted and is removed from the menu bar when the last
    // handle is dropped — keeping it in managed state ties its lifetime
    // to the app itself. The menu itself is owned by tray::refresh, which
    // the watcher thread re-runs as run status changes; the initial
    // refresh below populates the static items.
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
        // And let banners through while Agency itself is the app in front —
        // otherwise macOS files them away unseen and only the ones raised
        // while the app is in the background are ever visible (AGE-33).
        crate::notif_macos::present_while_frontmost();
        // And take WebKit's own dead items out of the context menus the app
        // cannot reach from JS: the ones inside the preview frames (AGE-158).
        crate::webview_menu::install();
    }

    // A `tauri dev` build takes a data dir of its own so it cannot share (and
    // fight over) the installed app's DB and terminal daemon — see datadir.
    let data_dir = datadir::prepare(&app.path().app_data_dir()?, tauri::is_dev())?;
    let db_path = data_dir.join(datadir::DB_NAME);
    // Snapshot the DB once per app-version change before migrations touch it.
    // Best-effort: a failed backup is worth a log line, not a failed launch.
    let app_version = app.package_info().version.to_string();
    match agency_core::registry::backup_before_migrations(&db_path, &app_version) {
        Ok(Some(bak)) => log::info!("backed up db to {} before migrations", bak.display()),
        Ok(None) => {}
        Err(e) => log::warn!("db backup before migrations failed: {e:#}"),
    }
    let state = AppState::new(&db_path, &data_dir)?;
    app.manage(state);
    {
        // Preview MCP servers (AGE-143): give them their native-screenshot
        // provider, then bring servers up for the runs that already exist —
        // agents surviving in the daemon reconnect to their tools without
        // waiting out the first notifier tick.
        let state = app.state::<AppState>();
        state.set_preview_shot(crate::preview_shot::provider(app.handle().clone()));
        state.sync_preview_servers();
    }
    let handle = app.handle().clone();
    std::thread::Builder::new().name("notifier".into()).spawn(move || {
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
            // catch_unwind per tick: a panic here would otherwise kill
            // notifications and tray updates for the rest of the app's life
            // while the UI keeps running. The hook has already logged it.
            let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                    Err(_) => return,
                };
                let now_ms = crate::activity::now_ms();
                let mut seen: HashSet<String> = HashSet::new();
                for snap in &snaps {
                    seen.insert(snap.id.clone());
                    // Busy/idle bookkeeping shares the notification poll: the
                    // pane-changed bit here is the same edge step() detects.
                    let pane_changed = watches
                        .get(&snap.id)
                        .map(|w| w.pane_hash != snap.pane_hash)
                        .unwrap_or(true);
                    state.update_activity(&snap.id, pane_changed, now_ms);
                    let (watch, events) = crate::notifier::step(
                        watches.get(&snap.id),
                        snap,
                        tick,
                        poll_secs,
                        settings.idle_secs,
                    );
                    for ev in &events {
                        // Consume the idle gate at the edge (whether or not the
                        // notification is shown) so the run won't nudge again
                        // until the user drives another turn.
                        if matches!(ev, crate::notifier::NotifyKind::Idle) {
                            state.clear_input_seen(&snap.id);
                        }
                        let enabled = match ev {
                            crate::notifier::NotifyKind::Finished => settings.agent_finished,
                            crate::notifier::NotifyKind::RunCrashed(_) => settings.run_crashed,
                            crate::notifier::NotifyKind::Idle => settings.agent_idle,
                        };
                        let suppressed = crate::notifier::suppressed(
                            &settings,
                            focused,
                            active.as_deref(),
                            &snap.id,
                        );
                        if enabled && !suppressed {
                            let (title, body) = crate::notifier::message(ev, &snap.label);
                            let _ = handle.notification().builder().title(title).body(body).show();
                            // Clicking the notification activates the app; the
                            // focus edge in set_ui_state deep-links to this run.
                            // Only armed while unfocused — a notification seen
                            // while already in the app shouldn't cause a jump
                            // on some later blur/refocus. A `project:` snapshot
                            // (the checkout's own run scripts) names no run, so
                            // it opens the project without deep-linking.
                            if !focused && !snap.id.starts_with("project:") {
                                state.note_notification(&snap.project_id, &snap.id);
                            }
                        }
                    }
                    watches.insert(snap.id.clone(), watch);
                }
                watches.retain(|id, _| seen.contains(id));
                state.retain_activity(&seen);
                // Text Agency owes an agent goes out here, after the loop above
                // has refreshed the busy/idle bookkeeping the decision reads.
                // See `crate::sendq` for what it waits for.
                for notice in state.drain_send_queues(now_ms) {
                    // A queue thrown away, or a message appended to whatever
                    // was on the prompt line. Both happen long after whoever
                    // sent the text closed the window they sent it from, so the
                    // UI toasts them; being held is already on the run's own
                    // marker. Not an OS notification: it carries no action, and
                    // the categories in Settings are per run event.
                    use tauri::Emitter;
                    let _ = handle.emit("send-queue-notice", &notice);
                }
                // Token accounting rides this poll rather than running a
                // thread of its own: an unchanged transcript costs one stat,
                // so the marginal price of doing it here is close to nothing.
                if let Err(e) = state.refresh_usage() {
                    log::warn!("usage refresh failed: {e}");
                }
                // Preview MCP servers converge here too: archive, discard,
                // restore and config edits all land within one tick, and a
                // server that failed to bind retries on its own cadence.
                state.sync_preview_servers();
            }));
            if tick_result.is_err() {
                log::error!("notifier tick panicked; continuing");
            }
        }
    })?;

    // Loop driver: advance active loops (respawn attempts, run
    // checks) and toast their terminal transitions, with the same
    // suppression rules as run notifications. Its own thread — a
    // respawn does blocking git work (WIP commit) that must not
    // stall the notifier/tray tick above. Idle cost is one atomic
    // load per tick while no loops exist.
    let loop_handle = app.handle().clone();
    std::thread::Builder::new().name("loop-driver".into()).spawn(move || {
        use tauri::Manager;
        use tauri_plugin_notification::NotificationExt;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            // catch_unwind per tick: a dead driver silently freezes every
            // loop's progression while the UI lives on.
            let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let state = loop_handle.state::<AppState>();
                let notices = match state.drive_loops() {
                    Ok(n) => n,
                    Err(e) => {
                        log::warn!("drive_loops: {e}");
                        return;
                    }
                };
                if notices.is_empty() {
                    return;
                }
                let settings = state.notif_settings().unwrap_or_default();
                let (focused, active) = state.ui_snapshot();
                for n in notices {
                    let suppressed = !settings.loop_events
                        || crate::notifier::suppressed(
                            &settings,
                            focused,
                            active.as_deref(),
                            &n.run_id,
                        );
                    if suppressed {
                        continue;
                    }
                    let (title, body) = if n.done {
                        (
                            "Loop complete".to_string(),
                            format!("{} — checks passed on attempt {}", n.label, n.attempt),
                        )
                    } else {
                        use agency_core::loops::StallReason;
                        // Name the cap that tripped: "Stalled" alone doesn't
                        // say whether to raise a cap, fix a flag, or rewrite
                        // the prompt (AGE-110).
                        let why = match n.reason {
                            Some(StallReason::WallClock) => {
                                format!("stopped at the time cap after attempt {}", n.attempt)
                            }
                            Some(StallReason::Budget) => {
                                format!("stopped at the token cap after attempt {}", n.attempt)
                            }
                            Some(StallReason::CrashLoop) => format!(
                                "stopped after {} agent failures in a row",
                                crate::looper::MAX_CONSECUTIVE_FAILURES
                            ),
                            // AttemptCap, and stalls from before reasons
                            // existed (spawn failures included).
                            _ => {
                                format!("stopped after attempt {}, checks still failing", n.attempt)
                            }
                        };
                        ("Loop stalled".to_string(), format!("{} — {}", n.label, why))
                    };
                    let _ = loop_handle.notification().builder().title(title).body(body).show();
                    if !focused {
                        state.note_notification(&n.project_id, &n.run_id);
                    }
                }
            }));
            if tick_result.is_err() {
                log::error!("loop-driver tick panicked; continuing");
            }
        }
    })?;

    // Background remote fetch: without this, ahead/behind compares against
    // local tracking refs that never move, so "behind" is stale forever. This
    // is the floor — every project's origin is contacted at least every 5 min
    // whether or not anyone is looking at it; the Source Control panel asks for
    // something fresher when it opens (`git_auto_fetch`), and both share the
    // per-project schedule and backoff in `AppState`. Its own thread —
    // `git fetch` blocks on the network and must not stall the notifier/tray or
    // loop-driver ticks; it holds no lock across the network call.
    let fetch_handle = app.handle().clone();
    std::thread::Builder::new().name("remote-fetch".into()).spawn(move || {
        use std::time::Duration;
        use tauri::Manager;

        // Let startup (window, first poll, daemon spawns) settle before the
        // first sweep so it doesn't contend with launch work. A panel opened
        // during that window fetches its own project immediately anyway.
        std::thread::sleep(Duration::from_secs(20));
        loop {
            let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                fetch_handle.state::<AppState>().sweep_project_fetches(state::FETCH_SWEEP_AGE);
            }));
            if tick_result.is_err() {
                log::error!("remote-fetch tick panicked; continuing");
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    })?;
    Ok(())
}
