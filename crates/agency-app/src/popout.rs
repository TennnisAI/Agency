//! Popped-out windows (AGE-252): one agent, file or note in a window of its
//! own, with a way back into the main window from either side.
//!
//! The window is a second webview on the same frontend bundle. It asks for its
//! own target by label (`popout_self`) rather than carrying it in the URL, so
//! the only thing a popout can show is what the main window registered here,
//! and what can be registered is an allowlist of three exact shapes with their
//! ids and paths checked (`Target::validate`).
//!
//! Exactly one window shows a popped-out item. The main window draws a
//! placeholder in its place for as long as the registry lists it, which is
//! what keeps two windows from typing into one file or fighting over one
//! terminal's size.
//!
//! The pure parts (validation, labels, the registry, terminal ownership) are
//! here and unit-tested; the commands at the bottom are the thin layer that
//! opens, focuses and closes the windows.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// Every popout window's label starts with this. The capability that grants
/// the window API to popouts matches on it (`capabilities/default.json`).
pub const LABEL_PREFIX: &str = "popout-";

/// The main window's label (the one `tauri.conf.json` declares).
pub const MAIN: &str = "main";

/// Which tree a file or note is read from: a run's worktree or a project's own
/// checkout. Mirrors the frontend's `FileRoot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Root {
    Run { id: String },
    Project { id: String },
}

/// What a popout shows. Three shapes and nothing else: an unknown `kind` or a
/// stray field fails to deserialize, so a payload cannot smuggle in a URL or a
/// window option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Target {
    /// An agent (or a plain terminal run), by the run's id.
    #[serde(rename_all = "camelCase")]
    Run { project_id: String, run_id: String },
    /// A file in the Files tab, by its path relative to `root`.
    #[serde(rename_all = "camelCase")]
    File { project_id: String, root: Root, path: String },
    /// A note in the Docs tab, by its path relative to the docs folder, which
    /// is itself relative to `root` ("" when the root is the docs folder).
    #[serde(rename_all = "camelCase")]
    Note { project_id: String, root: Root, docs_dir: String, path: String },
}

const MAX_ID: usize = 128;
const MAX_PATH: usize = 4096;
const MAX_TITLE: usize = 200;

/// Ids are UUIDs, and a run's extra sessions are `<uuid>--<n>`; anything else
/// is not an id the app made.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A path the file commands would resolve under their root: relative, with no
/// way out of it. The file commands check this again on every read and write;
/// this is so a popout never even names a path outside the tree.
fn valid_rel_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH
        && !path.contains('\0')
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.split(['/', '\\']).any(|seg| seg == "..")
}

impl Root {
    fn validate(&self) -> Result<(), String> {
        let (Root::Run { id } | Root::Project { id }) = self;
        if valid_id(id) {
            Ok(())
        } else {
            Err(format!("not a valid root id: {id:?}"))
        }
    }
}

impl Target {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Target::Run { project_id, run_id } => {
                if !valid_id(project_id) || !valid_id(run_id) {
                    return Err("not a valid agent to pop out".into());
                }
            }
            Target::File { project_id, root, path }
            | Target::Note { project_id, root, path, .. } => {
                if !valid_id(project_id) {
                    return Err("not a valid project to pop out from".into());
                }
                root.validate()?;
                if !valid_rel_path(path) {
                    return Err(format!("not a path that can be popped out: {path:?}"));
                }
            }
        }
        if let Target::Note { docs_dir, .. } = self {
            if !docs_dir.is_empty() && !valid_rel_path(docs_dir) {
                return Err(format!("not a docs folder that can be popped out: {docs_dir:?}"));
            }
        }
        Ok(())
    }

    /// Whether two targets put the same thing on screen. A run is its id. A
    /// file and a note are one file when they share a root and a path from it:
    /// a note is a markdown file in the docs folder, and the Files tab can
    /// open it too. Without this, popping a note out and then the same file
    /// from the Files tab gave two windows editing one file.
    pub fn same_place(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::Run { run_id: a, .. }, Target::Run { run_id: b, .. }) => a == b,
            (Target::Run { .. }, _) | (_, Target::Run { .. }) => false,
            _ => self.root() == other.root() && self.root_path() == other.root_path(),
        }
    }

    fn root(&self) -> Option<&Root> {
        match self {
            Target::Run { .. } => None,
            Target::File { root, .. } | Target::Note { root, .. } => Some(root),
        }
    }

    /// The path from the root: a note's sits under the docs folder.
    fn root_path(&self) -> Option<String> {
        match self {
            Target::Run { .. } => None,
            Target::File { path, .. } => Some(path.clone()),
            Target::Note { docs_dir, path, .. } if docs_dir.is_empty() => Some(path.clone()),
            Target::Note { docs_dir, path, .. } => {
                Some(format!("{}/{path}", docs_dir.trim_end_matches('/')))
            }
        }
    }

    /// The window label for this target. Deterministic, so popping out
    /// something that already has a window finds that window instead of
    /// opening a second one on it. A hash rather than the ids themselves:
    /// labels allow only `[A-Za-z0-9-/:_]`, and a path does not fit that.
    pub fn label(&self) -> String {
        // serde_json writes fields in declaration order, so this is canonical.
        let key = serde_json::to_string(self).unwrap_or_default();
        format!("{LABEL_PREFIX}{:016x}", fnv1a(key.as_bytes()))
    }
}

