//! Native pixel screenshots of the Run tab's preview pane, for the preview
//! MCP server's `preview_screenshot` tool (AGE-143).
//!
//! Why native at all: the bridge script cannot export pixels from inside the
//! page — WebKit taints any canvas that a foreignObject SVG (the only
//! DOM-to-image route a page has) is drawn into, so `toDataURL` throws. The
//! app, owning the window, can ask WKWebView itself: `takeSnapshot` renders
//! from the layer tree, which is exactly "the pane as the user sees it".
//!
//! The crop rect comes from the Run tab, which reports its preview iframe's
//! `getBoundingClientRect` while the pane is mounted (`set_preview_rect`).
//! That is also the tool's honest limit: an unmounted pane is not rendered
//! anywhere, so there is nothing to photograph and the tool says so —
//! `preview_snapshot` (DOM text, via the bridge) is the always-works sibling.

use crate::state::{AppState, PreviewRect, PreviewShotFn};
use std::sync::Arc;

/// The provider installed into [`AppState`] at startup: run id → PNG bytes of
/// that run's preview pane, or the reason there is no picture right now.
pub fn provider(handle: tauri::AppHandle) -> PreviewShotFn {
    Arc::new(move |run_id: &str| take(&handle, run_id))
}

const NOT_ON_SCREEN: &str = "The preview pane is not on screen in Agency right now, so there \
    is no picture to take. preview_snapshot describes the page without it, or ask the user to \
    open this run's Run tab.";

fn take(handle: &tauri::AppHandle, run_id: &str) -> Result<Vec<u8>, String> {
    use tauri::Manager;
    let Some(rect) = handle.state::<AppState>().preview_rect(run_id) else {
        return Err(NOT_ON_SCREEN.to_string());
    };
    // A collapsed pane (mid-resize, or hidden behind a zero-width split) has
    // nothing to show either.
    if rect.width < 16.0 || rect.height < 16.0 {
        return Err(NOT_ON_SCREEN.to_string());
    }
    shoot(handle, rect)
}

/// Ask WKWebView for a snapshot of `rect`, on the main thread, and wait here
/// (an MCP worker thread) for the completion handler.
#[cfg(target_os = "macos")]
fn shoot(handle: &tauri::AppHandle, rect: PreviewRect) -> Result<Vec<u8>, String> {
    use tauri::Manager;
    let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<u8>, String>>();
    let handle2 = handle.clone();
    handle
        .run_on_main_thread(move || {
            let Some(window) = handle2.get_webview_window("main") else {
                let _ = tx.send(Err("the app window is gone".to_string()));
                return;
            };
            let tx2 = tx.clone();
            let ran = window.with_webview(move |webview| {
                // SAFETY: on macOS `inner()` is the WKWebView backing the
                // window, and we are on the main thread (with_webview's
                // contract), which is where WebKit wants to be called.
                unsafe { snapshot_wkwebview(webview.inner().cast(), rect, tx2) }
            });
            if let Err(e) = ran {
                let _ = tx.send(Err(format!("the app webview is unavailable: {e}")));
            }
        })
        .map_err(|e| format!("the app's main thread is unavailable: {e}"))?;
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(result) => result,
        Err(_) => Err("the snapshot timed out; try again".to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
fn shoot(_handle: &tauri::AppHandle, _rect: PreviewRect) -> Result<Vec<u8>, String> {
    Err("preview screenshots are only implemented on macOS; preview_snapshot works everywhere"
        .to_string())
}

/// `-[WKWebView takeSnapshotWithConfiguration:completionHandler:]`, cropped to
/// `rect` (view coordinates, which for the window-filling webview are the CSS
/// pixels the frontend measured). The completion handler runs later on the
/// main thread and answers through `tx`.
#[cfg(target_os = "macos")]
unsafe fn snapshot_wkwebview(
    webview: *mut objc2_web_kit::WKWebView,
    rect: PreviewRect,
    tx: std::sync::mpsc::Sender<Result<Vec<u8>, String>>,
) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSImage;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::NSError;
    use objc2_web_kit::WKSnapshotConfiguration;

    let Some(webview) = webview.as_ref() else {
        let _ = tx.send(Err("the app webview is gone".to_string()));
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = tx.send(Err("snapshots must run on the main thread".to_string()));
        return;
    };
    let config = WKSnapshotConfiguration::new(mtm);
    config.setRect(CGRect {
        origin: CGPoint { x: rect.x, y: rect.y },
        size: CGSize { width: rect.width, height: rect.height },
    });
    let block = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let result = match image.as_ref() {
            Some(image) => png_bytes(image).ok_or_else(|| {
                "WebKit produced an image the app could not encode as PNG".to_string()
            }),
            None => {
                let why = error
                    .as_ref()
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "WebKit returned no image".to_string());
                Err(format!("the snapshot failed: {why}"))
            }
        };
        let _ = tx.send(result);
    });
    webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
}

/// NSImage → PNG via a bitmap rep, the classic AppKit route.
#[cfg(target_os = "macos")]
unsafe fn png_bytes(image: &objc2_app_kit::NSImage) -> Option<Vec<u8>> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::NSDictionary;

    let tiff = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let png =
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
    Some(png.to_vec())
}
