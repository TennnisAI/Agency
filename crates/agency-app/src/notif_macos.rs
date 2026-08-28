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
//! into this binary. A delegate object of our own would strand the delivery
//! callback their send path waits on, so the class is patched in place and
//! [`on_notification_click`] hands their own implementation the calls it
//! replaces.
//!
//! The second thing patched in here is being told when a notification is
//! *clicked*. Their delegate implements that callback (it is how
//! `NotificationResponse::Click` reaches Rust) and tauri-plugin-notification
//! drops it on the floor, so before AGE-166 the app inferred the click from
//! the app regaining focus — which is also what a click anywhere in a window
//! some banner or panel had blurred looks like. Hence the jumps to whichever
//! agent had just notified.

//! Only macOS needs this: other platforms' notification services show a
//! banner regardless of which app is in front. `lib.rs` gates the module.

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::OnceLock;

/// The delegate class mac-notification-sys compiles into this binary. Private
/// to that crate, so a dependency bump could rename it; the lookup below fails
/// loudly (a log line, and today's background-only behaviour) rather than
/// breaking anything if it ever does.
const DELEGATE_CLASS: &CStr = c"NotificationCenterDelegate";
const SELECTOR: &CStr = c"userNotificationCenter:shouldPresentNotification:";
/// Their click callback, wrapped rather than replaced — see
/// [`on_notification_click`].
const ACTIVATED_SELECTOR: &CStr = c"userNotificationCenter:didActivateNotification:";
/// ObjC type encoding for `-(void)…:(id)…:(id)`: void return, self, _cmd, and
/// the protocol's two object arguments.
const ACTIVATED_SIGNATURE: &CStr = c"v@:@@";
/// `NSUserNotificationActivationTypeNone`: the notification came back to the
/// delegate without the user having answered it. Every other activation type
/// is a click of some kind (ours carry no buttons, so in practice it is
/// `ContentsClicked`).
const ACTIVATION_NONE: i64 = 0;

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
type DidActivateFn = extern "C" fn(Id, Sel, Id, Id);
type ActivationTypeFn = extern "C" fn(Id, Sel) -> i64;

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
    fn objc_msgSend();
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

/// mac-notification-sys' own click callback, kept so ours can hand every call
/// on to it: theirs is what wakes a send waiting on a response and takes the
/// notification out of Notification Center.
static ORIGINAL_ACTIVATED: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// What to do about a click, as installed by [`on_notification_click`].
static ON_CLICK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Be told when the user clicks one of our notifications. `handler` runs on the
/// main thread, where AppKit delivers the callback. Returns whether the hook
/// went in; a `false` means clicks go unnoticed, which is the behaviour of
/// every platform that isn't macOS.
///
/// Installs once: patching a second time would capture our own implementation
/// as the one to hand on to, and a click would then recurse until the stack
/// gave out.
pub fn on_notification_click(handler: impl Fn() + Send + Sync + 'static) -> bool {
    if ON_CLICK.set(Box::new(handler)).is_err() {
        return !ORIGINAL_ACTIVATED.load(Ordering::Relaxed).is_null();
    }
    // SAFETY: plain ObjC runtime calls, as in `present_while_frontmost` above.
    // `did_activate` matches the selector's signature exactly and passes its
    // arguments through untouched to the implementation it replaced.
    unsafe {
        let cls = objc_getClass(DELEGATE_CLASS.as_ptr());
        if cls.is_null() {
            log::warn!(
                "notification delegate class {DELEGATE_CLASS:?} not found; clicking a \
                 notification will not open the run it is about"
            );
            return false;
        }
        let sel = sel_registerName(ACTIVATED_SELECTOR.as_ptr());
        let original = class_replaceMethod(
            cls,
            sel,
            std::mem::transmute::<DidActivateFn, Imp>(did_activate),
            ACTIVATED_SIGNATURE.as_ptr(),
        );
        if original.is_null() {
            // Nothing to hand on to: their implementation is gone, so the
            // notification would stay in Notification Center after a click and
            // a waiting send would never wake. Ours still opens the run.
            log::warn!("notification delegate has no didActivateNotification: to call through to");
        }
        ORIGINAL_ACTIVATED.store(original.cast_mut(), Ordering::Relaxed);
        true
    }
}