/// FNV-1a, 64-bit. Not for security: the label only has to be stable across
/// runs and distinct between the handful of windows a person has open.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The window title, from the caption the frontend proposes. Control
/// characters out, length capped, and never empty.
pub fn window_title(proposed: &str) -> String {
    let clean: String = proposed.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "Agency".to_string()
    } else {
        clean.to_string()
    }
}

/// One popped-out item, as the main window lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub label: String,
    pub target: Target,
}

/// What a popout window is handed when it asks what it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub target: Target,
    /// Unsaved edits the main window had in that file when it was popped out.
    /// Handed over once: the first ask takes it.
    pub draft: Option<String>,
}

/// An item coming back to the main window: the payload of the
/// `popout-reattached` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reattached {
    pub label: String,
    pub target: Target,
    /// Unsaved edits the popout had, for the main window to keep.
    pub draft: Option<String>,
    /// Bring the item up in the main window. False when the popout was simply
    /// closed: the item goes back where it came from without taking the main
    /// window somewhere the user did not ask to go.
    pub show: bool,
}

struct Entry {
    target: Target,
    draft: Option<String>,
    /// The window's page has asked what it shows, so it is running and
    /// listening for "Bring back". Until then only this side can hand the item
    /// back, and the draft is still here to hand back with it.
    greeted: bool,
}

/// A popout's entry, removed: what goes back to the main window.
#[derive(Debug, PartialEq, Eq)]
pub struct Closed {
    pub target: Target,
    /// The draft the window never collected (it never loaded), if any.
    pub draft: Option<String>,
}

/// The open popouts, by window label.
#[derive(Default)]
pub struct Popouts {
    open: BTreeMap<String, Entry>,
}

/// The result of asking for a popout.
#[derive(Debug, PartialEq, Eq)]
pub struct Opened {
    pub label: String,
    /// False when this target already had a window, which is to be focused
    /// rather than created.
    pub fresh: bool,
}

impl Popouts {
    pub fn open(&mut self, target: Target, draft: Option<String>) -> Opened {
        if let Some((label, _)) = self.open.iter().find(|(_, e)| e.target.same_place(&target)) {
            return Opened { label: label.clone(), fresh: false };
        }
        let label = target.label();
        self.open.insert(label.clone(), Entry { target, draft, greeted: false });
        Opened { label, fresh: true }
    }

    /// The window's target, taking the draft with it.
    pub fn hello(&mut self, label: &str) -> Option<Hello> {
        let e = self.open.get_mut(label)?;
        e.greeted = true;
        Some(Hello { target: e.target.clone(), draft: e.draft.take() })
    }

    /// Whether the window has loaded and asked what it shows. None when the
    /// label is not open.
    pub fn greeted(&self, label: &str) -> Option<bool> {
        self.open.get(label).map(|e| e.greeted)
    }

    pub fn close(&mut self, label: &str) -> Option<Closed> {
        self.open.remove(label).map(|e| Closed { target: e.target, draft: e.draft })
    }

