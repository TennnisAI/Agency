//! Menu-bar tray icon: a live mission-control menu instead of bare Open/Quit.
//!
//! The watcher thread polls run status every couple of seconds and calls
//! [`refresh`] (on the main thread — menu APIs require it) whenever the set of
//! runs or their statuses change. Each run gets its own menu item; clicking it
//! surfaces the window and tells the frontend to open that run in focus mode.

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Emitter, Manager};

/// One run as shown in the tray menu. `Eq` so the watcher can cheaply detect
/// "menu contents changed" between polls and skip rebuilding when idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayRun {
    pub run_id: String,
    pub project_id: String,
    pub project: String,
    pub label: String,
    pub running: bool,
}

/// Payload for the `tray-open-run` event the frontend listens for. Also reused
/// by the notification deep-link (commands::set_ui_state) — both mean "surface
/// this run in focus mode".
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRun {
    pub project_id: String,
    pub run_id: String,
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Shared click handler for every tray menu item (set once on the tray icon;
/// it survives `set_menu` swaps).
pub fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    match id {
        "open" => show_main(app),
        "quit" => crate::lifecycle::request_quit(app),
        _ => {
            if let Some(rest) = id.strip_prefix("run:") {
                // Project ids are UUIDs and run ids are [a-z0-9-] slugs, so the
                // first ':' unambiguously separates them.
                if let Some((project_id, run_id)) = rest.split_once(':') {
                    show_main(app);
                    let _ = app.emit(
                        "tray-open-run",
                        OpenRun { project_id: project_id.into(), run_id: run_id.into() },
                    );
                }
            } else if let Some(project_id) = id.strip_prefix("proj:") {
                show_main(app);
                let _ = app.emit("tray-open-project", project_id.to_string());
            }
        }
    }
}

/// Menu items keep their full label readable but bounded — a run titled from a
/// long prompt would otherwise stretch the menu across the screen.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Rebuild the tray menu (and icon badge/tooltip) from the current runs.
/// Must run on the main thread.
pub fn refresh(app: &AppHandle, runs: &[TrayRun]) -> tauri::Result<()> {
    let tray = app.state::<TrayIcon>();
    let running = runs.iter().filter(|r| r.running).count();

    let mut menu = MenuBuilder::new(app);
    let header = if runs.is_empty() {
        "No active agents".to_string()
    } else {
        format!(
            "{} agent{} · {} running",
            runs.len(),
            if runs.len() == 1 { "" } else { "s" },
            running
        )
    };
    menu =
        menu.item(&MenuItemBuilder::with_id("hdr", header).enabled(false).build(app)?).separator();

    // Runs arrive grouped by project (tray_runs iterates projects in order);
    // emit a clickable project row before each group.
    let mut current_project: Option<&str> = None;
    for run in runs {
        if current_project != Some(run.project_id.as_str()) {
            current_project = Some(run.project_id.as_str());
            menu = menu.item(
                &MenuItemBuilder::with_id(
                    format!("proj:{}", run.project_id),
                    clip(&run.project, 40),
                )
                .build(app)?,
            );
        }
        let glyph = if run.running { "●" } else { "○" };
        menu = menu.item(
            &MenuItemBuilder::with_id(
                format!("run:{}:{}", run.project_id, run.run_id),
                format!("   {glyph} {}", clip(&run.label, 44)),
            )
            .build(app)?,
        );
    }
    if !runs.is_empty() {
        menu = menu.separator();
    }
    menu = menu
        .item(&MenuItemBuilder::with_id("open", "Open Agency").build(app)?)
        .item(&MenuItemBuilder::with_id("quit", "Quit Agency").build(app)?);

    tray.set_menu(Some(menu.build()?))?;
    // Dev builds now run alongside the installed app rather than replacing its
    // daemon, so two identical icons can sit in the menu bar; name which is which.
    let name = if tauri::is_dev() { "Agency (dev)" } else { "Agency" };
    let _ = tray.set_tooltip(Some(format!("{name} — {running} running")));
    // macOS: show the active-agent count next to the menu-bar icon.
    #[cfg(target_os = "macos")]
    {
        let title = if running > 0 { Some(running.to_string()) } else { None };
        let _ = tray.set_title(title);
    }
    Ok(())
}