/// `-[NotificationCenterDelegate userNotificationCenter:didActivateNotification:]`,
/// as installed by [`on_notification_click`].
extern "C" fn did_activate(this: Id, cmd: Sel, center: Id, notification: Id) {
    // Read and act before handing on: their implementation ends by removing the
    // notification from the center. A panic must not unwind out of Rust and
    // into AppKit — an unopened run is worth less than the process.
    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if activation_type(notification) == ACTIVATION_NONE {
            return;
        }
        if let Some(handler) = ON_CLICK.get() {
            handler();
        }
    }));
    if handled.is_err() {
        log::error!("handling a notification click panicked");
    }
    let original = ORIGINAL_ACTIVATED.load(Ordering::Relaxed);
    if original.is_null() {
        return;
    }
    // SAFETY: `original` was read off this very selector, so it takes these
    // arguments, and they are passed on untouched.
    let original: DidActivateFn = unsafe { std::mem::transmute(original) };
    original(this, cmd, center, notification);
}

/// How the user answered a notification: `NSUserNotificationActivationType`.
fn activation_type(notification: Id) -> i64 {
    if notification.is_null() {
        return ACTIVATION_NONE;
    }
    // SAFETY: `activationType` is a property on every NSUserNotification and
    // returns an NSInteger, which `objc_msgSend` is transmuted to here.
    unsafe {
        let ask: ActivationTypeFn = std::mem::transmute(objc_msgSend as *const c_void);
        ask(notification, sel_registerName(c"activationType".as_ptr()))
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

    /// Clicks counted by the hook every test in this module shares: `ON_CLICK`
    /// takes the first handler set in the process and keeps it.
    static CLICKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn install_counting_hook() -> bool {
        on_notification_click(|| {
            CLICKS.fetch_add(1, Ordering::Relaxed);
        })
    }

    /// Allocate a real `NSUserNotification`, the argument macOS passes the
    /// delegate. A fresh one has never been answered, so its activation type is
    /// `None`.
    fn new_notification() -> Id {
        // SAFETY: `alloc`/`init` on a Foundation class, each transmuted to the
        // signature of the selector being sent.
        unsafe {
            let new: extern "C" fn(Id, Sel) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            let cls = objc_getClass(c"NSUserNotification".as_ptr());
            assert!(!cls.is_null(), "NSUserNotification not found");
            new(new(cls, sel_registerName(c"alloc".as_ptr())), sel_registerName(c"init".as_ptr()))
        }
    }

    /// The click hook only works if their implementation is there to hand the
    /// call on to — it is what removes the delivered notification and wakes a
    /// waiting send. A dependency bump that drops it fails here.
    #[test]
    fn wraps_the_backends_own_click_callback() {
        assert!(install_counting_hook(), "no delegate class — see DELEGATE_CLASS");
        let original = ORIGINAL_ACTIVATED.load(Ordering::Relaxed);
        assert!(!original.is_null(), "nothing to call through to");
        // Installing again must not capture *our* implementation as the one to
        // hand on to: that click would recurse forever.
        assert!(install_counting_hook());
        assert_eq!(ORIGINAL_ACTIVATED.load(Ordering::Relaxed), original);
        assert_ne!(original.cast_const(), did_activate as DidActivateFn as *const c_void);
    }

    /// A notification the user never answered comes back to the delegate with
    /// activation type `None`. Nothing to open — and the call still reaches
    /// mac-notification-sys underneath without falling over.
    #[test]
    fn an_unanswered_notification_opens_nothing() {
        assert!(install_counting_hook());
        let notification = new_notification();
        assert_eq!(activation_type(notification), ACTIVATION_NONE);
        assert_eq!(activation_type(std::ptr::null_mut()), ACTIVATION_NONE);
        let before = CLICKS.load(Ordering::Relaxed);
        // SAFETY: the delegate is sent the message macOS sends it, with the
        // notification it would carry; a null center is one nothing reads.
        unsafe {
            let new: extern "C" fn(Id, Sel) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            let cls = objc_getClass(DELEGATE_CLASS.as_ptr());
            let delegate = new(
                new(cls, sel_registerName(c"alloc".as_ptr())),
                sel_registerName(c"init".as_ptr()),
            );
            let send: extern "C" fn(Id, Sel, Id, Id) =
                std::mem::transmute(objc_msgSend as *const c_void);
            send(
                delegate,
                sel_registerName(ACTIVATED_SELECTOR.as_ptr()),
                std::ptr::null_mut(),
                notification,
            );
        }
        assert_eq!(CLICKS.load(Ordering::Relaxed), before, "opened a run nobody asked for");
    }
}