    pub fn list(&self) -> Vec<Listed> {
        self.open
            .iter()
            .map(|(label, e)| Listed { label: label.clone(), target: e.target.clone() })
            .collect()
    }
}

/// Something on screen in exactly one window at a time: a terminal stream it
/// is attached to, or a run's preview pane.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Stream {
    Agent(String),
    Script {
        target: String,
        script: String,
    },
    Shell(String),
    /// The Run tab's preview pane, by run id. The screenshot tool photographs
    /// the window that reported the pane's rect (`preview_shot.rs`); a popped-
    /// out agent's Run tab is in the popout, and photographing the main window
    /// at the popout's coordinates returned a picture of whatever was there.
    Preview(String),
}

/// Which window holds each terminal attach, and each preview pane.
///
/// The backend keeps one attach per session, and a detach drops whatever is
/// there. With one window that was fine. With two, handing a terminal back is
/// a race: the main window attaches as the popout goes, and a detach from the
/// popout that lands after that attach would cut the main window's stream and
/// leave its pane frozen. So a detach only counts from the window that made
/// the attach it would drop.
#[derive(Default)]
pub struct Owners {
    by_stream: HashMap<Stream, String>,
}

impl Owners {
    pub fn attached(&mut self, stream: Stream, owner: &str) {
        self.by_stream.insert(stream, owner.to_string());
    }

    /// Whether `owner` may drop `stream`'s attach. True for the window that
    /// holds it, and for a stream nobody is recorded against (an attach made
    /// before this existed, or one already released). Forgets the stream when
    /// it says yes.
    pub fn detach(&mut self, stream: &Stream, owner: &str) -> bool {
        match self.by_stream.get(stream) {
            Some(o) if o != owner => false,
            _ => {
                self.by_stream.remove(stream);
                true
            }
        }
    }

    pub fn owner(&self, stream: &Stream) -> Option<&str> {
        self.by_stream.get(stream).map(String::as_str)
    }

    /// Everything `owner` holds, forgotten: for a window that went away
    /// without detaching, which closing a webview does.
    pub fn release(&mut self, owner: &str) -> Vec<Stream> {
        let gone: Vec<Stream> =
            self.by_stream.iter().filter(|(_, o)| *o == owner).map(|(s, _)| s.clone()).collect();
        for s in &gone {
            self.by_stream.remove(s);
        }
        gone
    }
}

/// What each window last said about the user's attention: whether its page has
/// focus, and the run whose pane it has on screen.
///
/// The backend's notion of "watching" was one `(focused, run)` pair, written by
/// whichever window spoke last. With a popout that is two writers racing. The
/// main window reports on every change of its on-screen run, focused or not,
/// so it overwrote the popout's run with its own and the agent the user was
/// watching in the popout notified. And switching windows sends the old
/// window's blur and the new one's focus from two pages, in either order: the
/// blur landing last left the app looking unfocused while the user was in it.
/// So each window reports its own pair, and the backend sees the summary.
#[derive(Default)]
pub struct Watching {
    by_window: HashMap<String, (bool, Option<String>)>,
    /// The window most recently reported focused. Its run is the one on screen
    /// while no page has focus (a native menu or a banner blurred it): the
    /// window has not gone anywhere.
    last_focused: Option<String>,
}

impl Watching {
    pub fn report(
        &mut self,
        window: &str,
        focused: bool,
        run: Option<String>,
    ) -> (bool, Option<String>) {
        if focused {
            self.last_focused = Some(window.to_string());
        }
        self.by_window.insert(window.to_string(), (focused, run));
        self.summary()
    }

    /// A window that went away: it holds no focus and shows no run.
    pub fn forget(&mut self, window: &str) -> (bool, Option<String>) {
        self.by_window.remove(window);
        if self.last_focused.as_deref() == Some(window) {
            self.last_focused = None;
        }
        self.summary()
    }

    /// Focused if any window is. The run is the focused window's, else the
    /// last focused one's, else the main window's.
    pub fn summary(&self) -> (bool, Option<String>) {
        let focused = self.by_window.iter().find(|(_, (f, _))| *f).map(|(w, _)| w.as_str());
        let showing = focused.or(self.last_focused.as_deref()).unwrap_or(MAIN);
        let run = self.by_window.get(showing).and_then(|(_, r)| r.clone());
        (focused.is_some(), run)
    }
}

