//! `navigator.onLine` and the `online` / `offline` events, from the OS
//! (docs/DESKTOP.md §3).
//!
//! The value means what it means in Chromium: "the OS believes there is a
//! network", not "the internet is reachable". Chromium on Windows asks NLA
//! whether any network is connected. This asks the network connectivity hint
//! (iphlpapi, Windows 10 2004+). Its `None` level is the same "no network at
//! all", and `LocalAccess`, a LAN with no internet, is online in both.
//!
//! ```text
//!   iphlpapi: NotifyNetworkConnectivityHintChange ── thread-pool callback ──┐
//!                                                                         ▼
//!   vn-net thread: Chromium's change calculator (0.5 s to go online, 1.5 s to
//!                  go offline, restarted by every hint)                   │
//!                                                                         ▼
//!   redjs navigator::set_on_line ── a queue per context ── its pump ── Event("online"/"offline")
//! ```
//!
//! [`install`] seeds the state synchronously, before any context exists, so
//! the bundle's first read of `navigator.onLine` is the OS's answer and no
//! event is queued for the seed. Both functions are resolved at run time, so
//! an older Windows still starts. There the state stays unset: redjs answers
//! `true`, Chromium's answer when it cannot tell, and an advisory says so.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use vn_runtime::redjs::webapi::navigator;

/// How long a change towards online waits before it is announced. Chromium's
/// `NetworkChangeNotifierWin` calculator: `connection_type_online_delay_`.
pub const ONLINE_DELAY: Duration = Duration::from_millis(500);
/// How long a change towards offline waits: `connection_type_offline_delay_`.
/// The longer wait is what keeps a switch between two networks, which
/// passes through "none" for a moment, from firing `offline` + `online`.
pub const OFFLINE_DELAY: Duration = Duration::from_millis(1500);

/// `NL_NETWORK_CONNECTIVITY_LEVEL_HINT` to `onLine`. Only `None` (1) is
/// offline. `Unknown` (0) and `Hidden` (5) mean the OS cannot tell this
/// process, so they read as online, as Chromium reads "cannot tell".
pub fn level_is_online(level: i32) -> bool {
    level != 1
}

/// Chromium's `NetworkChangeCalculator` for the connection type, with time
/// passed in. Every hint restarts one timer. Its length depends on what was
/// last announced: [`ONLINE_DELAY`] after an offline, [`OFFLINE_DELAY`]
/// otherwise. When the timer fires, the newest hint is announced if it
/// differs from the last announcement. Chromium also notifies a change
/// between two connected types, but Blink fires `online` / `offline` only
/// when the boolean flips, so a no-op is dropped here.
#[derive(Debug, Clone)]
pub struct Debounce {
    announced: bool,
    pending: bool,
    deadline: Option<Instant>,
}

impl Debounce {
    /// Starts from the state that was announced (seeded) already.
    pub fn new(announced: bool) -> Self {
        Debounce { announced, pending: announced, deadline: None }
    }

    /// A hint arrived at `now`.
    pub fn hint(&mut self, online: bool, now: Instant) {
        self.pending = online;
        let delay = if self.announced { OFFLINE_DELAY } else { ONLINE_DELAY };
        self.deadline = Some(now + delay);
    }

    /// When the timer fires, if one is running.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// The timer at `now`: `Some(state)` when a change is to be announced.
    pub fn fire(&mut self, now: Instant) -> Option<bool> {
        match self.deadline {
            Some(d) if now >= d => self.deadline = None,
            _ => return None,
        }
        if self.pending == self.announced {
            return None;
        }
        self.announced = self.pending;
        Some(self.announced)
    }
}

/// The calculator the callback feeds and the `vn-net` thread drains. `None`
/// until [`install`] has seeded it, and again if the subscription failed.
static CALC: Mutex<Option<Debounce>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn calc() -> MutexGuard<'static, Option<Debounce>> {
    CALC.lock().unwrap_or_else(|e| e.into_inner())
}

/// A hint, from the OS's thread pool. Never panics: it is called across the
/// Win32 ABI.
fn on_hint(online: bool) {
    if let Some(d) = calc().as_mut() {
        d.hint(online, Instant::now());
        WAKE.notify_one();
    }
}

/// The `vn-net` thread: waits for the calculator's timer and announces.
fn run() {
    let mut guard = calc();
    loop {
        let now = Instant::now();
        if let Some(online) = guard.as_mut().and_then(|d| d.fire(now)) {
            drop(guard);
            navigator::set_on_line(Some(online));
            guard = calc();
            continue;
        }
        guard = match guard.as_ref().and_then(Debounce::deadline) {
            Some(d) => WAKE.wait_timeout(guard, d.saturating_duration_since(now)).unwrap_or_else(|e| e.into_inner()).0,
            None => WAKE.wait(guard).unwrap_or_else(|e| e.into_inner()),
        };
    }
}

