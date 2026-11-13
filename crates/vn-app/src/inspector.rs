//! The embedded inspector: `F12`, `--devtools` mode, and the child registry.
//!
//! F12 re-spawns this executable as `vn-app.exe --devtools --target <slot> --window <id> --parent
//! <pid>`, which inflates and runs the inspector bundle embedded in it. A second process rather
//! than a panel, so a wedged frame pump or poisoned JS bridge in the app cannot take the inspector
//! down with it; the same binary, so nothing extra ships.
//!
//! The child's window opens unfocused (`focus: false`) and never activates, so F12 does not take the
//! caret from whatever you were typing into. Pressing F12 again raises the existing inspector: the
//! app calls `AllowSetForegroundWindow(child pid)` and asks the child over the devtools wire to
//! activate itself, since Windows would otherwise refuse the child's `SetForegroundWindow`.

use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::App;
use vn_runtime::JsRuntime;

/// The inspector, prod-bundled by `build.rs` from `apps/devtools` and stored as raw deflate.
const INSPECTOR_JS_DEFLATE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/devtools/app.prod.js.deflate"));

/// The bundle's source, inflated for the one eval that needs it and dropped
/// after it.
pub(crate) fn inspector_js() -> std::io::Result<String> {
    use std::io::Read;
    let mut js = String::with_capacity(INSPECTOR_JS_DEFLATE.len() * 4);
    flate2::read::DeflateDecoder::new(INSPECTOR_JS_DEFLATE).read_to_string(&mut js)?;
    Ok(js)
}

/// Its `vn.config.json` window block, emitted beside the bundle by `prod.ts`.
const INSPECTOR_WINDOW_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/devtools/window.json"));

/// What this process was asked to inspect when started with `--devtools`.
pub struct Mode {
    /// The slot directory of the app under inspection.
    pub target: Option<String>,
    /// The gpui window id F12 was pressed in, when the target has more than one.
    pub window: Option<u64>,
    /// The pid to die with.
    pub parent: Option<u32>,
}

static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();

/// `None` in an ordinary host. Read off the one argv parser (`vn_desktop::args`), which honours
/// `--devtools` only as a leading host flag and never in a protocol launch.
pub fn mode() -> Option<&'static Mode> {
    MODE.get_or_init(|| {
        let host = &vn_desktop::args::process().host;
        if !host.devtools {
            return None;
        }
        Some(Mode {
            target: host.target.clone(),
            window: host.window.as_deref().and_then(|v| v.parse().ok()),
            parent: host.parent.as_deref().and_then(|v| v.parse().ok()),
        })
    })
    .as_ref()
}

/// True when this process IS an inspector.
pub fn is_devtools_host() -> bool {
    mode().is_some()
}

