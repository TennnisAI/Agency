//! Drop the WebKit context-menu items that cannot do anything in an Agency
//! window, so the previews that need a frame stop offering a dead menu
//! (AGE-158).
//!
//! Right-clicking inside an `<iframe>` gets WebKit's menu, never ours: a frame
//! is a separate document, and the app's `contextmenu` suppression in
//! `main.tsx` listens on its own window, so the event is not reachable from
//! any JS the app can run — the `.html` preview is sandboxed against scripts
//! and the localhost preview is cross-origin. Three previews genuinely need a
//! frame: an `.html` file (sandboxed, so design-handoff HTML renders itself
//! without same-origin or IPC access), a `.pdf` (WebKit's PDF viewer only
//! exists in a frame), and the Run tab's `http://localhost:*` dev server. In
//! all three, WebKit's menu for a subframe is one item, "Open Frame in New
//! Window", and it is inert: wry asks the embedder to create that window and
//! Agency registers no new-window handler, so
//! `-createWebViewWithConfiguration:` answers nil and the click does nothing.
//! AGE-156 fixed the fourth case, the markdown preview, by rendering it into
//! the app's own document instead; these three cannot follow it.
//!
//! wry 0.55 can turn the default menu off for WebView2
//! (`with_default_context_menus`) but has nothing for WKWebView, so the fix is
//! native. AppKit sends `-[NSView willOpenMenu:withEvent:]` to the view a
//! context menu is popping up over — for WebKit's menu that is the WKWebView —
//! after the menu is built and before it is shown, which is exactly where an
//! item can be taken out. macOS does not display a menu with no items left,
//! and that is what turns the subframe's one-item menu into no menu at all.
//!
//! Deliberately narrow: only `INERT` goes, so a frame keeps every item that
//! does work (Copy out of a PDF, Copy Link, Look Up, Inspect Element), and the
//! menus Agency keeps native on purpose — text fields and terminals, where
//! copy and paste have to keep working — are untouched, because none of them
//! carries one of these identifiers.

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2_app_kit::{NSMenu, NSUserInterfaceItemIdentification};
use std::sync::{Once, OnceLock};

/// The WebKit menu items that cannot do anything here, by the
/// `NSMenuItem.identifier` WebKit stamps on each one. The strings are WebKit's
/// own `_WKMenuItemIdentifier*` constants, read out of the exported symbols in
/// `WebKit.framework` rather than transcribed from a header.
const INERT: &[&str] = &[
    // Every "open ... in a new window": no new-window handler is registered,
    // so wry returns nil for the request and nothing happens. The frame one is
    // the whole menu a subframe gets, which is what made this visible.
    "WKMenuItemIdentifierOpenFrameInNewWindow",
    "WKMenuItemIdentifierOpenImageInNewWindow",
    "WKMenuItemIdentifierOpenLinkInNewWindow",
    "WKMenuItemIdentifierOpenMediaInNewWindow",
    // Every download, the same shape one layer down: wry installs a
    // WKDownloadDelegate only when the embedder asks for download callbacks,
    // and Agency asks for none, so a download WebKit starts has nowhere to
    // land.
    "WKMenuItemIdentifierDownloadImage",
    "WKMenuItemIdentifierDownloadLinkedFile",
    "WKMenuItemIdentifierDownloadMedia",
];

/// As much of a menu item as the decision needs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item<'a> {
    Separator,
    /// A command, carrying WebKit's identifier for it (empty when it has none,
    /// which is every item AppKit itself contributes).
    Command(&'a str),
}

