//! Opt-in interactive Windows fixture: only owned HWND messages, no global input.
use super::*;
use crate::tray::Tray;
use std::sync::mpsc::{Receiver, Sender};
use windows::core::BOOL;

const MN_GETHMENU: u32 = 0x01e1;
fn send(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> usize {
    let mut result = 0usize;
    assert_ne!(
        unsafe {
            SendMessageTimeoutW(
                hwnd,
                message,
                WPARAM(wparam),
                LPARAM(lparam),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                1500,
                Some(&mut result),
            )
        }
        .0,
        0,
        "native tray probe message {message:#x} timed out"
    );
    result
}
unsafe extern "system" fn find_popup(hwnd: HWND, data: LPARAM) -> BOOL {
    let mut class = [0u16; 32];
    let length = unsafe { GetClassNameW(hwnd, &mut class) };
    if length == 6 && class[..6] == [35, 51, 50, 55, 54, 56] {
        unsafe {
            *(data.0 as *mut HWND) = hwnd;
        }
        false.into()
    } else {
        true.into()
    }
}
fn popup(thread: u32) -> Option<HWND> {
    let mut result = HWND::default();
    let _enumerated = unsafe {
        EnumThreadWindows(
            thread,
            Some(find_popup),
            LPARAM((&mut result as *mut HWND) as isize),
        )
    };
    (!result.0.is_null()).then_some(result)
}
fn wait_popup(thread: u32) -> HWND {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(window) = popup(thread) {
            return window;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("owned native tray menu did not open");
}
fn expect_event(events: &Receiver<&'static str>, value: &str) {
    assert_eq!(
        events
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("native menu selection {value} did not arrive")),
        value
    );
}
fn model(events: &Sender<&'static str>, label: &'static str, custom_icon: bool) -> Options {
    let click = events.clone();
    let open = events.clone();
    let muted = events.clone();
    let quit = events.clone();
    Options {
        tooltip: "Skathi isolated tray probe".into(),
        icon: custom_icon.then(|| Icon::rgba(2, 1, vec![255, 0, 0, 128, 0, 0, 255, 255]).unwrap()),
        on_click: Some(Arc::new(move || {
            click.send(label).expect("probe event receiver");
        })),
        on_quit: Some(Arc::new(move || {
            quit.send("quit").expect("probe event receiver");
        })),
        menu: vec![
            MenuItem::Action {
                label: "Open & restore".into(),
                disabled: false,
                checked: None,
                default: true,
                on_select: Some(Arc::new(move || {
                    open.send("open").expect("probe event receiver");
                })),
            },
            MenuItem::Separator,
            MenuItem::submenu(
                "More",
                vec![
                    MenuItem::Action {
                        label: "Muted".into(),
                        disabled: false,
                        checked: Some(true),
                        default: false,
                        on_select: Some(Arc::new(move || {
                            muted.send("muted").expect("probe event receiver");
                        })),
                    },
                    MenuItem::Action {
                        label: "Disabled".into(),
                        disabled: true,
                        checked: None,
                        default: false,
                        on_select: None,
                    },
                ],
            ),
            MenuItem::quit(),
        ],
    }
}
#[test]
#[ignore = "requires a logged-in Windows desktop; creates only its own temporary tray/menu"]
fn owned_native_tray_supports_menus_mutation_recovery_and_teardown() {
    assert_eq!(
        std::env::var("SK_TRAY_NATIVE_PROBE").as_deref(),
        Ok("1"),
        "explicit native probe opt-in required"
    );
    let tray = Tray::new().unwrap();
    let hwnd = HWND(tray.owner.0.window.load(Ordering::Acquire) as *mut _);
    let thread = tray.owner.0.thread_id.load(Ordering::Acquire);
    struct Cleanup(Tray, HWND);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if unsafe { IsWindow(Some(self.1)) }.as_bool() {
                if let Err(error) =
                    unsafe { PostMessageW(Some(self.1), WM_CANCELMODE, WPARAM(0), LPARAM(0)) }
                {
                    eprintln!("probe menu cleanup: {error}");
                }
            }
            self.0.stop();
        }
    }
    let _cleanup = Cleanup(tray.clone(), hwnd);
    let (events, received) = mpsc::channel();
    tray.set(Some(model(&events, "first", false))).unwrap();
    send(hwnd, UPDATE, 0, 0);
    let identity = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    assert!(
        unsafe { Shell_NotifyIconGetRect(&identity) }.is_ok(),
        "Explorer did not register isolated probe icon"
    );
    send(
        hwnd,
        CALLBACK,
        0,
        ((ICON_ID << 16) | NIN_KEYSELECT) as isize,
    );
    expect_event(&received, "first");
    unsafe {
        PostMessageW(
            Some(hwnd),
            CALLBACK,
            WPARAM((24 << 16) | 24),
            LPARAM(((ICON_ID << 16) | WM_CONTEXTMENU) as isize),
        )
    }
    .unwrap();
    let menu_window = wait_popup(thread);
    let menu = HMENU(send(menu_window, MN_GETHMENU, 0, 0) as *mut _);
    assert!(!menu.0.is_null());
    assert_eq!(unsafe { GetMenuItemCount(Some(menu)) }, 4);
    assert_eq!(
        unsafe { GetMenuDefaultItem(menu, 0, GET_MENU_DEFAULT_ITEM_FLAGS(0)) },
        1
    );
    let child = unsafe { GetSubMenu(menu, 2) };
    assert!(!child.0.is_null());
    assert_ne!(
        unsafe { GetMenuState(child, 0, MF_BYPOSITION) } & MF_CHECKED.0,
        0
    );
    assert_ne!(
        unsafe { GetMenuState(child, 1, MF_BYPOSITION) } & MF_GRAYED.0,
        0
    );
    // TrackPopupMenu does not accept synthetic keyboard messages as hardware
    // input on this desktop. Cancel only our own popup; don't use global input.
    send(hwnd, WM_CANCELMODE, 0, 0);
    assert!(received.try_recv().is_err());
    unsafe {
        PostMessageW(
            Some(hwnd),
            CALLBACK,
            WPARAM((24 << 16) | 24),
            LPARAM(((ICON_ID << 16) | WM_CONTEXTMENU) as isize),
        )
    }
    .unwrap();
    let stale = wait_popup(thread);
    tray.set(Some(model(&events, "second", true))).unwrap();
    let _posted_to_old_popup =
        unsafe { PostMessageW(Some(stale), WM_KEYDOWN, WPARAM(0x0d), LPARAM(1)) };
    send(hwnd, UPDATE, 0, 0);
    assert!(
        received.try_recv().is_err(),
        "retired menu dispatched a stale selection"
    );
    send(hwnd, CALLBACK, 0, ((ICON_ID << 16) | NIN_SELECT) as isize);
    expect_event(&received, "second");
    let data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    assert!(unsafe { Shell_NotifyIconW(NIM_DELETE, &data) }.as_bool());
    let restarted = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    send(hwnd, restarted, 0, 0);
    assert!(
        unsafe { Shell_NotifyIconGetRect(&identity) }.is_ok(),
        "owned notification icon did not recover"
    );
    unsafe {
        PostMessageW(
            Some(hwnd),
            CALLBACK,
            WPARAM((24 << 16) | 24),
            LPARAM(((ICON_ID << 16) | WM_CONTEXTMENU) as isize),
        )
    }
    .unwrap();
    let _menu_window = wait_popup(thread);
    send(hwnd, WM_CANCELMODE, 0, 0);
    tray.set(None).unwrap();
    send(hwnd, UPDATE, 0, 0);
    assert!(
        unsafe { Shell_NotifyIconGetRect(&identity) }.is_err(),
        "notification icon remained after removal"
    );
    tray.stop();
    assert_eq!(
        tray.set(Some(model(&events, "closed", false))),
        Err(Error::Closed)
    );
    assert!(
        !unsafe { IsWindow(Some(hwnd)) }.as_bool(),
        "owned hidden tray window leaked"
    );
}