/// Sets up the environment a `--devtools` process needs; called first thing in `main`.
///
/// Both variables would otherwise be inherited from the spawning app and be wrong:
/// * `VN_DIST`: the slot this process publishes its own handshake into. It must not be the target's,
///   or the inspector would overwrite the `devtools.json` it reads. [`open_for_window`] always
///   passes one; this is the fallback for `vn-app --devtools` typed by hand.
/// * `VN_DEVTOOLS_TARGET`: what `--target` means to `__vn_devtools_targets()`.
pub fn init() {
    let Some(mode) = mode() else { return };
    if std::env::var_os("VN_DIST").is_none_or(|v| v.is_empty()) {
        let dir = default_child_slot(std::process::id());
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("VN_DIST", &dir);
    }
    if let Some(target) = &mode.target {
        std::env::set_var("VN_DEVTOOLS_TARGET", target);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnginePolicy {
    Auto,
    Native,
    Jit,
}
pub(crate) fn engine_policy(value: Option<&OsStr>) -> Result<EnginePolicy, String> {
    match value.and_then(OsStr::to_str) {
        None if value.is_none() => Ok(EnginePolicy::Auto),
        Some("auto") => Ok(EnginePolicy::Auto),
        Some("native") => Ok(EnginePolicy::Native),
        Some("jit") => Ok(EnginePolicy::Jit),
        _ => Err("VN_DEVTOOLS_ENGINE must be auto, native or jit".into()),
    }
}

#[cfg(not(feature = "native-aot"))]
pub(crate) fn engine_without_native(policy: Option<&std::ffi::OsStr>, external: bool) -> Result<(), String> {
    match engine_policy(policy)? {
        EnginePolicy::Jit => Ok(()),
        EnginePolicy::Auto if !external => Ok(()),
        _ => Err("native inspector requested, but this host was built without native-aot".into()),
    }
}

/// Evaluates the embedded inspector. The bundle carries its own stylesheet and mounts itself, like
/// [`crate::prod::boot`].
pub fn boot(rt: &JsRuntime) -> Result<(), String> {
    let mode = mode();
    // What the UI cannot derive: which window of the target it was opened for, and whether it is a
    // child (which adds a "close with the app" affordance).
    let args = crate::json::Json::obj(vec![
        (
            "target",
            match mode.and_then(|m| m.target.as_deref()) {
                Some(t) => crate::json::Json::s(t),
                None => crate::json::Json::Null,
            },
        ),
        (
            "window",
            match mode.and_then(|m| m.window) {
                Some(w) => crate::json::Json::n(w as f64),
                None => crate::json::Json::Null,
            },
        ),
        (
            "parent",
            match mode.and_then(|m| m.parent) {
                Some(p) => crate::json::Json::n(f64::from(p)),
                None => crate::json::Json::Null,
            },
        ),
        ("embedded", crate::json::Json::Bool(true)),
    ])
    .to_json();
    let js = format!("globalThis.__VN_DEVTOOLS_ARGS__ = {args};");
    if let Err(e) = rt.eval(&js, "vn://devtools-args.js") {
        eprintln!("[devtools-host] args: {}\n{}", e.header, e.stack);
    }
    #[cfg(feature = "native-aot")]
    match crate::native_aot::boot(rt) {
        Ok(true) => return Ok(()),
        Ok(false) => {},
        Err(error) => {
            return Err(format!("native inspector: {error}; restart a fresh inspector with VN_DEVTOOLS_ENGINE=jit"));
        }
    }
    let js = match inspector_js() {
        Ok(js) => js,
        Err(e) => {
            eprintln!("[devtools-host] inspector bundle: cannot inflate the embedded copy: {e}");
            return Ok(());
        }
    };
    if let Err(e) = rt.eval(&js, "vn://devtools.prod.js") {
        eprintln!("[devtools-host] inspector bundle: {}\n{}", e.header, e.stack);
    }
    Ok(())
}

/// The inspector's window block, baked in beside the bundle.
pub fn window_config() -> Option<String> {
    Some(INSPECTOR_WINDOW_JSON.to_string())
}

/// `%LOCALAPPDATA%\Temp\vue-native\devtools\<key>`: where a child publishes its handshake.
/// Deterministic, so the parent can read it back to raise the window instead of opening a second.
fn default_child_slot(key: u32) -> PathBuf {
    std::env::temp_dir().join("vue-native").join("devtools").join(key.to_string())
}

fn child_slot(parent_pid: u32, window: Option<u64>) -> PathBuf {
    let leaf = match window {
        Some(w) => format!("{parent_pid}-w{w}"),
        None => format!("{parent_pid}"),
    };
    std::env::temp_dir().join("vue-native").join("devtools").join(leaf)
}

struct Child {
    window: Option<u64>,
    slot: PathBuf,
    pid: u32,
    proc: std::process::Child,
}

thread_local! {
    static CHILDREN: std::cell::RefCell<Vec<Child>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Have we already told the user the devtools are off in this process?
static OPTOUT_SAID: AtomicBool = AtomicBool::new(false);
/// Likewise for the key being off: separate reasons, and a process can hit both.
static KEY_OFF_SAID: AtomicBool = AtomicBool::new(false);

/// How long after an F12 the next one is ignored: holding F12 auto-repeats at ~30 Hz, and one
/// synthesized keystroke has been seen reaching the interceptor twice (gpui's key path).
const F12_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

thread_local! {
    static LAST_F12: std::cell::Cell<Option<(Option<u64>, std::time::Instant)>> =
        const { std::cell::Cell::new(None) };
}

/// Installs the F12 key; called once from `main`, before any window exists.
///
/// An interceptor rather than a key binding, so an app cannot accidentally eat it: it runs before
/// action resolution and every element's key listener, in whichever window has focus. The app still
/// sees the `keydown`, as in a browser.
pub fn install_hotkey(cx: &mut App) {
    // An inspector does not open an inspector on F12.
    if is_devtools_host() {
        return;
    }
    let sub = cx.intercept_keystrokes(|event, window, cx| {
        if !is_f12(&event.keystroke) {
            return;
        }
        let id = window.window_handle().window_id().as_u64();
        open_for_window(Some(id), cx);
    });
    // The subscription lives as long as the process; nothing ever unbinds F12.
    std::mem::forget(sub);
}

/// F12 with no modifiers; `Shift+F12` and friends stay bindable by apps.
fn is_f12(k: &gpui::Keystroke) -> bool {
    k.key == "f12"
        && !k.modifiers.control
        && !k.modifiers.alt
        && !k.modifiers.platform
        && !k.modifiers.shift
}

/// Opens (or raises) the inspector for one of this process's windows.
pub fn open_for_window(window: Option<u64>, cx: &mut App) {
    let now = std::time::Instant::now();
    let repeat = LAST_F12.with(|c| match c.get() {
        Some((w, at)) if w == window && now.duration_since(at) < F12_DEBOUNCE => true,
        _ => {
            c.set(Some((window, now)));
            false
        }
    });
    if repeat {
        return;
    }
    // Two gates (see `crate::window::Config`): the master switch first (the app turned its devtools
    // off, so there is no listener to attach to), then the key, which a harness can turn off alone.
    if !crate::devtools::enabled() {
        if !OPTOUT_SAID.swap(true, Ordering::Relaxed) {
            let msg = "F12: this app has its devtools disabled (`appDevtools.disable()`, or \
                       `\"devtools\": false` in vn.config.json), so the inspector is not opened \
                       and there is no port for it to attach to. `appDevtools.enable()` — or \
                       VN_DEVTOOLS=1 in the environment — turns them back on.";
            eprintln!("[devtools] {msg}");
            vn_tree::log::record("dev", format!("[devtools] {msg}"));
        }
        return;
    }
    if !crate::window::devtools_key_enabled() {
        if !KEY_OFF_SAID.swap(true, Ordering::Relaxed) {
            let msg = "F12: VN_DEVTOOLS_KEY=off, so the inspector is not opened. (The devtools \
                       protocol itself is unaffected — the port is still there for ctl.ts and \
                       vn-test, and `appDevtools.open()` still works.)";
            eprintln!("[devtools] {msg}");
            vn_tree::log::record("dev", format!("[devtools] {msg}"));
        }
        return;
    }
    if let Some(existing) = raise_existing(window) {
        let msg = format!("F12: raising the inspector already open on window {window:?} (pid {existing})");
        eprintln!("[devtools] {msg}");
        vn_tree::log::record("dev", format!("[devtools] {msg}"));
        return;
    }
    match spawn(window) {
        Ok(pid) => {
            let msg = format!(
                "F12: opened the inspector (pid {pid}) on window {} — it did not take focus",
                window.map_or_else(|| "main".into(), |w| w.to_string())
            );
            eprintln!("[devtools] {msg}");
            vn_tree::log::record("dev", format!("[devtools] {msg}"));
        }
        Err(e) => {
            let msg = format!("F12: could not start the inspector: {e}");
            eprintln!("[devtools] {msg}");
            vn_tree::log::record("error", format!("[devtools] {msg}"));
        }
    }
    let _ = cx;
}

/// Reaps children that exited on their own, so the next F12 opens a new one.
fn reap() {
    CHILDREN.with(|c| {
        c.borrow_mut().retain_mut(|child| match child.proc.try_wait() {
            Ok(Some(_)) => false,
            _ => true,
        })
    });
}

/// `Some(pid)` when an inspector for this window already exists and was asked
/// to come forward.
fn raise_existing(window: Option<u64>) -> Option<u32> {
    reap();
    let (pid, slot) = CHILDREN.with(|c| {
        c.borrow()
            .iter()
            .find(|child| child.window == window)
            .map(|child| (child.pid, child.slot.clone()))
    })?;
    // We are the foreground process, so we may hand the foreground to the child; otherwise its
    // `SetForegroundWindow` is refused and its taskbar button flashes.
    allow_foreground(pid);
    // Off-thread: a TCP round trip on the frame pump's thread would hitch the app being debugged.
    std::thread::Builder::new()
        .name("vn-devtools-raise".into())
        .spawn(move || {
            if let Err(e) = send_raise(&slot) {
                eprintln!("[devtools] could not raise the inspector: {e}");
            }
        })
        .ok();
    Some(pid)
}

/// `{"cmd":"raise"}` to the child's own devtools port, read out of the
/// handshake it published into the slot we gave it.
fn send_raise(slot: &Path) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};

    // `VN_DEVTOOLS_RAISE=off`, for automated runs on a machine somebody is using: everything but
    // the window coming forward still happens.
    if std::env::var("VN_DEVTOOLS_RAISE").is_ok_and(|v| v == "off") {
        return Ok(());
    }
    let text = std::fs::read_to_string(slot.join("devtools.json"))
        .map_err(|e| format!("{}: {e}", slot.join("devtools.json").display()))?;
    let doc = crate::json::parse(&text).ok_or("the inspector's handshake is not JSON")?;
    let port = doc
        .get("port")
        .and_then(crate::json::Json::as_f64)
        .ok_or("the inspector's handshake has no port")? as u16;
    let token = doc
        .get("token")
        .and_then(crate::json::Json::as_str)
        .ok_or("the inspector's handshake has no token")?;
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let mut sock = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400))
        .map_err(|e| format!("connect {port}: {e}"))?;
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    let req = crate::json::Json::obj(vec![
        ("token", crate::json::Json::s(token)),
        ("cmd", crate::json::Json::s("raise")),
    ])
    .to_json();
    sock.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    sock.write_all(b"\n").map_err(|e| e.to_string())?;
    sock.flush().ok();
    let mut line = String::new();
    BufReader::new(&sock).read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(())
}

