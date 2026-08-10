//! Native application menu bar (the macOS top menu). It mirrors the app's real
//! features so they are discoverable and keyboard-accessible instead of living
//! only behind buttons and hidden shortcuts.
//!
//! Each custom item carries a `menu:<action>` id and, when clicked, emits a
//! `menu` event whose payload is `<action>`. The frontend routes that to the
//! same handlers its buttons/shortcuts already use (see App.tsx `onMenu`), so
//! the menu never drifts from the UI. Edit/Window items and text-editing
//! commands are *predefined* items so native copy/paste/undo and window
//! management work without any wiring.
//!
//! Context-dependent items (New Agent/Terminal need a selected project; the
//! Agent menu needs a focused agent) start disabled and are toggled by
//! [`set_context`], which the frontend drives via the `set_menu_context`
//! command as the selection changes — otherwise they'd be clickable no-ops.

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};

/// Build the whole menu bar. Called once from `setup`.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    // App menu — on macOS its title is replaced by the app name automatically.
    let app_menu = SubmenuBuilder::new(app, "Agency")
        .about(None)
        .separator()
        .item(
            &MenuItemBuilder::with_id("menu:settings", "Settings…")
                .accelerator("CmdOrCtrl+,")
                .build(app)?,
        )
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        // Custom quit (not the predefined one) so Cmd+Q routes through the same
        // "stop running sessions?" confirmation as the tray and window-close.
        .item(
            &MenuItemBuilder::with_id("menu:quit", "Quit Agency")
                .accelerator("CmdOrCtrl+Q")
                .build(app)?,
        )
        .build()?;

    let file_menu = SubmenuBuilder::new(app, "File")
        // Disabled until a project is selected (see set_context) — spawning a
        // run needs a project to spawn it in.
        .item(
            &MenuItemBuilder::with_id("menu:new-agent", "New Agent")
                .accelerator("CmdOrCtrl+N")
                .enabled(false)
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("menu:new-terminal", "New Terminal")
                .accelerator("CmdOrCtrl+T")
                .enabled(false)
                .build(app)?,
        )
        .separator()
        // Always enabled: the daily and weekly notes live in the app-global
        // workspace, not the selected project (the frontend creates both on
        // demand).
        .item(
            &MenuItemBuilder::with_id("menu:daily-note", "Today's Note")
                .accelerator("CmdOrCtrl+Shift+D")
                .build(app)?,
        )
        .item(&MenuItemBuilder::with_id("menu:weekly-note", "Generate Weekly Note").build(app)?)
        .separator()
        .item(
            &MenuItemBuilder::with_id("menu:add-project", "Add Project…")
                .accelerator("CmdOrCtrl+Shift+O")
                .build(app)?,
        )
        .item(&MenuItemBuilder::with_id("menu:clone-project", "Clone Repository…").build(app)?)
        .separator()
        .close_window()
        .build()?;

    // Predefined so the OS provides native text editing inside inputs/editors.
    // Find is ours: the frontend routes it to whichever surface is on screen
    // and focused (notes, files, an issue description, the issue board's
    // filter, a terminal's scrollback) — see lib/findBus.ts. Claiming the
    // accelerator here is also what keeps CodeMirror's own search panel from
    // opening instead: on macOS the menu bar sees the key before the webview.
    //
    // Find Next/Previous take the Mac-standard ⌘G / ⇧⌘G, which every editor
    // binds — Source & Diff gave the key up and moved to ⌘D (see the View
    // menu). Enter/⇧Enter still step from inside the find field; the
    // accelerators are what keep the stepping going with the bar closed.
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .separator()
        .item(
            &MenuItemBuilder::with_id("menu:find", "Find…")
                .accelerator("CmdOrCtrl+F")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("menu:replace", "Find and Replace…")
                .accelerator("CmdOrCtrl+Alt+F")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("menu:find-next", "Find Next")
                .accelerator("CmdOrCtrl+G")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("menu:find-prev", "Find Previous")
                .accelerator("CmdOrCtrl+Shift+G")
                .build(app)?,
        )
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(
            &MenuItemBuilder::with_id("menu:palette", "Command Palette…")
                .accelerator("CmdOrCtrl+K")
                .build(app)?,
        )
        // Project-gated: the source/diff tab only exists inside a project.
        // ⌘D, not the ⌘G it used to hold — that key is Find Next everywhere
        // else on the platform, and Agency now honours that (see Edit).
        .item(
            &MenuItemBuilder::with_id("menu:source", "Source & Diff")
                .accelerator("CmdOrCtrl+D")
                .enabled(false)
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("menu:toggle-sidebar", "Toggle Sidebar")
                .accelerator("CmdOrCtrl+B")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("menu:home", "All Projects")
                .accelerator("CmdOrCtrl+Shift+H")
                .build(app)?,
        )
        .separator()
        .fullscreen()
        .build()?;

    // Every item acts on the focused agent, so the whole menu is disabled until
    // one is focused (see set_context) rather than silently doing nothing.
    let agent_menu = SubmenuBuilder::new(app, "Agent")
        .item(
            &MenuItemBuilder::with_id("menu:approve", "Approve & Merge")
                .accelerator("CmdOrCtrl+Return")
                .enabled(false)
                .build(app)?,
        )
        .separator()
        .item(&MenuItemBuilder::with_id("menu:archive", "Archive Agent").enabled(false).build(app)?)
        .item(&MenuItemBuilder::with_id("menu:discard", "Discard Agent").enabled(false).build(app)?)
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&MenuItemBuilder::with_id("menu:report-issue", "Report an Issue…").build(app)?)
        .item(&MenuItemBuilder::with_id("menu:github", "Agency on GitHub").build(app)?)
        .build()?;

    MenuBuilder::new(app)
        .item(&app_menu)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&agent_menu)
        .item(&window_menu)
        .item(&help_menu)
        .build()
}

