use super::{Button, Hotkeys, RAW};
use std::ptr::{null, null_mut};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{Weak, atomic::Ordering, mpsc::SyncSender},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Input::KeyboardAndMouse::GetAsyncKeyState, WindowsAndMessaging::*},
};
thread_local! { static OWNER: RefCell<Weak<Hotkeys>> = const { RefCell::new(Weak::new()) }; }

pub(super) fn held_modifiers() -> Vec<Button> {
    [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c]
        .into_iter()
        .map(Button::keyboard)
        .filter(physical_down)
        .collect()
}
pub(super) fn wake_stop(thread: u32) {
    if thread != 0 {
        unsafe {
            PostThreadMessageW(thread, WM_QUIT, 0, 0);
        }
    }
}
/// Asks the hook thread to install or remove the mouse hook now that the
/// bindings or the capture changed, instead of at its next watchdog tick.
const WM_SYNC_HOOKS: u32 = WM_APP + 1;
pub(super) fn wake_sync(thread: u32) {
    if thread != 0 {
        unsafe {
            PostThreadMessageW(thread, WM_SYNC_HOOKS, 0, 0);
        }
    }
}
/// The keyboard hook is always installed. The mouse hook only while a binding
/// uses a mouse button or a capture may record one: every mouse event in the
/// session, a move from a 1-8 kHz gaming mouse included, is delivered to the
/// hook thread synchronously and waited for before it reaches the game.
struct Hooks {
    module: HINSTANCE,
    keyboard: HHOOK,
    mouse: Option<HHOOK>,
}
impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.keyboard);
        }
        self.set_mouse(false).ok();
    }
}
fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}
impl Hooks {
    fn install(mouse: bool) -> Result<Self, String> {
        let module = unsafe { GetModuleHandleW(null()) };
        if module.is_null() {
            return Err(format!("global hotkey module: {}", last_error()));
        }
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
        if keyboard.is_null() {
            return Err(format!("global keyboard hook: {}", last_error()));
        }
        let mut hooks = Self {
            module,
            keyboard,
            mouse: None,
        };
        hooks.set_mouse(mouse)?;
        Ok(hooks)
    }
    /// Returns whether the mouse hook was newly installed.
    fn set_mouse(&mut self, wanted: bool) -> Result<bool, String> {
        match (wanted, self.mouse) {
            (true, None) => {
                let hook =
                    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), self.module, 0) };
                if hook.is_null() {
                    return Err(format!("global mouse hook: {}", last_error()));
                }
                self.mouse = Some(hook);
                Ok(true)
            }
            (false, Some(hook)) => {
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
                self.mouse = None;
                Ok(false)
            }
            _ => Ok(false),
        }
    }
}
/// Matches the mouse hook to what the engine needs; a newly installed hook
/// learns which bound mouse buttons are already held.
fn sync_mouse(hooks: &mut Hooks, service: &Hotkeys) -> Result<(), String> {
    let mut state = service.state.lock().unwrap_or_else(|p| p.into_inner());
    let wanted = state.engine.wants_mouse();
    if hooks.set_mouse(wanted)? {
        let held: Vec<_> = state
            .engine
            .watched()
            .into_iter()
            .filter(|b| b.device == 1)
            .filter(physical_down)
            .collect();
        state.engine.seed(held);
    } else if !wanted {
        // Nothing reports their releases any more.
        state.engine.forget_device(1);
    }
    Ok(())
}
struct Timer(usize);
impl Drop for Timer {
    fn drop(&mut self) {
        unsafe {
            KillTimer(null_mut(), self.0);
        }
    }
}