/// `AllowSetForegroundWindow(pid)` — no-op off Windows.
fn allow_foreground(pid: u32) {
    #[cfg(target_os = "windows")]
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = pid;
}

/// Starts a new inspector for `window`.
fn spawn(window: Option<u64>) -> Result<u32, String> {
    // The inspector is this host exe started again in `--devtools` mode; an embedded vue-native uses
    // the vn-app.exe the embedder named.
    let exe = if crate::embed::active() {
        crate::embed::inspector_exe().ok_or(
            "vue-native is embedded, so there is no host exe to start as the inspector; \
             name one with embed::Builder::inspector_exe (a vn-app.exe of this vue-native version)",
        )?
    } else {
        std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?
    };
    let target = crate::slot::dir();
    let slot = child_slot(std::process::id(), window);
    std::fs::create_dir_all(&slot).map_err(|e| format!("{}: {e}", slot.display()))?;
    // Prune slots left behind by crashed children.
    crate::slot::prune_dead_slots(&slot);

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--devtools")
        .arg("--target")
        .arg(&target)
        .arg("--parent")
        .arg(std::process::id().to_string());
    if let Some(w) = window {
        cmd.arg("--window").arg(w.to_string());
    }
    // Every inherited setting here would make the child the wrong host (our dist slot, window size,
    // scenario pin). `VN_DIST` is replaced, not removed, to keep its handshake out of the target's slot.
    cmd.env("VN_DIST", &slot);
    cmd.env("VN_DEVTOOLS_TARGET", &target);
    // The inspector needs a port of its own (the parent raises it through it), so this is forced:
    // `VN_DEVTOOLS=0` in the shell would otherwise leave an unreachable child.
    cmd.env("VN_DEVTOOLS", "1");
    for name in ["VN_WINDOW", "VN_SCENARIO", "VN_APP_ROOT", "VN_PERF"] {
        cmd.env_remove(name);
    }
    cmd.stdin(std::process::Stdio::null());
    // Inheriting stderr would interleave two hosts' logs in one terminal.
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    let proc = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
    let pid = proc.id();
    CHILDREN.with(|c| c.borrow_mut().push(Child { window, slot, pid, proc }));
    Ok(pid)
}