/// Which indices to take out of `items`, descending so the caller can remove
/// them one at a time without the earlier removals shifting the later ones.
///
/// Inert commands go, and with them any separator their removal left leading,
/// trailing or doubled — otherwise stripping "Open Link in New Window" and
/// "Download Linked File" out of a link menu leaves a rule against the top of
/// the menu.
fn to_remove(items: &[Item<'_>]) -> Vec<usize> {
    let mut keep: Vec<bool> =
        items.iter().map(|item| !matches!(item, Item::Command(id) if INERT.contains(id))).collect();
    // A menu with nothing inert in it is left exactly as WebKit built it,
    // separators and all. Only our own removals earn a tidy-up.
    if keep.iter().all(|k| *k) {
        return Vec::new();
    }
    let mut after_command = false;
    let mut trailing: Option<usize> = None;
    for (i, item) in items.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        match item {
            // A separator earns its place only between two surviving commands,
            // so one that follows anything else (the top of the menu, or
            // another separator) goes.
            Item::Separator => {
                if after_command {
                    trailing = Some(i);
                } else {
                    keep[i] = false;
                }
                after_command = false;
            }
            Item::Command(_) => {
                trailing = None;
                after_command = true;
            }
        }
    }
    if let Some(i) = trailing {
        keep[i] = false;
    }
    (0..items.len()).rev().filter(|&i| !keep[i]).collect()
}

/// Install the filter on WKWebView, once, at startup. It patches the class, so
/// it reaches the webviews Tauri has already built as well as any it builds
/// later; all it has to be is in place before a menu can open.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let cls: &AnyClass = <objc2_web_kit::WKWebView as objc2::ClassType>::class();
        let sel = objc2::sel!(willOpenMenu:withEvent:);
        // Whatever runs today, kept so the replacement can call through to it:
        // AppKit's own do-nothing NSView implementation as of macOS 26, or
        // WKWebView's if a later one grows one.
        if let Some(method) = cls.instance_method(sel) {
            let _ = ORIGINAL.set(method.implementation());
        }
        // Add the method to WKWebView rather than repointing the one we just
        // read: as of macOS 26 that one is NSView's, shared by every view in
        // the app, and `method_setImplementation` on it would route every
        // context menu in the process — a terminal's, a text field's —
        // through this code.
        let added = unsafe {
            objc2::ffi::class_addMethod(
                (cls as *const AnyClass).cast_mut(),
                sel,
                std::mem::transmute::<WillOpenMenu, Imp>(will_open_menu),
                c"v@:@@".as_ptr(),
            )
        };
        if added.as_bool() {
            return;
        }
        // WKWebView implements it itself, which is the only reason the add can
        // fail. Repointing is safe now precisely because the method found is
        // WKWebView's own and not the shared NSView one.
        match cls.instance_method(sel) {
            // SAFETY: the signature matches `v@:@@`, which is what the method
            // being replaced has, and the replacement handles a null menu.
            Some(method) => unsafe {
                method.set_implementation(std::mem::transmute::<WillOpenMenu, Imp>(will_open_menu));
            },
            None => log::warn!("could not filter the WebKit context menu: no method to replace"),
        }
    });
}

/// The signature of `-[NSView willOpenMenu:withEvent:]` (`v@:@@`): receiver,
/// selector, the menu, and the event, which is never read and so stays an
/// opaque object pointer.
type WillOpenMenu = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut NSMenu, *mut AnyObject);

/// The implementation `install` replaced, called through to first.
static ORIGINAL: OnceLock<Imp> = OnceLock::new();

/// `-[WKWebView willOpenMenu:withEvent:]`, as installed by [`install`].
unsafe extern "C-unwind" fn will_open_menu(
    this: *mut AnyObject,
    cmd: Sel,
    menu: *mut NSMenu,
    event: *mut AnyObject,
) {
    if let Some(&original) = ORIGINAL.get() {
        // SAFETY: `original` was read off this very selector, so it takes
        // these arguments, and they are passed on untouched.
        unsafe { std::mem::transmute::<Imp, WillOpenMenu>(original)(this, cmd, menu, event) };
    }
    // A panic must not unwind out of Rust and into AppKit; the menu opening is
    // worth less than the process.
    let filtered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: AppKit sends this on the main thread, which is where NSMenu
        // may be touched, and the menu is fully built by now — AppKit sends it
        // after the menu is assembled and before it is displayed.
        unsafe { filter(menu) }
    }));
    if filtered.is_err() {
        log::error!("filtering the WebKit context menu panicked");
    }
}

