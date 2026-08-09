//! Letting notifications appear while Agency is the app in front.
//!
//! `notifier::suppressed` already decides that an agent finishing in a tab you
//! are not looking at should notify whether or not Agency is in front. macOS
//! has the last word, though: `NSUserNotificationCenter` asks its delegate
//! `shouldPresentNotification:` before showing a banner posted by the app that
//! is currently frontmost, and reads a missing method as "no". The backend
//! under tauri-plugin-notification (mac-notification-sys) implements the
//! delivered / activated / dismissed callbacks but not that one, so every
//! notification raised while the user had Agency open was filed silently into
//! Notification Center and never shown. Hence AGE-33: banners only ever
//! arrived when the app was in the background.
//!
//! A method cannot be added to someone else's ObjC class at compile time, so
//! it is added at runtime, to the delegate class mac-notification-sys links
//! into this binary. The rest of their delegate is left alone deliberately:
//! installing a delegate object of our own would strand the delivery callback
//! their send path waits on.

//! Only macOS needs this: other platforms' notification services show a
//! banner regardless of which app is in front. `lib.rs` gates the module.

use std::ffi::{c_char, c_void, CStr};

/// The delegate class mac-notification-sys compiles into this binary. Private
/// to that crate, so a dependency bump could rename it; the lookup below fails
/// loudly (a log line, and today's background-only behaviour) rather than
/// breaking anything if it ever does.
const DELEGATE_CLASS: &CStr = c"NotificationCenterDelegate";
const SELECTOR: &CStr = c"userNotificationCenter:shouldPresentNotification:";

/// ObjC type encoding for `-(BOOL)…:(id)…:(id)` — return, self, _cmd, two
/// object arguments. `BOOL` is a `_Bool` ("B") on arm64 and a `signed char`
/// ("c") on x86_64; both are a single byte carrying 0 or 1, which is what the
/// Rust `bool` below returns either way.
#[cfg(target_arch = "aarch64")]
const SIGNATURE: &CStr = c"B@:@@";
#[cfg(not(target_arch = "aarch64"))]
const SIGNATURE: &CStr = c"c@:@@";

type Id = *mut c_void;
type Sel = *const c_void;
type Imp = unsafe extern "C" fn();
type ShouldPresentFn = extern "C" fn(Id, Sel, Id, Id) -> bool;

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn class_getInstanceMethod(cls: *mut c_void, sel: Sel) -> *const c_void;
    fn class_replaceMethod(
        cls: *mut c_void,
        sel: Sel,
        imp: Imp,
        types: *const c_char,
    ) -> *const c_void;
}

/// Always present. Whether this particular run deserves a banner was settled
/// long before macOS asked — by `notifier::suppressed`, which stays quiet only
/// about the one run the user is actually watching. By the time a notification
/// has been posted at all, it is one they should see.
extern "C" fn should_present(_self: Id, _cmd: Sel, _center: Id, _notification: Id) -> bool {
    true
}

/// Teach the notification delegate to show banners while Agency is frontmost.
/// Call once at startup, before any notification is posted. Returns whether
/// the method was installed; a `false` means notifications keep working, just
/// only while the app is in the background.
pub fn present_while_frontmost() -> bool {
    // SAFETY: plain ObjC runtime calls. `objc_getClass` is null-checked, and
    // `should_present` matches the selector's signature exactly (self, _cmd,
    // then the protocol's two object arguments), so the runtime's call into it
    // is well-formed.
    unsafe {
        let cls = objc_getClass(DELEGATE_CLASS.as_ptr());
        if cls.is_null() {
            log::warn!(
                "notification delegate class {:?} not found; notifications will only appear \
                 while Agency is in the background",
                DELEGATE_CLASS
            );
            return false;
        }
        let sel = sel_registerName(SELECTOR.as_ptr());
        if !class_getInstanceMethod(cls, sel).is_null() {
            // The backend grew its own answer. Ours still wins (see the
            // suppression rules above), but it is worth knowing about.
            log::info!(
                "notification delegate already answers shouldPresentNotification:; overriding"
            );
        }
        class_replaceMethod(
            cls,
            sel,
            std::mem::transmute::<ShouldPresentFn, Imp>(should_present),
            SIGNATURE.as_ptr(),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole fix rests on a class name owned by a dependency. If a bump
    /// ever renames it, this fails here instead of silently swallowing every
    /// notification raised while the user has Agency open.
    #[test]
    fn installs_the_present_answer_on_the_backend_delegate() {
        assert!(present_while_frontmost(), "delegate class not found — see DELEGATE_CLASS");
        // SAFETY: same runtime calls as above, on a class just confirmed found.
        unsafe {
            let cls = objc_getClass(DELEGATE_CLASS.as_ptr());
            let sel = sel_registerName(SELECTOR.as_ptr());
            assert!(!class_getInstanceMethod(cls, sel).is_null(), "method not installed");
        }
    }

    #[test]
    fn installing_twice_is_harmless() {
        assert!(present_while_frontmost());
        assert!(present_while_frontmost());
    }

    /// Installed is not the same as callable: macOS reaches the method by
    /// ordinary message dispatch and reads a single byte back as `BOOL`. Ask
    /// the delegate the way the system does and check it says yes.
    #[test]
    fn the_delegate_answers_yes_when_asked() {
        #[link(name = "objc", kind = "dylib")]
        extern "C" {
            fn objc_msgSend();
        }
        assert!(present_while_frontmost());
        // SAFETY: each `objc_msgSend` is transmuted to the exact signature of
        // the selector being sent — `alloc`/`init` return the object, and
        // `shouldPresentNotification:` takes the center and the notification
        // (ignored by our implementation, hence null) and returns BOOL.
        unsafe {
            let cls = objc_getClass(DELEGATE_CLASS.as_ptr());
            let new: extern "C" fn(Id, Sel) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            let delegate = new(
                new(cls, sel_registerName(c"alloc".as_ptr())),
                sel_registerName(c"init".as_ptr()),
            );
            let ask: extern "C" fn(Id, Sel, Id, Id) -> bool =
                std::mem::transmute(objc_msgSend as *const c_void);
            assert!(ask(
                delegate,
                sel_registerName(SELECTOR.as_ptr()),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ));
        }
    }
}