/// Kills every inspector this process opened; called from `App::on_app_quit`.
pub fn close_children() {
    CHILDREN.with(|c| {
        for child in c.borrow_mut().iter_mut() {
            let _ = child.proc.kill();
        }
        c.borrow_mut().clear();
    });
}

/// How many inspectors this process has open, for the `windows` command and the e2e specs.
pub fn children() -> Vec<(Option<u64>, u32)> {
    reap();
    CHILDREN.with(|c| c.borrow().iter().map(|child| (child.window, child.pid)).collect())
}

static PARENT_GONE: AtomicBool = AtomicBool::new(false);

/// Starts the watch that closes this inspector when the inspected app exits (`--devtools` mode).
///
/// It probes the target's devtools port rather than the pid, which can be recycled. Three
/// consecutive refusals are required, so a host mid-restart does not make the inspector vanish.
pub fn watch_parent() {
    let Some(mode) = mode() else { return };
    let Some(target) = mode.target.clone() else { return };
    let expect_pid = mode.parent;
    std::thread::Builder::new()
        .name("vn-devtools-parent".into())
        .spawn(move || {
            let dir = PathBuf::from(&target);
            let mut misses = 0u32;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                if parent_alive(&dir, expect_pid) {
                    misses = 0;
                    continue;
                }
                misses += 1;
                if misses >= 3 {
                    eprintln!(
                        "[devtools-host] the app being inspected ({}) is gone — closing",
                        dir.display()
                    );
                    PARENT_GONE.store(true, Ordering::Relaxed);
                    return;
                }
            }
        })
        .ok();
}