/// Take [`INERT`] out of `menu`.
unsafe fn filter(menu: *mut NSMenu) {
    // SAFETY: the caller passes AppKit's menu argument straight through, and a
    // null one is simply nothing to filter.
    let Some(menu) = (unsafe { menu.as_ref() }) else { return };
    let items = menu.itemArray();
    let described: Vec<(bool, String)> = items
        .iter()
        .map(|item| {
            (item.isSeparatorItem(), item.identifier().map(|id| id.to_string()).unwrap_or_default())
        })
        .collect();
    let model: Vec<Item<'_>> = described
        .iter()
        .map(|(separator, id)| if *separator { Item::Separator } else { Item::Command(id) })
        .collect();
    let remove = to_remove(&model);
    if remove.is_empty() {
        return;
    }
    log::debug!("dropping {} inert WebKit context menu item(s)", remove.len());
    for i in remove {
        menu.removeItemAtIndex(i as isize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Item = Item::Command("WKMenuItemIdentifierOpenFrameInNewWindow");
    const COPY: Item = Item::Command("WKMenuItemIdentifierCopy");

    #[test]
    fn the_filter_installs_on_wkwebview_itself() {
        install();
        // Directly on WKWebView, not inherited: an inherited match would mean
        // `class_addMethod` had failed and left NSView's own do-nothing
        // implementation answering, with the dead menu still on screen. This
        // is the half of the fix that is testable without a right-click.
        let cls = <objc2_web_kit::WKWebView as objc2::ClassType>::class();
        let sel = objc2::sel!(willOpenMenu:withEvent:);
        assert!(
            cls.instance_methods().iter().any(|m| m.name() == sel),
            "willOpenMenu:withEvent: is not implemented on WKWebView itself"
        );
        // Once means once: a second call must not stack a second filter.
        install();
    }

    #[test]
    fn a_subframes_only_item_leaves_an_empty_menu() {
        // The reported bug: right-clicking a PDF, an .html preview or the Run
        // tab's dev server offered this one item and it did nothing.
        assert_eq!(to_remove(&[FRAME]), vec![0]);
    }

    #[test]
    fn a_menu_with_nothing_inert_is_untouched() {
        let menu = [COPY, Item::Separator, Item::Command("WKMenuItemIdentifierLookUp")];
        assert!(to_remove(&menu).is_empty());
    }

    #[test]
    fn unknown_items_are_kept() {
        // Default-keep: WebKit and AppKit both add items we have never seen,
        // and AppKit's own (Services, Speech) carry no identifier at all.
        let menu = [Item::Command("WKMenuItemIdentifierSomethingNew"), Item::Command("")];
        assert!(to_remove(&menu).is_empty());
    }

    #[test]
    fn indices_come_back_descending() {
        // So the caller can remove them in order without re-indexing.
        let menu = [FRAME, COPY, FRAME];
        assert_eq!(to_remove(&menu), vec![2, 0]);
    }

    #[test]
    fn a_separator_left_leading_goes_too() {
        // A link menu stripped down to Copy Link should not open with a rule.
        let menu =
            [Item::Command("WKMenuItemIdentifierOpenLinkInNewWindow"), Item::Separator, COPY];
        assert_eq!(to_remove(&menu), vec![1, 0]);
    }

    #[test]
    fn a_separator_left_trailing_goes_too() {
        let menu = [COPY, Item::Separator, Item::Command("WKMenuItemIdentifierDownloadImage")];
        assert_eq!(to_remove(&menu), vec![2, 1]);
    }

    #[test]
    fn a_separator_left_doubled_collapses_to_one() {
        let menu = [COPY, Item::Separator, FRAME, Item::Separator, COPY];
        assert_eq!(to_remove(&menu), vec![3, 2]);
    }

    #[test]
    fn separators_alone_leave_an_empty_menu() {
        let menu = [Item::Separator, FRAME, Item::Separator];
        assert_eq!(to_remove(&menu), vec![2, 1, 0]);
    }
}
