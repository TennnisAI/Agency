//! Reading the clipboard as *whole* text.
//!
//! macOS pasteboards can hold several items at once — copying a multi-row
//! selection out of an app like Xcode's issue navigator writes one item per
//! row. WebKit's paste path only ever reads item 0 (`Pasteboard::read` takes a
//! single index and defaults it to zero), so a paste inside the webview gets
//! the first row and silently drops the rest. Pasting the same clipboard into
//! a native app that reads every item — TextEdit, Terminal — yields all of it,
//! and re-copying from there collapses the pasteboard to one item, which is
//! why the round trip "fixes" it (AGE-41).
//!
//! So the frontend does not trust the webview's copy of the clipboard: it asks
//! here instead, and this reads every item and joins them with newlines, which
//! is the text the user selected.

/// Every plain-text item on the clipboard, newline-joined. `None` when the
/// clipboard holds no text at all (an image, say), which tells the caller to
/// fall back to whatever the webview handed it.
#[tauri::command]
pub fn read_clipboard_text() -> Option<String> {
    read_all()
}

#[cfg(target_os = "macos")]
fn read_all() -> Option<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};

    // NSPasteboard is documented as safe to use from any thread.
    let pasteboard = NSPasteboard::generalPasteboard();
    let items = pasteboard.pasteboardItems();
    let parts: Vec<String> = items
        .into_iter()
        .flatten()
        .filter_map(|item| unsafe { item.stringForType(NSPasteboardTypeString) })
        .map(|s| s.to_string())
        .collect();
    if !parts.is_empty() {
        return Some(parts.join("\n"));
    }
    // No per-item string: fall back to the whole-pasteboard read, which also
    // covers promised/lazy flavors an item enumeration can miss.
    unsafe { pasteboard.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string())
}

#[cfg(not(target_os = "macos"))]
fn read_all() -> Option<String> {
    // The multi-item truncation is a macOS pasteboard/WebKit behavior; elsewhere
    // the webview's own clipboard read is authoritative.
    None
}