/// Managed state: the popouts, the terminal owners and each window's report of
/// what it shows, each behind its lock.
#[derive(Default)]
pub struct Registry {
    pub popouts: Mutex<Popouts>,
    pub owners: Mutex<Owners>,
    pub watching: Mutex<Watching>,
}

pub fn is_popout(label: &str) -> bool {
    label.starts_with(LABEL_PREFIX)
}

fn announce(app: &AppHandle) {
    let _ = app.emit_to(MAIN, "popouts-changed", ());
}

/// Open `target` in a window of its own, or focus the one it already has.
/// Async because building a window from a synchronous command deadlocks on
/// some platforms.
#[tauri::command]
pub async fn pop_out(
    app: AppHandle,
    reg: State<'_, Registry>,
    target: Target,
    title: String,
    draft: Option<String>,
) -> Result<String, String> {
    target.validate()?;
    let opened = reg.popouts.lock().unwrap().open(target, draft);
    if !opened.fresh {
        focus(&app, &opened.label);
        return Ok(opened.label);
    }
    let builder =
        WebviewWindowBuilder::new(&app, &opened.label, WebviewUrl::App("index.html".into()))
            .title(window_title(&title))
            .inner_size(980.0, 720.0)
            .min_inner_size(420.0, 280.0);
    // The same chrome as the main window (tauri.conf.json): the macOS traffic
    // lights over the app's own title bar, and on Linux no decorations at all,
    // because the frontend draws the window controls.
    #[cfg(target_os = "macos")]
    let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    #[cfg(target_os = "linux")]
    let builder = builder.decorations(false);
    if let Err(e) = builder.build() {
        reg.popouts.lock().unwrap().close(&opened.label);
        return Err(format!("couldn't open the window: {e}"));
    }
    announce(&app);
    Ok(opened.label)
}

/// What the calling popout window shows. None for any other window, and for a
/// popout whose item has already gone back.
#[tauri::command]
pub fn popout_self(webview: tauri::Webview, reg: State<'_, Registry>) -> Option<Hello> {
    reg.popouts.lock().unwrap().hello(webview.label())
}

#[tauri::command]
pub fn list_popouts(reg: State<'_, Registry>) -> Vec<Listed> {
    reg.popouts.lock().unwrap().list()
}

/// Put a popped-out item back in the main window and close its window.
#[tauri::command]
pub fn reattach_popout(
    app: AppHandle,
    reg: State<'_, Registry>,
    label: String,
    draft: Option<String>,
    show: bool,
) {
    hand_back(&app, &reg, &label, draft, show);
}

/// Close `label`'s entry, send its item to the main window with `draft` (or
/// the draft the window never collected), and destroy the window.
fn hand_back(app: &AppHandle, reg: &Registry, label: &str, draft: Option<String>, show: bool) {
    let closed = reg.popouts.lock().unwrap().close(label);
    if let Some(Closed { target, draft: uncollected }) = closed {
        let draft = draft.or(uncollected);
        let _ = app.emit_to(
            MAIN,
            "popout-reattached",
            Reattached { label: label.to_string(), target, draft, show },
        );
        if show {
            crate::tray::show_main(app);
        }
        announce(app);
    }
    // `destroy`, not `close`: close asks the window's own close handler, which
    // in a popout is the thing that called this.
    if let Some(w) = app.get_webview_window(label).filter(|_| is_popout(label)) {
        let _ = w.destroy();
    }
}

