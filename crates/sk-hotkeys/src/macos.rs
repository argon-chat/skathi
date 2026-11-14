//! Quartz event tap. The callback only updates the bounded native chord engine;
//! callbacks into JavaScript are delivered by the separate Dispatch worker.
//! Persisted chords use the same VK-compatible physical key identities on both OSes.
use super::{Button, Hotkeys, RAW};
use std::{
    ffi::c_void,
    ptr,
    sync::{Weak, atomic::Ordering, mpsc::SyncSender},
    time::{Duration, Instant},
};

type Ref = *mut c_void;
type Event = Ref;
type TapCallback = unsafe extern "C" fn(Ref, u32, Event, Ref) -> Event;
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: Ref) -> u8;
    static kAXTrustedCheckOptionPrompt: Ref;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        location: u32,
        placement: u32,
        options: u32,
        mask: u64,
        callback: TapCallback,
        context: Ref,
    ) -> Ref;
    fn CGEventTapEnable(tap: Ref, enable: bool);
    fn CGEventGetIntegerValueField(event: Event, field: u32) -> i64;
    fn CGEventGetFlags(event: Event) -> u64;
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: *const CFDictionaryKeyCallBacks,
        value_callbacks: *const CFDictionaryValueCallBacks,
    ) -> Ref;
    fn CFRelease(value: Ref);
    fn CFMachPortCreateRunLoopSource(allocator: Ref, port: Ref, order: isize) -> Ref;
    fn CFMachPortInvalidate(port: Ref);
    fn CFRunLoopGetCurrent() -> Ref;
    fn CFRunLoopAddSource(run_loop: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRemoveSource(run_loop: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRunInMode(mode: Ref, seconds: f64, return_after_source: u8) -> i32;
    static kCFRunLoopDefaultMode: Ref;
    static kCFBooleanTrue: Ref;
    // The CFType callbacks: the dictionary retains its key and value and
    // compares them as CF objects. Null callbacks would hold them unretained
    // and compare by pointer, which AX's lookup must not rely on.
    static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
}

/// `CFDictionaryKeyCallBacks`: only ever passed by address, never read.
#[repr(C)]
struct CFDictionaryKeyCallBacks {
    version: isize,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
}

/// `CFDictionaryValueCallBacks`: only ever passed by address, never read.
#[repr(C)]
struct CFDictionaryValueCallBacks {
    version: isize,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

pub(super) fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}
/// No automatic prompt at boot: only the application's explicit Grant Access
/// action calls this, and the actual grant remains owned by System Settings.
pub(super) fn request_accessibility() -> bool {
    unsafe {
        let key = kAXTrustedCheckOptionPrompt;
        let value = kCFBooleanTrue;
        let options = CFDictionaryCreate(
            ptr::null_mut(),
            &key,
            &value,
            1,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        if options.is_null() {
            return false;
        }
        let granted = AXIsProcessTrustedWithOptions(options) != 0;
        CFRelease(options);
        granted
    }
}

pub(super) fn held_modifiers() -> Vec<Button> {
    MODIFIERS
        .iter()
        .filter_map(|&(key, vk, _)| {
            unsafe { CGEventSourceKeyState(1, key) }.then(|| Button::keyboard(vk))
        })
        .collect()
}
// The event loop has a 50ms bounded wait, so stopping and reconfiguration never
// dereference a CFRunLoop belonging to an already-finished input thread.
pub(super) fn wake_stop(_: u32) {}
pub(super) fn wake_sync(_: u32) {}

const MODIFIERS: &[(u16, u32, u64)] = &[
    (0x38, 0xa0, 0x02),
    (0x3c, 0xa1, 0x04),
    (0x3b, 0xa2, 0x01),
    (0x3e, 0xa3, 0x2000),
    (0x3a, 0xa4, 0x20),
    (0x3d, 0xa5, 0x40),
    (0x37, 0x5b, 0x08),
    (0x36, 0x5c, 0x10),
];
fn modifier_button(key: u16, flags: u64) -> Option<(Button, bool)> {
    MODIFIERS
        .iter()
        .find(|&&(code, _, _)| code == key)
        .map(|&(_, vk, mask)| (Button::keyboard(vk), flags & mask != 0))
}

struct Context {
    owner: Weak<Hotkeys>,
    tap: Ref,
}
unsafe extern "C" fn event_callback(_: Ref, kind: u32, event: Event, context: Ref) -> Event {
    // Prevent a Rust panic from unwinding into Quartz. A failed callback must
    // pass the event through rather than strand the user's keyboard.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let context = &*(context as *const Context);
        let Some(owner) = context.owner.upgrade() else { return event; };
        if owner.stopping.load(Ordering::Acquire) { return event; }
        if kind == 0xffff_fffe {
            owner.reset();
            CGEventTapEnable(context.tap, true);
            owner.restarts.fetch_add(1, Ordering::AcqRel);
            return event;
        }
        if kind == 0xffff_ffff {
            owner.reset();
            owner.fail("macOS disabled the global input tap; check Accessibility access and restart hotkeys".into());
            return event;
        }
        if event.is_null() { return event; }
        let suppress = match kind {
            10 | 11 => {
                let key = CGEventGetIntegerValueField(event, 9) as u16;
                key_button(key).is_some_and(|button| owner.input(button, kind == 10))
            }
            12 => {
                let key = CGEventGetIntegerValueField(event, 9) as u16;
                if let Some((button, down)) = modifier_button(key, CGEventGetFlags(event)) {
                    owner.input(button, down)
                } else if key == 0x39 {
                    // Caps Lock toggles report flagsChanged instead of a normal
                    // key-up pair. Deliver a complete stroke, never a stuck hold.
                    let consumed = owner.input(Button::keyboard(0x14), true);
                    let released = owner.input(Button::keyboard(0x14), false);
                    consumed || released
                } else { false }
            }
            1 | 2 | 3 | 4 | 25 | 26 => {
                let index = CGEventGetIntegerValueField(event, 3);
                if (0..5).contains(&index) {
                    let wanted = owner.state.lock().unwrap_or_else(|p| p.into_inner()).engine.wants_mouse();
                    wanted && owner.input(Button { device: 1, code: index as u32 + 1 }, matches!(kind, 1 | 3 | 25))
                } else { false }
            }
            _ => false,
        };
        if suppress { ptr::null_mut() } else { event }
    })).unwrap_or(event)
}

