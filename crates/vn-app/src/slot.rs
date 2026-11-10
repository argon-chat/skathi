//! Where THIS process keeps the files that identify it: the dist slot.
//!
//! One place, because three subsystems have to agree on it and two of them now
//! exist in builds the third does not:
//!
//! * the dev transport reads `manifest.json` / `changes.json` / the module
//!   scripts out of it (`crate::dev`, `not(feature = "prod")` only);
//! * the devtools listener publishes its handshake `devtools.json` into it, and
//!   discovery scans its siblings (`crate::devtools`, `cfg(vn_devtools)`);
//! * the embedded inspector is pointed at a slot by `--target`.
//!
//! `--features prod` is the build where those two sets stop overlapping: there
//! is no `dist/`, no manifest and no HMR, but there IS a devtools port that has
//! to be discoverable. Until m25 the slot degraded to the exe's own directory —
//! which put a `devtools.json` beside an installed `Hello.exe` (a folder the
//! user may not be able to write, and one two copies of the app SHARE, so the
//! second instance overwrote the first one's handshake). Now a packaged exe's
//! slot is
//!
//! ```text
//!   %TEMP%\vue-native\apps\<vn.config name>\<pid>\devtools.json
//! ```
//!
//! — per app (the `name` that also keys its storage, docs/STORAGE.md §3), per
//! instance, always writable, and pruned of dead instances on the way in
//! (docs/DEVTOOLS.md "Handshake").

use std::path::{Path, PathBuf};

/// The compile-time default: the dev builder's output directory.
///
/// Only a dev build has one, which is also the only build that reads it — a
/// prod host derives its slot from its payload. `cfg`'d rather than
/// `allow(dead_code)`'d so the constant is genuinely absent rather than
/// silently unused.
#[cfg(not(feature = "prod"))]
const DEV_DIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/vue-native/dist"
);

/// This process's slot directory.
///
/// `VN_DIST` wins everywhere — it is what points one exe at another app's build
/// output, and it is how every parallel host in this repo (the e2e slot pool,
/// the two-window voice demo, the inspector itself) stays independently
/// addressable.
///
/// `prod` + `no-devtools` is the one build with no caller at all: no dev
/// transport to read a manifest, no listener to publish a handshake. The
/// function stays rather than being `cfg`'d away, because it is the definition
/// of "this process's slot" and a build that happens to ask nothing of it is
/// not a reason to make the definition conditional.
#[cfg_attr(all(feature = "prod", not(vn_devtools)), allow(dead_code))]
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VN_DIST").filter(|v| !v.is_empty()) {
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

/// A file inside this process's slot.
#[cfg_attr(all(feature = "prod", not(vn_devtools)), allow(dead_code))]
pub fn path(rel: &str) -> PathBuf {
    dir().join(rel)
}

/// `%TEMP%\vue-native\apps` — every packaged app's slots, one folder per app
/// name. Devtools discovery (`targets`, `vn-devtools ctl --app`) scans it.
#[cfg_attr(not(vn_devtools), allow(dead_code))]
pub fn apps_root() -> PathBuf {
    std::env::temp_dir().join("vue-native").join("apps")
}

/// Made once, on first ask: created, and its dead siblings pruned.
#[cfg(feature = "prod")]
#[cfg_attr(not(vn_devtools), allow(dead_code))]
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

/// Deletes the sibling slot directories of processes that are no longer
/// running.
///
/// A host that CRASHES leaves its slot behind. That is not a disk-space problem
/// — it is 60 bytes — it is a DISCOVERY problem: those slots are siblings, so
/// every later scan lists half a dozen dead instances (`targets_json`). Pruning
/// on the way in keeps the list to what is actually running, and it is the one
/// moment we are certainly allowed to touch that directory.
///
/// "No longer running":
///
/// * a slot WITH a handshake is answered by its PORT, as `read_target` does: a
///   pid can be recycled, a refused connect on loopback cannot lie, and it is
///   instant;
/// * a slot with NO handshake and a pid for a name (a packaged app's
///   `apps\<name>\<pid>`, or an inspector's `devtools\<parent pid>` before its
///   child has published) is answered by the pid — a live process keeps it
///   even if it has turned its devtools off.
///
/// Anything else is left alone: it is not ours to judge.
#[cfg_attr(not(vn_devtools), allow(dead_code))]
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
        let Some(pid) = libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0) else { return false };
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
    #[cfg(not(feature = "prod"))]
    #[test]
    fn the_dev_default_is_the_builders_output_directory() {
        // Not asserting the absolute path (it embeds the checkout), only that
        // the two halves agree about the name — `dev.rs` used to own this
        // constant and `devtools` reached into it.
        assert!(super::DEV_DIST.ends_with("packages/vue-native/dist"));
    }

    #[test]
    fn packaged_slots_live_under_the_temp_directory_per_app() {
        let root = super::apps_root();
        assert!(root.starts_with(std::env::temp_dir()));
        assert!(root.ends_with("vue-native/apps") || root.ends_with("vue-native\\apps"));
    }

    #[test]
    fn pruning_removes_a_dead_pid_slot_and_keeps_the_rest() {
        let base = std::env::temp_dir()
            .join("vn-slot-test")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let keep = base.join("keep");
        // A pid that cannot be running: Windows pids are multiples of 4, and
        // this one is far beyond any live process table.
        let dead = base.join("4294967291");
        let live = base.join(std::process::id().to_string());
        let foreign = base.join("not-a-pid");
        for d in [&keep, &dead, &live, &foreign] {
            std::fs::create_dir_all(d).unwrap();
        }
        // A handshake whose port refuses: dead regardless of the name.
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