/// Starts the provider behind `navigator.onLine`. Once per process; a second
/// call does nothing. vn-app calls it before `JsRuntime::init`.
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(imp::install);
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;

    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

    use super::{calc, level_is_online, navigator, on_hint, Debounce};

    /// `NL_NETWORK_CONNECTIVITY_HINT`. The BOOLEANs are `u8`, not `bool`: the
    /// OS writes them, and a byte other than 0 or 1 must not be UB.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct Hint {
        pub level: i32,
        pub cost: i32,
        pub approaching_data_limit: u8,
        pub over_data_limit: u8,
        pub roaming: u8,
    }

    type Callback = unsafe extern "system" fn(ctx: *const c_void, hint: Hint);
    type GetHint = unsafe extern "system" fn(hint: *mut Hint) -> u32;
    type NotifyHint = unsafe extern "system" fn(cb: Callback, ctx: *const c_void, initial: u8, handle: *mut *mut c_void) -> u32;

    unsafe extern "system" fn callback(_ctx: *const c_void, hint: Hint) {
        on_hint(level_is_online(hint.level));
    }

    /// Both functions, or `None` before Windows 10 2004. The library is never
    /// freed: the subscription lives as long as the process.
    pub(super) fn resolve() -> Option<(GetHint, NotifyHint)> {
        unsafe {
            let lib = LoadLibraryExW(w!("iphlpapi.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
            let get = GetProcAddress(lib, s!("GetNetworkConnectivityHint"))?;
            let notify = GetProcAddress(lib, s!("NotifyNetworkConnectivityHintChange"))?;
            Some((std::mem::transmute::<_, GetHint>(get), std::mem::transmute::<_, NotifyHint>(notify)))
        }
    }

    /// The level now, or the error code.
    pub(super) fn current(get: GetHint) -> Result<i32, u32> {
        let mut hint = Hint::default();
        match unsafe { get(&mut hint) } {
            0 => Ok(hint.level),
            err => Err(err),
        }
    }

    const UNSET: &str = "so it reads true, Chromium's answer when it cannot tell, and online/offline never fire";

    pub(super) fn install() {
        let Some((get, notify)) = resolve() else {
            crate::advise_later(&format!("navigator.onLine: this Windows has no network connectivity hint (Windows 10 2004 and later have it), {UNSET}"));
            return;
        };
        let level = match current(get) {
            Ok(level) => level,
            Err(err) => {
                crate::advise_later(&format!("navigator.onLine: GetNetworkConnectivityHint failed (error {err}), {UNSET}"));
                return;
            }
        };
        // Seed first, then subscribe. A hint that arrives in between already
        // finds the calculator, and the initial notification closes the gap
        // between the read and the subscription.
        let seed = level_is_online(level);
        *calc() = Some(Debounce::new(seed));
        navigator::set_on_line(Some(seed));
        let mut handle: *mut c_void = std::ptr::null_mut();
        let err = unsafe { notify(callback, std::ptr::null(), 1, &mut handle) };
        if err != 0 {
            *calc() = None;
            navigator::set_on_line(None);
            crate::advise_later(&format!("navigator.onLine: NotifyNetworkConnectivityHintChange failed (error {err}), {UNSET}"));
            return;
        }
        if let Err(e) = std::thread::Builder::new().name("vn-net".into()).spawn(super::run) {
            crate::advise_later(&format!("navigator.onLine: the vn-net thread did not start ({e}), so onLine keeps its startup value"));
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    /// No provider: redjs answers `true` and advises on the first read.
    pub(super) fn install() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_none_level_is_offline() {
        // Unknown, None, LocalAccess, InternetAccess, ConstrainedInternetAccess, Hidden.
        let levels: Vec<bool> = (0..=5).map(level_is_online).collect();
        assert_eq!(levels, [true, false, true, true, true, true]);
    }

    #[test]
    fn a_change_waits_chromium_s_delay_and_a_flap_inside_it_is_not_announced() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut d = Debounce::new(true);
        assert_eq!(d.fire(ms(10_000)), None, "no timer, nothing to say");

        // Online → offline waits 1.5 s.
        d.hint(false, t0);
        assert_eq!(d.deadline(), Some(ms(1500)));
        assert_eq!(d.fire(ms(1499)), None);
        assert_eq!(d.fire(ms(1500)), Some(false));
        assert_eq!(d.fire(ms(1501)), None, "fired once");

        // Offline → online waits 0.5 s.
        d.hint(true, ms(2000));
        assert_eq!(d.deadline(), Some(ms(2500)));
        assert_eq!(d.fire(ms(2500)), Some(true));

        // A switch between networks passes through "none": the second hint
        // restarts the timer, and at the end nothing changed.
        d.hint(false, ms(3000));
        d.hint(true, ms(3100));
        assert_eq!(d.deadline(), Some(ms(4600)), "restarted, still the offline delay");
        assert_eq!(d.fire(ms(4600)), None);

        // The newest hint wins.
        d.hint(false, ms(5000));
        d.hint(true, ms(5200));
        d.hint(false, ms(5400));
        assert_eq!(d.fire(ms(6899)), None);
        assert_eq!(d.fire(ms(6900)), Some(false));

        // Repeating the announced state is not a change.
        d.hint(false, ms(7000));
        assert_eq!(d.fire(ms(7500)), None);
    }

    /// The real OS: the hint resolves on this Windows, and installing seeds
    /// redjs with its answer before anything has to ask.
    #[cfg(target_os = "windows")]
    #[test]
    fn install_seeds_navigator_on_line_from_the_os() {
        let Some((get, _)) = imp::resolve() else {
            eprintln!("skipped: no network connectivity hint on this Windows");
            return;
        };
        let level = imp::current(get).expect("GetNetworkConnectivityHint");
        assert!((0..=5).contains(&level), "a known level: {level}");
        install();
        assert_eq!(navigator::on_line_state(), Some(level_is_online(level)));
        install(); // once per process
        assert!(navigator::on_line_state().is_some());
    }
}