pub(super) fn run(owner: Weak<Hotkeys>, ready: SyncSender<Result<(), String>>) {
    let Some(service) = owner.upgrade() else {
        let _ = ready.send(Err("hotkeys were destroyed".into()));
        return;
    };
    if !accessibility_granted() {
        let _ = ready.send(Err("Allow this app in System Settings → Privacy & Security → Accessibility, then restart hotkeys".into()));
        return;
    }
    let mut context = Box::new(Context {
        owner: owner.clone(),
        tap: ptr::null_mut(),
    });
    let mask = [1u32, 2, 3, 4, 10, 11, 12, 25, 26]
        .into_iter()
        .fold(0u64, |mask, kind| mask | (1 << kind));
    // Session tap (not privileged HID tap), active filtering only after AX grant.
    let tap = unsafe {
        CGEventTapCreate(
            1,
            0,
            0,
            mask,
            event_callback,
            (&mut *context as *mut Context).cast(),
        )
    };
    if tap.is_null() {
        let _ = ready.send(Err("macOS could not create the global input tap; enable Accessibility for this app and relaunch".into()));
        return;
    }
    context.tap = tap;
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null_mut(), tap, 0) };
    if source.is_null() {
        unsafe {
            CFMachPortInvalidate(tap);
            CFRelease(tap);
        }
        let _ = ready.send(Err(
            "could not create the global hotkey run-loop source".into()
        ));
        return;
    }
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
        CGEventTapEnable(tap, true);
    }
    {
        let mut state = service.state.lock().unwrap_or_else(|p| p.into_inner());
        let held: Vec<_> = state
            .engine
            .watched()
            .into_iter()
            .filter(physical_down)
            .chain(held_modifiers())
            .collect();
        state.engine.seed(held);
    }
    // Publish before ready, so an immediately delivered disable notification is
    // not accidentally overwritten by the parent's startup handshake.
    service.running.store(true, Ordering::Release);
    drop(service);
    if ready.send(Ok(())).is_err() {
        unsafe {
            CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
            CFMachPortInvalidate(tap);
            CFRelease(source);
            CFRelease(tap);
        }
        return;
    }
    let mut previous = Instant::now();
    let mut last_permission_check = previous;
    loop {
        let Some(service) = owner.upgrade() else {
            break;
        };
        if service.stopping.load(Ordering::Acquire) || !service.running.load(Ordering::Acquire) {
            break;
        }
        let now = Instant::now();
        if now.saturating_duration_since(previous) > Duration::from_secs(2) {
            service.reset();
        }
        previous = now;
        service.tick(now);
        // A secure-input/session change may hide releases from an event tap.
        // Reconcile only releases; never synthesize a press from polling.
        let pressed = service
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .engine
            .pressed();
        for button in pressed {
            if !physical_down(&button) {
                service.input(button, false);
            }
        }
        if now.saturating_duration_since(last_permission_check) >= Duration::from_secs(1) {
            if !accessibility_granted() {
                service.fail("Accessibility access was revoked; global hotkeys stopped".into());
                break;
            }
            last_permission_check = now;
        }
        drop(service);
        let result = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 1) };
        if matches!(result, 1 | 2) {
            // An invalidated source must not turn the idle loop into a busy spin.
            if let Some(service) = owner.upgrade() {
                service.fail("macOS global input run loop stopped; restart hotkeys".into());
            }
            break;
        }
    }
    unsafe {
        CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
        CFMachPortInvalidate(tap);
        CFRelease(source);
        CFRelease(tap);
    }
    if let Some(service) = owner.upgrade() {
        service.reset();
        service.running.store(false, Ordering::Release);
    }
}