/// The main window's "Bring back" button. A popout whose page is running does
/// the handing back itself, because it holds what has to come with it (a
/// file's unsaved edits); this only asks it to. One whose page never loaded
/// cannot answer, and is handed back from here with the draft it never
/// collected.
///
/// This used to be a timer in the main window: no answer in two seconds, and
/// it closed the popout itself, with no draft. A popout that was merely slow
/// (a throttled, minimized window) lost its unsaved edits silently, its own
/// answer arriving to find the entry already gone.
#[tauri::command]
pub fn request_reattach(app: AppHandle, reg: State<'_, Registry>, label: String) {
    if !is_popout(&label) {
        return;
    }
    let alive = app.get_webview_window(&label).is_some();
    let greeted = reg.popouts.lock().unwrap().greeted(&label).unwrap_or(false);
    if alive && greeted {
        let _ = app.emit_to(label.as_str(), "popout-reattach-request", ());
    } else {
        hand_back(&app, &reg, &label, None, true);
    }
}

#[tauri::command]
pub fn focus_popout(app: AppHandle, label: String) {
    if is_popout(&label) {
        focus(&app, &label);
    }
}

/// Bring the main window up and hand it a navigation the popout cannot serve
/// itself (a wikilink to another note, an issue chip, Approve). The main
/// window's own router decides what the payload means and ignores what it
/// does not know.
#[tauri::command]
pub fn navigate_main(app: AppHandle, target: serde_json::Value) {
    crate::tray::show_main(&app);
    let _ = app.emit_to(MAIN, "popout-navigate", target);
}