pub(super) fn run(owner: Weak<Hotkeys>, ready: SyncSender<Result<(), String>>) {
    OWNER.with(|slot| *slot.borrow_mut() = owner.clone());
    let id = unsafe { GetCurrentThreadId() };
    let Some(service) = owner.upgrade() else {
        let _ = ready.send(Err("hotkeys were destroyed".into()));
        return;
    };
    service.thread_id.store(id, Ordering::Release);
    let mut message = MSG::default();
    unsafe {
        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
    }
    let wants_mouse = service
        .state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .engine
        .wants_mouse();
    let initial = Hooks::install(wants_mouse);
    let mut hooks = match initial {
        Ok(hooks) => Some(hooks),
        Err(error) => {
            service.thread_id.store(0, Ordering::Release);
            let _ = ready.send(Err(error));
            return;
        }
    };
    let timer = unsafe { SetTimer(null_mut(), 0, 100, None) };
    if timer == 0 {
        let _ = ready.send(Err("could not create global hotkey watchdog".into()));
        service.thread_id.store(0, Ordering::Release);
        return;
    }
    let _timer = Timer(timer);
    {
        let mut state = service.state.lock().unwrap_or_else(|p| p.into_inner());
        let initial: Vec<_> = state
            .engine
            .watched()
            .into_iter()
            .filter(physical_down)
            .chain(held_modifiers())
            .collect();
        state.engine.seed(initial);
    }
    drop(service);
    let _ = ready.send(Ok(()));
    let mut previous_tick = Instant::now();
    let mut last_restart = previous_tick;
    let mut unexpected = BTreeMap::<Button, Instant>::new();
    loop {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result <= 0 {
            if result < 0 {
                if let Some(service) = owner.upgrade() {
                    service.fail("global hotkey message loop failed".into());
                }
            }
            break;
        }
        let Some(service) = owner.upgrade() else {
            break;
        };
        if service.stopping.load(Ordering::Acquire) {
            break;
        }
        if matches!(message.message, WM_TIMER | WM_SYNC_HOOKS) {
            if let Some(hooks) = hooks.as_mut() {
                if let Err(error) = sync_mouse(hooks, &service) {
                    service.fail(error);
                    break;
                }
            }
        }
        if message.message == WM_TIMER {
            let now = Instant::now();
            if now.saturating_duration_since(previous_tick) > Duration::from_secs(2) {
                service.reset();
            }
            // On a locked/secure desktop the hook cannot observe releases;
            // cancel holds even though the message pump itself keeps ticking.
            if unsafe { GetForegroundWindow() }.is_null() {
                service.reset();
            }
            previous_tick = now;
            service.tick(now);
            let (pressed, pollable, mut watched) = {
                let state = service.state.lock().unwrap_or_else(|p| p.into_inner());
                (
                    state.engine.pressed(),
                    state.engine.pollable_pressed(),
                    state.engine.watched(),
                )
            };
            for button in &pollable {
                if physical_vk(*button).is_some() && !physical_down(button) {
                    service.input(*button, false);
                }
            }
            watched.extend(
                [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c]
                    .into_iter()
                    .map(Button::keyboard),
            );
            let mut missing = BTreeSet::new();
            for button in watched {
                // A raw key may share its VK with a different physical key
                // (main/numpad Enter). It can be released by polling, but cannot
                // be inferred as a new press from that ambiguous state.
                if button.code & RAW != 0 || pressed.contains(&button) || !physical_down(&button) {
                    continue;
                }
                if pressed
                    .iter()
                    .any(|p| physical_vk(*p) == physical_vk(button))
                {
                    continue;
                }
                missing.insert(button);
                unexpected.entry(button).or_insert(now);
            }
            unexpected.retain(|key, _| missing.contains(key));
            // Windows may silently remove a timed-out low-level hook. A
            // sustained OS-down key absent from the hook state proves a gap.
            if now.saturating_duration_since(last_restart) >= Duration::from_secs(1)
                && unexpected.values().any(|since| {
                    now.saturating_duration_since(*since) >= Duration::from_millis(300)
                })
            {
                service.reset();
                hooks.take(); // Unhook both before replacing either one.
                let wants_mouse = service
                    .state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .engine
                    .wants_mouse();
                match Hooks::install(wants_mouse) {
                    Ok(fresh) => {
                        hooks = Some(fresh);
                        last_restart = now;
                        unexpected.clear();
                        service.restarts.fetch_add(1, Ordering::AcqRel);
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
                    Err(error) => {
                        service.fail(error);
                        break;
                    }
                }
            }
        } else {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    drop(hooks);
    OWNER.with(|slot| *slot.borrow_mut() = Weak::new());
    if let Some(service) = owner.upgrade() {
        service.reset();
        service.running.store(false, Ordering::Release);
        service
            .thread_id
            .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok();
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0
        && matches!(
            wparam as u32,
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
        )
    {
        let data = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let button = keyboard_button(data.vkCode, data.scanCode, data.flags & LLKHF_EXTENDED != 0);
        let scan = (data.scanCode & 0xff) as u16
            | if data.flags & LLKHF_EXTENDED != 0 {
                0xe000
            } else {
                0
            };
        let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if let Some(button) = button {
            let consumed = OWNER.with(|slot| {
                slot.borrow()
                    .upgrade()
                    .is_some_and(|owner| owner.input_at(button, scan, down))
            });
            if consumed {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}
unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let data = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if let Some((button, down)) = mouse_button(wparam as u32, data.mouseData) {
            let consumed = OWNER.with(|slot| {
                slot.borrow()
                    .upgrade()
                    .is_some_and(|owner| owner.input(button, down))
            });
            if consumed {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}
fn keyboard_button(vk: u32, scan: u32, extended: bool) -> Option<Button> {
    let code = match vk {
        0x10 => {
            if scan == 0x36 {
                0xa1
            } else {
                0xa0
            }
        }
        0x11 => {
            if extended {
                0xa3
            } else {
                0xa2
            }
        }
        0x12 => {
            if extended {
                0xa5
            } else {
                0xa4
            }
        }
        0x0d if extended => RAW | 0x0e1c,
        0x6c if scan == 0x7e => RAW | 0x007e,
        0x0c if !extended && scan == 0x4c => RAW | 0xee4c,
        0x21..=0x28 | 0x2d | 0x2e if !extended && matches!(scan, 0x47..=0x53) => {
            RAW | 0xee00 | (scan & 0xff)
        }
        0 | 0xff if scan != 0 => RAW | (if extended { 0xe000 } else { 0 }) | (scan & 0xff),
        1..=255 => vk,
        _ => return None,
    };
    Some(Button::keyboard(code))
}
fn mouse_button(message: u32, data: u32) -> Option<(Button, bool)> {
    let (code, down) = match message {
        WM_LBUTTONDOWN => (1, true),
        WM_LBUTTONUP => (1, false),
        WM_RBUTTONDOWN => (2, true),
        WM_RBUTTONUP => (2, false),
        WM_MBUTTONDOWN => (3, true),
        WM_MBUTTONUP => (3, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let x = data >> 16;
            if !(1..=2).contains(&x) {
                return None;
            }
            (x + 3, message == WM_XBUTTONDOWN)
        }
        _ => return None,
    };
    Some((Button { device: 1, code }, down))
}
fn physical_vk(button: Button) -> Option<i32> {
    if button.code >= super::POSITION {
        return None;
    }
    if button.device == 1 {
        let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
        return Some(match button.code {
            1 => {
                if swapped {
                    2
                } else {
                    1
                }
            }
            2 => {
                if swapped {
                    1
                } else {
                    2
                }
            }
            3 => 4,
            4 => 5,
            5 => 6,
            _ => return None,
        });
    }
    if button.code & RAW == 0 {
        return Some(button.code as i32);
    }
    Some(match button.code & 0xffff {
        0x0e1c => 0x0d,
        0x007e => 0x6c,
        0xee52 => 0x2d,
        0xee53 => 0x2e,
        0xee47 => 0x24,
        0xee4f => 0x23,
        0xee49 => 0x21,
        0xee51 => 0x22,
        0xee48 => 0x26,
        0xee50 => 0x28,
        0xee4b => 0x25,
        0xee4d => 0x27,
        0xee4c => 0x0c,
        _ => return None,
    })
}
fn physical_down(button: &Button) -> bool {
    physical_vk(*button).is_some_and(|vk| unsafe { GetAsyncKeyState(vk) } < 0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_vk_sides_and_raw_numpad_are_not_xt_scan_codes() {
        assert_eq!(
            keyboard_button(0x4d, 0x32, false),
            Some(Button::keyboard(0x4d))
        );
        assert_eq!(
            keyboard_button(0x11, 0x1d, true),
            Some(Button::keyboard(0xa3))
        );
        assert_eq!(
            keyboard_button(0x10, 0x36, false),
            Some(Button::keyboard(0xa1))
        );
        assert_eq!(
            keyboard_button(0x0d, 0x1c, true),
            Some(Button::keyboard(RAW | 0x0e1c))
        );
        assert_eq!(
            keyboard_button(0x2d, 0x52, false),
            Some(Button::keyboard(RAW | 0xee52))
        );
        assert_eq!(
            keyboard_button(0x2d, 0x52, true),
            Some(Button::keyboard(0x2d))
        );
        assert_eq!(
            keyboard_button(0x0c, 0x4c, false),
            Some(Button::keyboard(RAW | 0xee4c))
        );
    }
    #[test]
    fn mouse_x_buttons_keep_the_client_numbering() {
        assert_eq!(
            mouse_button(WM_XBUTTONDOWN, 1 << 16),
            Some((Button { device: 1, code: 4 }, true))
        );
        assert_eq!(
            mouse_button(WM_XBUTTONUP, 2 << 16),
            Some((Button { device: 1, code: 5 }, false))
        );
    }
    #[test]
    #[ignore = "installs real global hooks; run explicitly on an interactive Windows desktop"]
    fn real_mouse_hook_is_installed_only_on_demand() {
        let mut hooks = Hooks::install(false).unwrap();
        assert!(hooks.mouse.is_none());
        assert!(hooks.set_mouse(true).unwrap(), "newly installed");
        assert!(hooks.mouse.is_some());
        assert!(!hooks.set_mouse(true).unwrap(), "already installed");
        assert!(!hooks.set_mouse(false).unwrap());
        assert!(hooks.mouse.is_none());
        let hooks = Hooks::install(true).unwrap();
        assert!(hooks.mouse.is_some());
    }
    #[test]
    #[ignore = "installs real global hooks; run explicitly on an interactive Windows desktop"]
    fn real_hooks_start_stop_and_restart_without_generating_input() {
        let service = Hotkeys::new();
        for _ in 0..3 {
            service.start().unwrap();
            assert!(service.status().running);
            assert_ne!(service.thread_id.load(Ordering::Acquire), 0);
            service.stop();
            assert!(!service.status().running);
            assert_eq!(service.thread_id.load(Ordering::Acquire), 0);
            assert!(service.workers.lock().unwrap().is_none());
        }
        assert!(service.restart().unwrap());
        assert_eq!(service.status().restarts, 1);
        service.stop();
    }
    #[test]
    #[ignore = "injects only suppressed F24 into an isolated hook service; run explicitly on an interactive Windows desktop"]
    fn real_f24_input_reaches_handler_with_balanced_suppressed_hold() {
        use super::super::{Binding, Chord};
        use std::sync::{Arc, mpsc};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_F24,
        };
        fn send_f24(up: bool) -> u32 {
            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_F24,
                        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                        dwExtraInfo: 0x564e484b,
                        ..Default::default()
                    },
                },
            };
            unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) }
        }
        struct Stop(Arc<Hotkeys>);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.stop();
            }
        }
        struct Release(bool);
        impl Drop for Release {
            fn drop(&mut self) {
                if self.0 {
                    let _ = send_f24(true);
                }
            }
        }

        assert!(
            !unsafe { GetForegroundWindow() }.is_null(),
            "interactive desktop required"
        );
        assert!(
            held_modifiers().is_empty(),
            "do not run while a real modifier is held"
        );
        assert!(
            !physical_down(&Button::keyboard(VK_F24 as u32)),
            "F24 is already physically held"
        );
        let service = Hotkeys::new();
        let _stop = Stop(service.clone());
        let (sender, receiver) = mpsc::channel();
        let owner = Arc::downgrade(&service);
        service.set_handler(Arc::new(move |event| {
            if event.id == "native-f24-proof"
                && owner
                    .upgrade()
                    .is_some_and(|service| service.is_current(&event))
            {
                let _ = sender.send(event);
            }
        }));
        service.start().unwrap();
        service
            .set_bindings(vec![Binding {
                id: "native-f24-proof".into(),
                chord: Chord {
                    buttons: vec![Button::keyboard(VK_F24 as u32)],
                },
                suppress: Some(true),
                trigger_cooldown_ms: None,
            }])
            .unwrap();
        let mut release = Release(true);
        assert_eq!(send_f24(false), 1, "SendInput did not inject F24 down");
        let down = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("installed hook never delivered F24 DOWN to its handler");
        assert_eq!(down.phase, "down");
        assert_eq!(down.generation, service.current_generation());
        assert!(
            matches!(
                receiver.recv_timeout(Duration::from_millis(250)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "the suppressed hold was prematurely released by the watchdog"
        );
        assert_eq!(
            send_f24(false),
            1,
            "SendInput did not inject the F24 autorepeat"
        );
        assert!(
            matches!(
                receiver.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "autorepeat emitted a second DOWN"
        );
        assert_eq!(send_f24(true), 1, "SendInput did not inject F24 up");
        release.0 = false;
        let up = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("installed hook never delivered F24 UP to its handler");
        assert_eq!(up.phase, "up");
        assert_eq!(up.generation, down.generation);
        service.stop();
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }
}