/// Handle a click on any application-menu item. Registered globally via
/// `Builder::on_menu_event`. Tray-menu clicks go through `tray::on_menu_event`
/// instead, so the id namespaces (`menu:` vs bare `open`/`quit`/`run:`) never
/// collide.
pub fn on_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let Some(action) = event.id().as_ref().strip_prefix("menu:") else {
        return;
    };

    // Quit routes through the shared confirmation flow, same as the tray.
    if action == "quit" {
        crate::lifecycle::request_quit(app);
        return;
    }

    // Every remaining action drives the UI, so surface the window first (it may
    // be hidden in the menu bar) before telling the frontend what to do.
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    let _ = app.emit("menu", action);
}

/// Enable/disable the context-dependent items to match the frontend selection:
/// `project` gates items that need a selected project, `focused_agent` gates the
/// Agent menu. Must run on the main thread (menu mutation is main-thread-only on
/// macOS); the `set_menu_context` command dispatches it there.
pub fn set_context(app: &AppHandle, project: bool, focused_agent: bool) {
    let Some(menu) = app.menu() else { return };
    set_enabled(&menu, &["menu:new-agent", "menu:new-terminal", "menu:source"], project);
    set_enabled(&menu, &["menu:approve", "menu:archive", "menu:discard"], focused_agent);
}

/// Set `enabled` on every item whose id is in `ids`. Items live one level deep
/// (inside their submenu), so walk the top-level submenus to find them.
fn set_enabled(menu: &Menu<Wry>, ids: &[&str], enabled: bool) {
    let Ok(items) = menu.items() else { return };
    for kind in items {
        let Some(submenu) = kind.as_submenu() else { continue };
        for id in ids {
            if let Some(item) = submenu.get(*id).and_then(|k| k.as_menuitem().cloned()) {
                let _ = item.set_enabled(enabled);
            }
        }
    }
}
