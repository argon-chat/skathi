//! macOS: `NWPathMonitor`, the system's view of the route traffic takes.

use std::ffi::c_void;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use block2::{Block, RcBlock};
use dispatch2::DispatchQueue;

use super::Details;

#[link(name = "Network", kind = "framework")]
extern "C" {
    fn nw_path_monitor_create() -> *mut c_void;
    fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: &DispatchQueue);
    fn nw_path_monitor_set_update_handler(
        monitor: *mut c_void,
        handler: &Block<dyn Fn(*mut c_void)>,
    );
    fn nw_path_monitor_start(monitor: *mut c_void);
    fn nw_path_get_status(path: *mut c_void) -> i32;
    fn nw_path_uses_interface_type(path: *mut c_void, kind: i32) -> bool;
    fn nw_path_is_expensive(path: *mut c_void) -> bool;
}

const STATUS_SATISFIED: i32 = 1;
const INTERFACE_WIFI: i32 = 1;
const INTERFACE_CELLULAR: i32 = 2;
const INTERFACE_WIRED: i32 = 3;

/// How long `install` waits for the monitor's first path, so `navigator.onLine` is right from the start.
const FIRST_PATH: Duration = Duration::from_millis(300);

static KIND: Mutex<&'static str> = Mutex::new("unknown");

pub(super) fn connection_type() -> &'static str {
    *KIND.lock().unwrap_or_else(|e| e.into_inner())
}

/// SAFETY: `path` is the path an update handler was called with.
unsafe fn read(path: *mut c_void) -> (bool, &'static str, Details) {
    let online = nw_path_get_status(path) == STATUS_SATISFIED;
    // A VPN's interface rides on a physical one, which is the answer.
    let kind = if !online {
        "none"
    } else if nw_path_uses_interface_type(path, INTERFACE_WIRED) {
        "ethernet"
    } else if nw_path_uses_interface_type(path, INTERFACE_WIFI) {
        "wifi"
    } else if nw_path_uses_interface_type(path, INTERFACE_CELLULAR) {
        "cellular"
    } else {
        "other"
    };
    let details = Details {
        // A satisfied path is a route, not a check that the internet answers.
        internet: if online { "unknown" } else { "none" },
        metered: online.then(|| nw_path_is_expensive(path)),
        roaming: None,
        over_data_limit: None,
    };
    (online, kind, details)
}

pub(super) fn install() {
    if let Err(e) = super::start() {
        crate::advise_later(&format!(
            "navigator.onLine: the sk-net thread did not start ({e}), so it reads true and online/offline never fire"
        ));
        return;
    }
    let (seeded, first_path) = mpsc::channel::<()>();
    let seeded = Mutex::new(Some(seeded));
    let handler = RcBlock::new(move |path: *mut c_void| {
        // SAFETY: the monitor calls this with a live path.
        let (online, kind, details) = unsafe { read(path) };
        *KIND.lock().unwrap_or_else(|e| e.into_inner()) = kind;
        let first = seeded.lock().unwrap_or_else(|e| e.into_inner()).take();
        match first {
            Some(first) => {
                super::refresh(Some(details), kind, false);
                super::seed(online);
                let _ = first.send(());
            }
            None => {
                super::refresh(Some(details), kind, true);
                super::on_hint(online);
            }
        }
    });
    let queue = DispatchQueue::new("sk-net-path", None);
    // SAFETY: the monitor copies the block and retains the queue; it runs for the life of the process.
    unsafe {
        let monitor = nw_path_monitor_create();
        nw_path_monitor_set_queue(monitor, &queue);
        nw_path_monitor_set_update_handler(monitor, &handler);
        nw_path_monitor_start(monitor);
    }
    let _ = first_path.recv_timeout(FIRST_PATH);
}
