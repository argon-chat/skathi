//! `navigator.onLine` and the `online` / `offline` events, from the OS. As in Chromium, online means "the OS
//! believes there is a network". Windows uses the connectivity hint (iphlpapi, 2004+), resolved at run time
//! so an older Windows still starts with the state unset; Linux uses rtnetlink. [`install`] seeds the state
//! synchronously before any context exists.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sk_runtime::redjs::webapi::navigator;

pub const ONLINE_DELAY: Duration = Duration::from_millis(500);
/// Delay towards offline; the longer wait keeps a network switch (which passes through "none") from firing both.
pub const OFFLINE_DELAY: Duration = Duration::from_millis(1500);

pub fn level_is_online(level: i32) -> bool {
    level != 1
}

/// Chromium's change calculator with time passed in: every hint restarts one timer; when it fires the
/// newest hint is announced if it differs from the last.
#[derive(Debug, Clone)]
pub struct Debounce {
    announced: bool,
    pending: bool,
    deadline: Option<Instant>,
}

impl Debounce {
    pub fn new(announced: bool) -> Self {
        Debounce {
            announced,
            pending: announced,
            deadline: None,
        }
    }

    pub fn hint(&mut self, online: bool, now: Instant) {
        self.pending = online;
        let delay = if self.announced {
            OFFLINE_DELAY
        } else {
            ONLINE_DELAY
        };
        self.deadline = Some(now + delay);
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

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

/// What the OS says about the connection beyond "there is a network", for `skathi:network`.
/// `None` is "the OS cannot tell".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Details {
    /// `full`, `limited`, `portal`, `none` or `unknown`: the OS's own check, never a probe of ours.
    pub internet: &'static str,
    pub metered: Option<bool>,
    pub roaming: Option<bool>,
    pub over_data_limit: Option<bool>,
}

const UNKNOWN: Details = Details {
    internet: "unknown",
    metered: None,
    roaming: None,
    over_data_limit: None,
};

static DETAILS: Mutex<Details> = Mutex::new(UNKNOWN);
/// The connection type, looked up on the first read after a change: asking the OS takes milliseconds.
static KIND: Mutex<Option<&'static str>> = Mutex::new(None);

pub fn details() -> Details {
    *DETAILS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `ethernet`, `wifi`, `cellular`, `other`, `none` or `unknown`.
pub fn connection_type() -> &'static str {
    let mut kind = KIND.lock().unwrap_or_else(|e| e.into_inner());
    *kind.get_or_insert_with(imp::connection_type)
}

fn announce(event: &str) {
    crate::emit(format!(r#"{{"kind":"network","event":"{event}"}}"#));
}

/// Never panics: called across the Win32 ABI.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn set_details(new: Details, announce_change: bool) {
    let changed = {
        let mut details = DETAILS.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *details, new) != new
    };
    *KIND.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if changed && announce_change {
        announce("change");
    }
}

static CALC: Mutex<Option<Debounce>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn calc() -> MutexGuard<'static, Option<Debounce>> {
    CALC.lock().unwrap_or_else(|e| e.into_inner())
}

/// Never panics: called across the Win32 ABI.
fn on_hint(online: bool) {
    if let Some(d) = calc().as_mut() {
        d.hint(online, Instant::now());
        WAKE.notify_one();
    }
}

fn run() {
    let mut guard = calc();
    loop {
        let now = Instant::now();
        if let Some(online) = guard.as_mut().and_then(|d| d.fire(now)) {
            drop(guard);
            navigator::set_on_line(Some(online));
            announce(if online { "online" } else { "offline" });
            guard = calc();
            continue;
        }
        guard = match guard.as_ref().and_then(Debounce::deadline) {
            Some(d) => {
                WAKE.wait_timeout(guard, d.saturating_duration_since(now))
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            }
            None => WAKE.wait(guard).unwrap_or_else(|e| e.into_inner()),
        };
    }
}

pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(imp::install);
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;

    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };

    use super::{calc, level_is_online, navigator, on_hint, Debounce};

    /// The BOOLEANs are `u8`: the OS writes them, and a byte other than 0 or 1 must not be UB.
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
    type NotifyHint = unsafe extern "system" fn(
        cb: Callback,
        ctx: *const c_void,
        initial: u8,
        handle: *mut *mut c_void,
    ) -> u32;

    unsafe extern "system" fn callback(_ctx: *const c_void, hint: Hint) {
        super::set_details(details_of(hint), true);
        on_hint(level_is_online(hint.level));
    }

    /// `NL_NETWORK_CONNECTIVITY_LEVEL_HINT` and `NL_NETWORK_CONNECTIVITY_COST_HINT`.
    pub(super) fn details_of(hint: Hint) -> super::Details {
        let costed = hint.cost != 0;
        super::Details {
            internet: match hint.level {
                1 => "none",
                2 => "limited",
                3 => "full",
                4 => "portal",
                _ => "unknown",
            },
            metered: costed.then_some(hint.cost != 1),
            roaming: costed.then_some(hint.roaming != 0),
            over_data_limit: costed.then_some(hint.over_data_limit != 0),
        }
    }

    const IF_TYPE_ETHERNET: u32 = 6;
    const IF_TYPE_LOOPBACK: u32 = 24;
    const IF_TYPE_WIFI: u32 = 71;
    const IF_TYPE_TUNNEL: u32 = 131;
    const IF_TYPE_WWAN: u32 = 243;
    const IF_TYPE_WWAN2: u32 = 244;

    /// The type of the connected adapter with a gateway and the lowest metric: the one traffic takes.
    pub(super) fn connection_type() -> &'static str {
        use windows_sys::Win32::NetworkManagement::IpHelper::{
            GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
            GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
        };
        use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;

        const FLAGS: u32 = GAA_FLAG_INCLUDE_GATEWAYS
            | GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_DNS_SERVER;
        const ERROR_BUFFER_OVERFLOW: u32 = 111;
        let mut size = 16 * 1024u32;
        let mut buffer: Vec<u64> = Vec::new();
        for _ in 0..3 {
            buffer.resize((size as usize).div_ceil(8), 0);
            let adapters = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            // AF_UNSPEC: both address families.
            match unsafe { GetAdaptersAddresses(0, FLAGS, std::ptr::null(), adapters, &mut size) } {
                0 => {
                    let mut best: Option<((bool, u32), &str)> = None;
                    let mut cursor: *const IP_ADAPTER_ADDRESSES_LH = adapters;
                    // SAFETY: the OS wrote a null-terminated list into `buffer`, which outlives the walk.
                    while let Some(adapter) = unsafe { cursor.as_ref() } {
                        cursor = adapter.Next;
                        if adapter.OperStatus != IfOperStatusUp
                            || adapter.FirstGatewayAddress.is_null()
                            || matches!(adapter.IfType, IF_TYPE_LOOPBACK | IF_TYPE_TUNNEL)
                        {
                            continue;
                        }
                        let kind = match adapter.IfType {
                            IF_TYPE_ETHERNET => "ethernet",
                            IF_TYPE_WIFI => "wifi",
                            IF_TYPE_WWAN | IF_TYPE_WWAN2 => "cellular",
                            _ => "other",
                        };
                        // A VPN's virtual adapter rides on a physical one, which is the answer.
                        let rank = (kind == "other", adapter.Ipv4Metric.min(adapter.Ipv6Metric));
                        if best.is_none_or(|(least, _)| rank < least) {
                            best = Some((rank, kind));
                        }
                    }
                    return best.map_or("none", |(_, kind)| kind);
                }
                ERROR_BUFFER_OVERFLOW => continue,
                _ => break,
            }
        }
        "unknown"
    }

    pub(super) fn resolve() -> Option<(GetHint, NotifyHint)> {
        unsafe {
            let lib =
                LoadLibraryExW(w!("iphlpapi.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
            let get = GetProcAddress(lib, s!("GetNetworkConnectivityHint"))?;
            let notify = GetProcAddress(lib, s!("NotifyNetworkConnectivityHintChange"))?;
            Some((
                std::mem::transmute::<_, GetHint>(get),
                std::mem::transmute::<_, NotifyHint>(notify),
            ))
        }
    }

    pub(super) fn current(get: GetHint) -> Result<i32, u32> {
        let mut hint = Hint::default();
        match unsafe { get(&mut hint) } {
            0 => {
                super::set_details(details_of(hint), false);
                Ok(hint.level)
            }
            err => Err(err),
        }
    }

    const UNSET: &str =
        "so it reads true, Chromium's answer when it cannot tell, and online/offline never fire";

    pub(super) fn install() {
        let Some((get, notify)) = resolve() else {
            crate::advise_later(&format!("navigator.onLine: this Windows has no network connectivity hint (Windows 10 2004 and later have it), {UNSET}"));
            return;
        };
        let level = match current(get) {
            Ok(level) => level,
            Err(err) => {
                crate::advise_later(&format!(
                    "navigator.onLine: GetNetworkConnectivityHint failed (error {err}), {UNSET}"
                ));
                return;
            }
        };
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
        if let Err(e) = std::thread::Builder::new()
            .name("sk-net".into())
            .spawn(super::run)
        {
            crate::advise_later(&format!("navigator.onLine: the sk-net thread did not start ({e}), so onLine keeps its startup value"));
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::{calc, navigator, on_hint, Debounce};

    const UNSET: &str =
        "so it reads true, Chromium's answer when it cannot tell, and online/offline never fire";

    pub(super) fn connection_type() -> &'static str {
        "unknown"
    }

    pub(super) fn online_now() -> io::Result<bool> {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if unsafe { libc::getifaddrs(&mut list) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let wanted = (libc::IFF_UP | libc::IFF_RUNNING) as u32;
        let mut online = false;
        let mut cursor = list;
        while !cursor.is_null() && !online {
            // SAFETY: a node of the list getifaddrs returned, freed below.
            let ifa = unsafe { &*cursor };
            cursor = ifa.ifa_next;
            if ifa.ifa_addr.is_null()
                || ifa.ifa_flags & wanted != wanted
                || ifa.ifa_flags & libc::IFF_LOOPBACK as u32 != 0
            {
                continue;
            }
            online = match i32::from(unsafe { (*ifa.ifa_addr).sa_family }) {
                libc::AF_INET => true,
                libc::AF_INET6 => {
                    // SAFETY: an AF_INET6 address is a sockaddr_in6.
                    let octets = unsafe {
                        (*ifa.ifa_addr.cast::<libc::sockaddr_in6>())
                            .sin6_addr
                            .s6_addr
                    };
                    !(octets[0] == 0xfe && octets[1] & 0xc0 == 0x80)
                }
                _ => false,
            };
        }
        unsafe { libc::freeifaddrs(list) };
        Ok(online)
    }

    fn subscribe() -> io::Result<OwnedFd> {
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                libc::NETLINK_ROUTE,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a descriptor this call just opened and nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        addr.nl_groups =
            (libc::RTMGRP_LINK | libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV6_IFADDR) as u32;
        let size = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        if unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&addr as *const libc::sockaddr_nl).cast(),
                size,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }

    fn watch(fd: OwnedFd) {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let n =
                unsafe { libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
            if n < 0 {
                let error = io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EINTR) => continue,
                    Some(libc::ENOBUFS) => {}
                    _ => {
                        crate::log_later("advisory", format!("navigator.onLine: the netlink socket failed ({error}); onLine keeps its last value"));
                        return;
                    }
                }
            }
            if let Ok(online) = online_now() {
                on_hint(online);
            }
        }
    }

    pub(super) fn install() {
        let fd = match subscribe() {
            Ok(fd) => fd,
            Err(error) => {
                crate::advise_later(&format!(
                    "navigator.onLine: no rtnetlink socket ({error}), {UNSET}"
                ));
                return;
            }
        };
        let seed = match online_now() {
            Ok(online) => online,
            Err(error) => {
                crate::advise_later(&format!(
                    "navigator.onLine: getifaddrs failed ({error}), {UNSET}"
                ));
                return;
            }
        };
        *calc() = Some(Debounce::new(seed));
        navigator::set_on_line(Some(seed));
        let started = std::thread::Builder::new()
            .name("sk-netlink".into())
            .spawn(move || watch(fd))
            .and_then(|_| {
                std::thread::Builder::new()
                    .name("sk-net".into())
                    .spawn(super::run)
            });
        if let Err(e) = started {
            crate::advise_later(&format!("navigator.onLine: its threads did not start ({e}), so onLine keeps its startup value"));
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod imp {
    pub(super) fn install() {}

    pub(super) fn connection_type() -> &'static str {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_waits_chromium_s_delay_and_a_flap_inside_it_is_not_announced() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut d = Debounce::new(true);
        assert_eq!(d.fire(ms(10_000)), None, "no timer, nothing to say");

        d.hint(false, t0);
        assert_eq!(d.deadline(), Some(ms(1500)));
        assert_eq!(d.fire(ms(1499)), None);
        assert_eq!(d.fire(ms(1500)), Some(false));
        assert_eq!(d.fire(ms(1501)), None, "fired once");

        d.hint(true, ms(2000));
        assert_eq!(d.deadline(), Some(ms(2500)));
        assert_eq!(d.fire(ms(2500)), Some(true));

        d.hint(false, ms(3000));
        d.hint(true, ms(3100));
        assert_eq!(
            d.deadline(),
            Some(ms(4600)),
            "restarted, still the offline delay"
        );
        assert_eq!(d.fire(ms(4600)), None);

        d.hint(false, ms(5000));
        d.hint(true, ms(5200));
        d.hint(false, ms(5400));
        assert_eq!(d.fire(ms(6899)), None);
        assert_eq!(d.fire(ms(6900)), Some(false));

        d.hint(false, ms(7000));
        assert_eq!(d.fire(ms(7500)), None);
    }

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
        install();
        assert!(navigator::on_line_state().is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn install_seeds_navigator_on_line_from_the_interfaces() {
        let online = imp::online_now().expect("getifaddrs");
        install();
        assert_eq!(navigator::on_line_state(), Some(online));
    }
}
