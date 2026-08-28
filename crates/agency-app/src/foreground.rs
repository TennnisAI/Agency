//! Whether Agency is the application the user is in front of.
//!
//! Not the same question as "does the webview have keyboard focus", which is
//! what the frontend reports through `set_ui_state` (`document.hasFocus()`).
//! A window loses key status — and the document its focus — to things that
//! never take the user out of the app: a native menu tracking, a panel, an
//! open dialog, a notification banner. AGE-166 is what that costs: a
//! notification posted during one of those blurs armed the deep-link as if
//! Agency were in the background, and the user's next click anywhere in the
//! app, which handed the webview its focus back, read as a return from the
//! background and teleported them to the agent that had notified.
//!
//! `NSRunningApplication` answers the real question, and is documented thread
//! safe, so the notifier threads can ask it directly instead of hopping to the
//! main thread.

/// Is Agency frontmost? `None` where there is nothing to ask: only macOS is
/// wired up, so other platforms keep their own answer (see [`in_front`]).
#[cfg(target_os = "macos")]
pub fn app_is_active() -> Option<bool> {
    Some(objc2_app_kit::NSRunningApplication::currentApplication().isActive())
}

#[cfg(not(target_os = "macos"))]
pub fn app_is_active() -> Option<bool> {
    None
}

/// Whether the user is in Agency right now: the OS's answer where there is
/// one, and `reported` — the webview's `document.hasFocus()`, as last sent
/// through `set_ui_state` — where there is not.
pub fn in_front(reported: bool) -> bool {
    app_is_active().unwrap_or(reported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the module is that it asks somebody other than the
    /// webview. A build where nothing answers (a renamed class, a dropped
    /// feature) would silently fall back to the focus bit this exists to stop
    /// trusting, so assert there is an answer at all.
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_has_an_answer() {
        assert!(app_is_active().is_some());
    }

    #[test]
    fn the_reported_bit_is_the_fallback() {
        // Whatever the platform says, `in_front` must never invent an answer
        // where there is none.
        match app_is_active() {
            Some(active) => {
                assert_eq!(in_front(true), active);
                assert_eq!(in_front(false), active);
            }
            None => {
                assert!(in_front(true));
                assert!(!in_front(false));
            }
        }
    }
}