fn parent_alive(slot: &Path, expect_pid: Option<u32>) -> bool {
    let Ok(text) = std::fs::read_to_string(slot.join("devtools.json")) else { return false };
    let Some(doc) = crate::json::parse(&text) else { return false };
    let Some(port) = doc.get("port").and_then(crate::json::Json::as_f64) else { return false };
    // A handshake rewritten by a DIFFERENT process on the same slot means the
    // app we were opened for is gone even though a host is listening.
    if let (Some(want), Some(have)) =
        (expect_pid, doc.get("pid").and_then(crate::json::Json::as_f64))
    {
        if have as u32 != want {
            return false;
        }
    }
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port as u16));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
}

/// Called once per frame. Quits when the watch above decided the app is gone.
pub fn poll_frame(cx: &mut App) {
    if PARENT_GONE.load(Ordering::Relaxed) {
        cx.quit();
    }
}

#[cfg(all(test, not(feature = "native-aot")))]
mod engine_policy_tests {
    use super::*;
    #[test]
    fn unsupported_native_selection_fails_before_vm_init_but_jit_is_explicit() {
        assert!(engine_without_native(None, false).is_ok());
        assert!(engine_without_native(Some(OsStr::new("auto")), false).is_ok());
        assert!(engine_without_native(Some(OsStr::new("native")), false).is_err());
        assert!(engine_without_native(None, true).is_err());
        assert!(engine_without_native(Some(OsStr::new("jit")), true).is_ok());
        assert!(engine_without_native(Some(OsStr::new("JIT")), false).is_err());
    }
}
