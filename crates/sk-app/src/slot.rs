//! Where this process keeps the files that identify it: the dist slot.
//!
//! One place, because the dev transport (`crate::dev`, dev builds only: `manifest.json`,
//! `changes.json`, module scripts), the devtools listener (`crate::devtools`: publishes
//! `devtools.json`, scans its siblings) and the embedded inspector (`--target`) must agree on it.
//!
//! A packaged exe has no `dist/`, but its devtools port must be discoverable, so its slot is
//!
//! ```text
//!   %TEMP%\skathi\apps\<sk.config name>\<pid>\devtools.json
//! ```
//!
//! per app (the `name` that also keys its storage), per instance, always writable (unlike the exe's
//! own folder, which two copies of the app would share), and pruned of dead instances on the way in.

use std::path::{Path, PathBuf};

/// The compile-time default: the dev builder's output directory. Only a dev build has one; `cfg`'d
/// rather than `allow(dead_code)`'d so it is genuinely absent otherwise.
#[cfg(not(feature = "prod"))]
const DEV_DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/cli/dist");

/// This process's slot directory. `SK_DIST` wins everywhere: it points one exe at another app's
/// build output and keeps every parallel host (the e2e slot pool, the inspector itself) addressable.
#[cfg_attr(all(feature = "prod", not(sk_devtools)), allow(dead_code))]
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SK_DIST").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    #[cfg(not(feature = "prod"))]
    {
        PathBuf::from(DEV_DIST)
    }
    #[cfg(feature = "prod")]
    {
        prod_dir().clone()
    }
}

#[cfg_attr(all(feature = "prod", not(sk_devtools)), allow(dead_code))]
pub fn path(rel: &str) -> PathBuf {
    dir().join(rel)
}

#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn apps_root() -> PathBuf {
    std::env::temp_dir().join("skathi").join("apps")
}

#[cfg(feature = "prod")]
#[cfg_attr(not(sk_devtools), allow(dead_code))]
fn prod_dir() -> &'static PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = apps_root()
            .join(crate::payload::slot_name())
            .join(std::process::id().to_string());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("[slot] cannot create {}: {e}", dir.display());
        }
        prune_dead_slots(&dir);
        dir
    })
}

/// A crashed host leaves its slot behind, which is a discovery problem: later scans
/// (`targets_json`) would list dead instances. Pruning on the way in is the one moment we may touch
/// that directory. "No longer running":
///
/// * a slot with a handshake is answered by its port, as `read_target` does: a pid can be recycled,
///   a refused connect on loopback cannot lie;
/// * a slot with no handshake and a pid for a name (`apps\<name>\<pid>`, or an inspector's
///   `devtools\<parent pid>` before its child has published) is answered by the pid, which a live
///   process keeps even with devtools turned off.
#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn prune_dead_slots(keep: &Path) {
    let Some(root) = keep.parent() else { return };
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path == keep {
            continue;
        }
        let dead = match std::fs::read_to_string(path.join("devtools.json")) {
            Ok(text) => match crate::json::parse(&text)
                .and_then(|d| d.get("port").and_then(crate::json::Json::as_f64))
            {
                Some(port) => !port_alive(port as u16),
                None => false,
            },
            Err(_) => match entry.file_name().to_string_lossy().parse::<u32>() {
                Ok(pid) => !pid_alive(pid),
                Err(_) => false,
            },
        };
        if dead {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn port_alive(port: u16) -> bool {
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(120)).is_ok()
}

/// Is `pid` a running process? "Cannot tell" counts as alive — the cost of a
/// wrong "dead" is deleting a live instance's handshake.
fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(handle) => {
                let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
                let _ = CloseHandle(handle);
                running
            }
            // ERROR_INVALID_PARAMETER is "no such process". Access denied
            // (another user's process on a recycled pid) is NOT proof of death.
            Err(e) => e.code() != ERROR_INVALID_PARAMETER.to_hresult(),
        }
    }
    #[cfg(unix)]
    {
        // 0, or above i32::MAX, would name a process group; no process has such a pid.
        let Some(pid) = libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0) else {
            return false;
        };
        // Signal 0 checks without sending. EPERM is another user's process,
        // alive; only ESRCH is "no such process".
        let signalled = unsafe { libc::kill(pid, 0) } == 0;
        signalled || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn pruning_removes_a_dead_pid_slot_and_keeps_the_rest() {
        let base = std::env::temp_dir()
            .join("sk-slot-test")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let keep = base.join("keep");
        let dead = base.join("4294967291");
        let live = base.join(std::process::id().to_string());
        let foreign = base.join("not-a-pid");
        for d in [&keep, &dead, &live, &foreign] {
            std::fs::create_dir_all(d).unwrap();
        }
        let stale = base.join("stale");
        std::fs::create_dir_all(&stale).unwrap();
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        }; // dropped: nothing listens there now
        std::fs::write(
            stale.join("devtools.json"),
            format!("{{\"port\":{port},\"pid\":1,\"token\":\"x\"}}"),
        )
        .unwrap();

        super::prune_dead_slots(&keep);
        assert!(keep.is_dir());
        assert!(!dead.exists(), "a slot named after a dead pid is pruned");
        assert!(live.is_dir(), "a slot named after a live pid is kept");
        assert!(
            foreign.is_dir(),
            "a slot that is neither is not ours to judge"
        );
        assert!(!stale.exists(), "a handshake whose port refuses is pruned");
        let _ = std::fs::remove_dir_all(&base);
    }
}