fn physical_down(button: &Button) -> bool {
    match button.device {
        0 => {
            // Caps Lock is modeled as a stroke, not its persistent toggle bit.
            if button.code == 0x14 {
                return false;
            }
            (0..128u16).any(|key| {
                key_button(key).is_some_and(|b| b == *button)
                    && unsafe { CGEventSourceKeyState(1, key) }
            })
        }
        1 if (1..=5).contains(&button.code) => unsafe {
            CGEventSourceButtonState(1, button.code - 1)
        },
        _ => false,
    }
}

fn key_button(key: u16) -> Option<Button> {
    let vk = match key {
        0x00 => 0x41,
        0x01 => 0x53,
        0x02 => 0x44,
        0x03 => 0x46,
        0x04 => 0x48,
        0x05 => 0x47,
        0x06 => 0x5a,
        0x07 => 0x58,
        0x08 => 0x43,
        0x09 => 0x56,
        0x0a => 0xe2,
        0x0b => 0x42,
        0x0c => 0x51,
        0x0d => 0x57,
        0x0e => 0x45,
        0x0f => 0x52,
        0x10 => 0x59,
        0x11 => 0x54,
        0x12 => 0x31,
        0x13 => 0x32,
        0x14 => 0x33,
        0x15 => 0x34,
        0x16 => 0x36,
        0x17 => 0x35,
        0x18 => 0xbb,
        0x19 => 0x39,
        0x1a => 0x37,
        0x1b => 0xbd,
        0x1c => 0x38,
        0x1d => 0x30,
        0x1e => 0xdd,
        0x1f => 0x4f,
        0x20 => 0x55,
        0x21 => 0xdb,
        0x22 => 0x49,
        0x23 => 0x50,
        0x24 => 0x0d,
        0x25 => 0x4c,
        0x26 => 0x4a,
        0x27 => 0xde,
        0x28 => 0x4b,
        0x29 => 0xba,
        0x2a => 0xdc,
        0x2b => 0xbc,
        0x2c => 0xbf,
        0x2d => 0x4e,
        0x2e => 0x4d,
        0x2f => 0xbe,
        0x30 => 0x09,
        0x31 => 0x20,
        0x32 => 0xc0,
        0x33 => 0x08,
        0x35 => 0x1b,
        0x36 => 0x5c,
        0x37 => 0x5b,
        0x38 => 0xa0,
        0x39 => 0x14,
        0x3a => 0xa4,
        0x3b => 0xa2,
        0x3c => 0xa1,
        0x3d => 0xa5,
        0x3e => 0xa3,
        0x40 => 0x80,
        0x41 => 0x6e,
        0x43 => 0x6a,
        0x45 => 0x6b,
        0x47 => 0x0c,
        0x4b => 0x6f,
        0x4c => RAW | 0x0e1c,
        0x4e => 0x6d,
        0x4f => 0x81,
        0x50 => 0x82,
        0x51 => 0xbb,
        0x52 => 0x60,
        0x53 => 0x61,
        0x54 => 0x62,
        0x55 => 0x63,
        0x56 => 0x64,
        0x57 => 0x65,
        0x58 => 0x66,
        0x59 => 0x67,
        0x5a => 0x83,
        0x5b => 0x68,
        0x5c => 0x69,
        0x60 => 0x74,
        0x61 => 0x75,
        0x62 => 0x76,
        0x63 => 0x72,
        0x64 => 0x77,
        0x65 => 0x78,
        0x67 => 0x7a,
        0x69 => 0x7c,
        0x6a => 0x7f,
        0x6b => 0x7d,
        0x6d => 0x79,
        0x6f => 0x7b,
        0x71 => 0x7e,
        0x72 => 0x2d,
        0x73 => 0x24,
        0x74 => 0x21,
        0x75 => 0x2e,
        0x76 => 0x73,
        0x77 => 0x23,
        0x78 => 0x71,
        0x79 => 0x22,
        0x7a => 0x70,
        0x7b => 0x25,
        0x7c => 0x27,
        0x7d => 0x28,
        0x7e => 0x26,
        _ => return None,
    };
    Some(Button::keyboard(vk))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_keys_preserve_existing_chord_identities() {
        assert_eq!(key_button(0x00), Some(Button::keyboard(0x41)));
        assert_eq!(key_button(0x38), Some(Button::keyboard(0xa0)));
        assert_eq!(key_button(0x3c), Some(Button::keyboard(0xa1)));
        assert_eq!(key_button(0x37), Some(Button::keyboard(0x5b)));
        assert_ne!(key_button(0x24), key_button(0x4c));
        assert_eq!(key_button(0x4c), Some(Button::keyboard(RAW | 0x0e1c)));
        assert_eq!(key_button(0x3f), None, "Fn has no portable VK identity");
    }
    #[test]
    fn left_and_right_modifier_releases_are_independent() {
        assert_eq!(
            modifier_button(0x38, 0x06),
            Some((Button::keyboard(0xa0), true))
        );
        assert_eq!(
            modifier_button(0x38, 0x04),
            Some((Button::keyboard(0xa0), false))
        );
        assert_eq!(
            modifier_button(0x3c, 0x04),
            Some((Button::keyboard(0xa1), true))
        );
        assert_eq!(
            modifier_button(0x3e, 0x2000),
            Some((Button::keyboard(0xa3), true))
        );
        assert_eq!(
            modifier_button(0x36, 0x08),
            Some((Button::keyboard(0x5c), false))
        );
    }
    #[test]
    fn missing_accessibility_refuses_start_without_prompt_or_workers() {
        if accessibility_granted() {
            return;
        }
        let hotkeys = Hotkeys::new();
        let error = hotkeys
            .start()
            .expect_err("untrusted processes must not intercept input");
        assert!(error.contains("Accessibility"));
        let status = hotkeys.status();
        assert_eq!(status.platform, "macos");
        assert!(!status.running);
        assert!(!status.accessibility_granted);
        assert!(hotkeys.workers.lock().unwrap().is_none());
    }
}