fn focus(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// A popout window is gone (closed, or crashed) without handing its item back.
/// Send the item home quietly, drop the terminal attaches it never got to
/// detach, and stop counting it as a window the user might be watching.
pub fn on_destroyed(app: &AppHandle, label: &str) {
    let Some(reg) = app.try_state::<Registry>() else { return };
    let closed = reg.popouts.lock().unwrap().close(label);
    let streams = reg.owners.lock().unwrap().release(label);
    let (focused, run) = reg.watching.lock().unwrap().forget(label);
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        // Forgetting a window only ever takes focus away, so this is never a
        // return to the app and has no notification to open.
        let _ = state.set_ui_state(focused, run);
        for s in streams {
            match s {
                Stream::Agent(id) => state.detach_run(&id),
                Stream::Script { target, script } => state.detach_run_script(&target, &script),
                Stream::Shell(id) => state.detach_shell(&id),
                Stream::Preview(run_id) => state.set_preview_rect(&run_id, None),
            }
        }
    }
    if let Some(Closed { target, draft }) = closed {
        let _ = app.emit_to(
            MAIN,
            "popout-reattached",
            Reattached { label: label.to_string(), target, draft, show: false },
        );
        announce(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str) -> Target {
        Target::Run { project_id: "p1".into(), run_id: id.into() }
    }

    fn file(path: &str) -> Target {
        Target::File {
            project_id: "p1".into(),
            root: Root::Project { id: "p1".into() },
            path: path.into(),
        }
    }

    fn note(docs_dir: &str, path: &str) -> Target {
        Target::Note {
            project_id: "p1".into(),
            root: Root::Project { id: "p1".into() },
            docs_dir: docs_dir.into(),
            path: path.into(),
        }
    }

    #[test]
    fn targets_deserialize_from_the_frontend_shape() {
        let t: Target = serde_json::from_str(
            r#"{"kind":"file","projectId":"p1","root":{"kind":"run","id":"r-1--2"},"path":"src/a.rs"}"#,
        )
        .unwrap();
        assert_eq!(
            t,
            Target::File {
                project_id: "p1".into(),
                root: Root::Run { id: "r-1--2".into() },
                path: "src/a.rs".into()
            }
        );
        assert!(t.validate().is_ok());
    }

    #[test]
    fn unknown_kinds_and_stray_fields_are_refused() {
        assert!(serde_json::from_str::<Target>(r#"{"kind":"url","projectId":"p1"}"#).is_err());
        assert!(serde_json::from_str::<Target>(
            r#"{"kind":"run","projectId":"p1","runId":"r1","url":"https://x"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<Target>(
            r#"{"kind":"note","projectId":"p1","root":{"kind":"disk","id":"x"},"path":"a.md"}"#
        )
        .is_err());
    }

    #[test]
    fn ids_must_look_like_ids() {
        assert!(run("3f2a-uuid--2").validate().is_ok());
        assert!(run("").validate().is_err());
        assert!(run("a/b").validate().is_err());
        assert!(run("a b").validate().is_err());
        assert!(run(&"x".repeat(MAX_ID + 1)).validate().is_err());
    }

    #[test]
    fn paths_stay_inside_their_root() {
        assert!(file("docs/Notes/a b.md").validate().is_ok());
        assert!(file(".agency/issues/AGE-1.md").validate().is_ok());
        assert!(file("").validate().is_err());
        assert!(file("/etc/passwd").validate().is_err());
        assert!(file("\\windows").validate().is_err());
        assert!(file("../outside").validate().is_err());
        assert!(file("a/../../b").validate().is_err());
        assert!(file("a\\..\\b").validate().is_err());
        assert!(file("a\0b").validate().is_err());
        // A name that merely contains dots is a name.
        assert!(file("a/..b/c..").validate().is_ok());
        // A note's docs folder is held to the same rule, and "" is the root.
        assert!(note("", "a.md").validate().is_ok());
        assert!(note("docs/notes", "a.md").validate().is_ok());
        assert!(note("../docs", "a.md").validate().is_err());
        assert!(note("/docs", "a.md").validate().is_err());
    }

    #[test]
    fn a_note_and_the_file_it_is_are_one_window() {
        let mut p = Popouts::default();
        let first = p.open(note("docs", "Plan.md"), None);
        assert!(first.fresh);
        // The same file from the Files tab finds the note's window.
        assert_eq!(p.open(file("docs/Plan.md"), None), Opened { label: first.label, fresh: false });
        assert!(note("docs/", "Plan.md").same_place(&file("docs/Plan.md")));
        assert!(note("", "Plan.md").same_place(&file("Plan.md")));
        assert!(!note("docs", "Plan.md").same_place(&file("Plan.md")));
        // Another tree's copy is another file.
        let in_run = Target::File {
            project_id: "p1".into(),
            root: Root::Run { id: "r1".into() },
            path: "docs/Plan.md".into(),
        };
        assert!(!note("docs", "Plan.md").same_place(&in_run));
        // A run is never a file, and is itself whatever project it is reached from.
        assert!(!run("r1").same_place(&file("r1")));
        assert!(run("r1").same_place(&Target::Run { project_id: "p2".into(), run_id: "r1".into() }));
    }

    #[test]
    fn labels_are_stable_distinct_and_legal() {
        let a = run("r1").label();
        assert_eq!(a, run("r1").label());
        assert_ne!(a, run("r2").label());
        // The same path in a different tree is a different window.
        let in_run = Target::File {
            project_id: "p1".into(),
            root: Root::Run { id: "p1".into() },
            path: "a.rs".into(),
        };
        assert_ne!(file("a.rs").label(), in_run.label());
        // A note and a file at the same path are different windows too.
        let note = note("", "a.rs");
        assert_ne!(file("a.rs").label(), note.label());
        assert!(a.starts_with(LABEL_PREFIX));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert!(is_popout(&a));
        assert!(!is_popout(MAIN));
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(window_title("  claude · fix the login \n"), "claude · fix the login");
        assert_eq!(window_title("a\u{7}b"), "ab");
        assert_eq!(window_title("   "), "Agency");
        assert_eq!(window_title(&"x".repeat(500)).chars().count(), MAX_TITLE);
    }

    #[test]
    fn popping_out_twice_finds_the_first_window() {
        let mut p = Popouts::default();
        let first = p.open(run("r1"), None);
        assert!(first.fresh);
        let again = p.open(run("r1"), None);
        assert_eq!(again, Opened { label: first.label.clone(), fresh: false });
        assert_eq!(p.list().len(), 1);
    }

    #[test]
    fn the_draft_is_handed_over_once() {
        let mut p = Popouts::default();
        let o = p.open(file("a.rs"), Some("unsaved".into()));
        assert_eq!(p.hello(&o.label).unwrap().draft.as_deref(), Some("unsaved"));
        // A reload of the popout must not resurrect edits it has since saved.
        let again = p.hello(&o.label).unwrap();
        assert_eq!(again.draft, None);
        assert_eq!(again.target, file("a.rs"));
        assert!(p.hello("popout-nope").is_none());
    }

    #[test]
    fn closing_returns_the_target_once() {
        let mut p = Popouts::default();
        let o = p.open(run("r1"), None);
        assert_eq!(p.list(), vec![Listed { label: o.label.clone(), target: run("r1") }]);
        assert_eq!(p.close(&o.label), Some(Closed { target: run("r1"), draft: None }));
        assert_eq!(p.close(&o.label), None);
        assert!(p.list().is_empty());
    }

    #[test]
    fn a_window_that_never_loaded_hands_back_the_draft_it_never_took() {
        let mut p = Popouts::default();
        let o = p.open(file("a.rs"), Some("unsaved".into()));
        // Not loaded yet: only the main window can bring it back, and the
        // edits are still here to go with it.
        assert_eq!(p.greeted(&o.label), Some(false));
        assert_eq!(p.close(&o.label).unwrap().draft.as_deref(), Some("unsaved"));

        let o = p.open(file("a.rs"), Some("unsaved".into()));
        p.hello(&o.label);
        // Loaded: the window holds the edits now, and answers for itself.
        assert_eq!(p.greeted(&o.label), Some(true));
        assert_eq!(p.close(&o.label).unwrap().draft, None);
        assert_eq!(p.greeted(&o.label), None);
    }

    #[test]
    fn the_window_in_front_decides_which_run_is_watched() {
        let mut w = Watching::default();
        w.report(MAIN, true, Some("main-run".into()));
        // The user moves to a popout. Its focus lands; the main window's blur
        // has not yet.
        assert_eq!(w.report("popout-1", true, Some("out-run".into())).0, true);
        // The main window's blur, and a change of its on-screen run while it
        // sits behind: neither takes the popout's run off the watched one.
        w.report(MAIN, false, Some("main-run".into()));
        assert_eq!(w.report(MAIN, false, Some("other".into())), (true, Some("out-run".into())));
    }

    #[test]
    fn a_blur_landing_after_the_next_focus_does_not_unfocus_the_app() {
        let mut w = Watching::default();
        w.report("popout-1", true, Some("out-run".into()));
        // Back to the main window: its focus first, the popout's blur second.
        w.report(MAIN, true, Some("main-run".into()));
        assert_eq!(
            w.report("popout-1", false, Some("out-run".into())),
            (true, Some("main-run".into()))
        );
    }

    #[test]
    fn with_no_page_focused_the_last_focused_window_is_still_on_screen() {
        let mut w = Watching::default();
        w.report(MAIN, false, Some("main-run".into()));
        w.report("popout-1", true, Some("out-run".into()));
        // A native menu blurs the popout's page; the popout is still in front.
        assert_eq!(
            w.report("popout-1", false, Some("out-run".into())),
            (false, Some("out-run".into()))
        );
        // The popout closes: the main window is what is left.
        assert_eq!(w.forget("popout-1"), (false, Some("main-run".into())));
    }

    #[test]
    fn a_detach_only_counts_from_the_window_that_attached() {
        let mut o = Owners::default();
        let s = Stream::Agent("r1".into());
        o.attached(s.clone(), "popout-1");
        // The popout is handing the terminal back; the main window attaches
        // before the popout's detach lands.
        o.attached(s.clone(), MAIN);
        assert!(!o.detach(&s, "popout-1"));
        assert_eq!(o.owner(&s), Some(MAIN));
        assert!(o.detach(&s, MAIN));
        assert_eq!(o.owner(&s), None);
        // Nobody recorded: an attach from before the record existed.
        assert!(o.detach(&s, "popout-1"));
    }

    #[test]
    fn a_closed_window_releases_what_it_held() {
        let mut o = Owners::default();
        o.attached(Stream::Agent("r1".into()), "popout-1");
        o.attached(Stream::Shell("r1".into()), "popout-1");
        o.attached(Stream::Script { target: "r2".into(), script: "dev".into() }, MAIN);
        let mut gone = o.release("popout-1");
        gone.sort_by_key(|s| format!("{s:?}"));
        assert_eq!(gone, vec![Stream::Agent("r1".into()), Stream::Shell("r1".into())]);
        assert!(o.release("popout-1").is_empty());
        // The main window's attach is untouched.
        assert!(!o.detach(&Stream::Script { target: "r2".into(), script: "dev".into() }, "x"));
    }
}
