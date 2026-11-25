//! `appWindow.theme` as AppKit appearance. A window's own `appearance` reaches its frame, traffic
//! lights and sheets; app-modal panels, alerts, menus and the colour panel take `NSApp.appearance`,
//! so both are set.

use std::cell::Cell;
use std::ffi::CStr;
use std::os::raw::c_char;

use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};
use objc2_foundation::NSString;

use super::Theme;

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSAppearanceNameAqua: &'static NSString;
    static NSAppearanceNameDarkAqua: &'static NSString;
}

thread_local! {
    // Nothing is written before the first forced theme, so an embedder's own appearance stays.
    static ENGAGED: Cell<bool> = const { Cell::new(false) };
}

fn name(theme: Theme) -> Option<&'static NSString> {
    // SAFETY: AppKit's constant strings, linked above.
    unsafe {
        match theme {
            Theme::System => None,
            Theme::Light => Some(NSAppearanceNameAqua),
            Theme::Dark => Some(NSAppearanceNameDarkAqua),
        }
    }
}

fn appearance(theme: Theme) -> *mut AnyObject {
    match name(theme) {
        None => std::ptr::null_mut(),
        // SAFETY: a class method taking one of AppKit's appearance names.
        Some(n) => unsafe { msg_send![class!(NSAppearance), appearanceNamed: n] },
    }
}

/// `NSAppearance.name` as a Rust string; `None` for nil.
fn name_of(appearance: *mut AnyObject) -> Option<String> {
    // SAFETY: an `NSAppearance` or nil, on the main thread.
    unsafe {
        let a = appearance.as_ref()?;
        let n: *mut AnyObject = msg_send![a, name];
        utf8(n)
    }
}

unsafe fn utf8(string: *mut AnyObject) -> Option<String> {
    let s = unsafe { string.as_ref()? };
    let p: *const c_char = unsafe { msg_send![s, UTF8String] };
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn holds(current: *mut AnyObject, theme: Theme) -> bool {
    // SAFETY: an `NSAppearance` or nil, on the main thread.
    unsafe {
        match (current.as_ref(), name(theme)) {
            (None, None) => true,
            (Some(a), Some(want)) => {
                let have: *mut AnyObject = msg_send![a, name];
                have.as_ref()
                    .is_some_and(|h| msg_send![h, isEqualToString: want])
            }
            _ => false,
        }
    }
}

fn app() -> *mut AnyObject {
    // SAFETY: the shared application, on the main thread.
    unsafe { msg_send![class!(NSApplication), sharedApplication] }
}

fn window_of(view: usize) -> Option<&'static AnyObject> {
    MainThreadMarker::new()?;
    if view == 0 {
        return None;
    }
    // SAFETY: gpui's live `NSView`; a view can be moved into another window, so it is asked each time.
    unsafe {
        let window: *mut AnyObject = msg_send![view as *mut AnyObject, window];
        window.as_ref()
    }
}

/// Sets `NSApp.appearance` for `theme`.
pub fn set_app(theme: Theme) {
    if MainThreadMarker::new().is_none() || (theme == Theme::System && !ENGAGED.get()) {
        return;
    }
    ENGAGED.set(true);
    // SAFETY: `NSApplication` setter on the main thread; nil follows the system.
    unsafe {
        let _: () = msg_send![app(), setAppearance: appearance(theme)];
    }
}

/// Puts the window holding `view` on `theme`; a read and nothing else when it already is.
pub fn keep(view: usize, theme: Theme) {
    if !ENGAGED.get() {
        return;
    }
    let Some(window) = window_of(view) else {
        return;
    };
    // SAFETY: `NSWindow` getter and setter on the main thread.
    unsafe {
        let current: *mut AnyObject = msg_send![window, appearance];
        if !holds(current, theme) {
            let _: () = msg_send![window, setAppearance: appearance(theme)];
        }
    }
}

/// What the devtools `windows` command reports for a window on macOS.
pub struct Debug {
    pub window: Option<String>,
    pub effective: Option<String>,
    pub app: Option<String>,
}

pub fn debug(view: usize) -> Option<Debug> {
    let window = window_of(view)?;
    // SAFETY: `NSWindow` and `NSApplication` getters on the main thread.
    unsafe {
        let own: *mut AnyObject = msg_send![window, appearance];
        let effective: *mut AnyObject = msg_send![window, effectiveAppearance];
        let app_own: *mut AnyObject = msg_send![app(), appearance];
        Some(Debug {
            window: name_of(own),
            effective: name_of(effective),
            app: name_of(app_own),
        })
    }
}
