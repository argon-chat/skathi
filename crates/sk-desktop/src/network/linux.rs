//! Linux: rtnetlink for "there is a network", the routing table and sysfs for the connection type, and
//! NetworkManager, where it runs, for its connectivity check and the metered flag.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use futures_lite::StreamExt;

use super::Details;

const UNSET: &str =
    "so it reads true, Chromium's answer when it cannot tell, and online/offline never fire";

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

/// The interfaces of the default routes in `/proc/net/route` and `/proc/net/ipv6_route`, with their metrics.
pub(super) fn default_routes(v4: &str, v6: &str) -> Vec<(String, u32)> {
    let mut routes = Vec::new();
    for line in v4.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if let [iface, "00000000", _, flags, _, _, metric, "00000000", ..] = f[..] {
            let up = u32::from_str_radix(flags, 16).is_ok_and(|flags| flags & 1 != 0);
            if up {
                routes.push((iface.to_owned(), metric.parse().unwrap_or(0)));
            }
        }
    }
    for line in v6.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if let [dest, "00", _, _, gateway, metric, _, _, _, iface] = f[..] {
            let zero = |s: &str| s.bytes().all(|b| b == b'0');
            // The kernel's own "unreachable" default sits on lo and has no gateway.
            if zero(dest) && !zero(gateway) && iface != "lo" {
                routes.push((
                    iface.to_owned(),
                    u32::from_str_radix(metric, 16).unwrap_or(0),
                ));
            }
        }
    }
    routes
}

/// `type` is the ARPHRD number of `/sys/class/net/<iface>/type`, `uevent` the file beside it.
pub(super) fn kind_of(arp_type: &str, uevent: &str, wireless: bool) -> &'static str {
    let devtype = uevent.lines().find_map(|l| l.strip_prefix("DEVTYPE="));
    match devtype {
        Some("wlan") => "wifi",
        Some("wwan") => "cellular",
        _ if wireless => "wifi",
        // ARPHRD_ETHER with a DEVTYPE is a bridge, a bond, a VLAN or a virtual pair.
        None if arp_type.trim() == "1" => "ethernet",
        _ => "other",
    }
}

/// The interface the default route with the lowest metric leaves by.
pub(super) fn connection_type() -> &'static str {
    let Ok(v4) = std::fs::read_to_string("/proc/net/route") else {
        return "unknown";
    };
    let v6 = std::fs::read_to_string("/proc/net/ipv6_route").unwrap_or_default();
    let mut best: Option<((bool, u32), &str)> = None;
    for (iface, metric) in default_routes(&v4, &v6) {
        let dir = format!("/sys/class/net/{iface}");
        let read =
            |name: &str| std::fs::read_to_string(format!("{dir}/{name}")).unwrap_or_default();
        let wireless = std::path::Path::new(&format!("{dir}/wireless")).exists();
        let kind = kind_of(&read("type"), &read("uevent"), wireless);
        // A VPN's interface rides on a physical one, which is the answer.
        let rank = (kind == "other", metric);
        if best.is_none_or(|(least, _)| rank < least) {
            best = Some((rank, kind));
        }
    }
    best.map_or("none", |(_, kind)| kind)
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
    addr.nl_groups = (libc::RTMGRP_LINK
        | libc::RTMGRP_IPV4_IFADDR
        | libc::RTMGRP_IPV6_IFADDR
        | libc::RTMGRP_IPV4_ROUTE
        | libc::RTMGRP_IPV6_ROUTE) as u32;
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
        let n = unsafe { libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
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
        super::refresh(None, connection_type(), true);
        if let Ok(online) = online_now() {
            super::on_hint(online);
        }
    }
}

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";

/// `NMConnectivityState` and `NMMetered`.
pub(super) fn details_of(connectivity: Option<u32>, metered: Option<u32>) -> Details {
    Details {
        internet: match connectivity {
            Some(1) => "none",
            Some(2) => "portal",
            Some(3) => "limited",
            Some(4) => "full",
            _ => "unknown",
        },
        metered: match metered {
            Some(1 | 3) => Some(true),
            Some(2 | 4) => Some(false),
            _ => None,
        },
        roaming: None,
        over_data_limit: None,
    }
}

/// Follows NetworkManager for as long as it answers; returns at once on a system without it.
async fn follow_network_manager() -> zbus::Result<()> {
    let bus = zbus::Connection::system().await?;
    let manager = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&bus)
        .destination(NM)
        .path(NM_PATH)
        .interface(NM)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let properties = zbus::fdo::PropertiesProxy::builder(&bus)
        .destination(NM)
        .path(NM_PATH)
        .build()
        .await?;
    let mut changes = properties.receive_properties_changed().await?;
    let mut announce = false;
    loop {
        let connectivity = manager.get_property::<u32>("Connectivity").await?;
        let metered = manager.get_property::<u32>("Metered").await.ok();
        let details = details_of(Some(connectivity), metered);
        super::refresh(Some(details), connection_type(), announce);
        announce = true;
        if changes.next().await.is_none() {
            return Ok(());
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
    super::seed(seed);
    super::refresh(None, connection_type(), false);
    let started = std::thread::Builder::new()
        .name("sk-netlink".into())
        .spawn(move || watch(fd))
        .and_then(|_| super::start());
    if let Err(e) = started {
        crate::advise_later(&format!(
            "navigator.onLine: its threads did not start ({e}), so onLine keeps its startup value"
        ));
    }
    // Without NetworkManager the connection fails or the first read does, and the thread ends.
    let _ = std::thread::Builder::new()
        .name("sk-nm".into())
        .spawn(|| drop(async_io::block_on(follow_network_manager())));
}
