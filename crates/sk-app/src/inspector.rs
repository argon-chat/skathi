//! The embedded inspector: `F12`, `--devtools` mode, and the child registry.
//!
//! F12 re-spawns this executable as `sk-app.exe --devtools --target <slot> --window <id> --parent
//! <pid>`, which inflates and runs the inspector bundle embedded in it. A second process rather
//! than a panel, so a wedged frame pump or poisoned JS bridge in the app cannot take the inspector
//! down with it; the same binary, so nothing extra ships.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::App;
use sk_runtime::JsRuntime;

const INSPECTOR_JS_DEFLATE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/devtools/app.prod.js.deflate"));

pub(crate) fn inspector_js() -> std::io::Result<String> {
    use std::io::Read;
    let mut js = String::with_capacity(INSPECTOR_JS_DEFLATE.len() * 4);
    flate2::read::DeflateDecoder::new(INSPECTOR_JS_DEFLATE).read_to_string(&mut js)?;
    Ok(js)
}

const INSPECTOR_WINDOW_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/devtools/window.json"));

pub struct Mode {
    pub target: Option<String>,
    pub window: Option<u64>,
    pub parent: Option<u32>,
}

static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();

/// `None` in an ordinary host. Read off the one argv parser (`sk_desktop::args`), which honours
/// `--devtools` only as a leading host flag and never in a protocol launch.
pub fn mode() -> Option<&'static Mode> {
    MODE.get_or_init(|| {
        let host = &sk_desktop::args::process().host;
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

pub fn is_devtools_host() -> bool {
    mode().is_some()
}

/// Resets inherited `SK_DIST` and `SK_DEVTOOLS_TARGET`. `SK_DIST` must not be the target's, or the
/// inspector would overwrite the `devtools.json` it reads.
pub fn init() {
    let Some(mode) = mode() else { return };
    if std::env::var_os("SK_DIST").is_none_or(|v| v.is_empty()) {
        let dir = default_child_slot(std::process::id());
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("SK_DIST", &dir);
    }
    if let Some(target) = &mode.target {
        std::env::set_var("SK_DEVTOOLS_TARGET", target);
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
        _ => Err("SK_DEVTOOLS_ENGINE must be auto, native or jit".into()),
    }
}

#[cfg(not(feature = "native-aot"))]
pub(crate) fn engine_without_native(
    policy: Option<&std::ffi::OsStr>,
    external: bool,
) -> Result<(), String> {
    match engine_policy(policy)? {
        EnginePolicy::Jit => Ok(()),
        EnginePolicy::Auto if !external => Ok(()),
        _ => Err("native inspector requested, but this host was built without native-aot".into()),
    }
}

pub fn boot(rt: &JsRuntime) -> Result<(), String> {
    let mode = mode();
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
    let js = format!("globalThis.__SK_DEVTOOLS_ARGS__ = {args};");
    if let Err(e) = rt.eval(&js, "sk://devtools-args.js") {
        eprintln!("[devtools-host] args: {}\n{}", e.header, e.stack);
    }
    #[cfg(feature = "native-aot")]
    match crate::native_aot::boot(rt, crate::native_aot::Kind::Inspector) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(error) => {
            return Err(format!(
                "native inspector: {error}; restart a fresh inspector with SK_DEVTOOLS_ENGINE=jit"
            ));
        }
    }
    let js = match inspector_js() {
        Ok(js) => js,
        Err(e) => {
            eprintln!("[devtools-host] inspector bundle: cannot inflate the embedded copy: {e}");
            return Ok(());
        }
    };
    if let Err(e) = rt.eval(&js, "sk://devtools.prod.js") {
        eprintln!(
            "[devtools-host] inspector bundle: {}\n{}",
            e.header, e.stack
        );
    }
    Ok(())
}

pub fn window_config() -> Option<String> {
    Some(INSPECTOR_WINDOW_JSON.to_string())
}

fn default_child_slot(key: u32) -> PathBuf {
    std::env::temp_dir()
        .join("skathi")
        .join("devtools")
        .join(key.to_string())
}

fn child_slot(parent_pid: u32, window: Option<u64>) -> PathBuf {
    let leaf = match window {
        Some(w) => format!("{parent_pid}-w{w}"),
        None => format!("{parent_pid}"),
    };
    std::env::temp_dir()
        .join("skathi")
        .join("devtools")
        .join(leaf)
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

static OPTOUT_SAID: AtomicBool = AtomicBool::new(false);
static KEY_OFF_SAID: AtomicBool = AtomicBool::new(false);

/// How long after an F12 the next one is ignored: holding F12 auto-repeats at ~30 Hz, and one
/// synthesized keystroke has been seen reaching the interceptor twice (gpui's key path).
const F12_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

thread_local! {
    static LAST_F12: std::cell::Cell<Option<(Option<u64>, std::time::Instant)>> =
        const { std::cell::Cell::new(None) };
}

/// An interceptor rather than a key binding, so an app cannot accidentally eat it: it runs before
/// action resolution and every element's key listener, in whichever window has focus. The app still
/// sees the `keydown`, as in a browser.
pub fn install_hotkey(cx: &mut App) {
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
    std::mem::forget(sub);
}

fn is_f12(k: &gpui::Keystroke) -> bool {
    k.key == "f12"
        && !k.modifiers.control
        && !k.modifiers.alt
        && !k.modifiers.platform
        && !k.modifiers.shift
}

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
                       `\"devtools\": false` in sk.config.json), so the inspector is not opened \
                       and there is no port for it to attach to. `appDevtools.enable()` — or \
                       SK_DEVTOOLS=1 in the environment — turns them back on.";
            eprintln!("[devtools] {msg}");
            sk_tree::log::record("dev", format!("[devtools] {msg}"));
        }
        return;
    }
    if !crate::window::devtools_key_enabled() {
        if !KEY_OFF_SAID.swap(true, Ordering::Relaxed) {
            let msg = "F12: SK_DEVTOOLS_KEY=off, so the inspector is not opened. (The devtools \
                       protocol itself is unaffected — the port is still there for ctl.ts and \
                       sk-test, and `appDevtools.open()` still works.)";
            eprintln!("[devtools] {msg}");
            sk_tree::log::record("dev", format!("[devtools] {msg}"));
        }
        return;
    }
    if let Some(existing) = raise_existing(window) {
        let msg = format!(
            "F12: raising the inspector already open on window {window:?} (pid {existing})"
        );
        eprintln!("[devtools] {msg}");
        sk_tree::log::record("dev", format!("[devtools] {msg}"));
        return;
    }
    match spawn(window) {
        Ok(pid) => {
            let msg = format!(
                "F12: opened the inspector (pid {pid}) on window {} — it did not take focus",
                window.map_or_else(|| "main".into(), |w| w.to_string())
            );
            eprintln!("[devtools] {msg}");
            sk_tree::log::record("dev", format!("[devtools] {msg}"));
        }
        Err(e) => {
            let msg = format!("F12: could not start the inspector: {e}");
            eprintln!("[devtools] {msg}");
            sk_tree::log::record("error", format!("[devtools] {msg}"));
        }
    }
    let _ = cx;
}

fn reap() {
    CHILDREN.with(|c| {
        c.borrow_mut()
            .retain_mut(|child| match child.proc.try_wait() {
                Ok(Some(_)) => false,
                _ => true,
            })
    });
}

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
        .name("sk-devtools-raise".into())
        .spawn(move || {
            if let Err(e) = send_raise(&slot) {
                eprintln!("[devtools] could not raise the inspector: {e}");
            }
        })
        .ok();
    Some(pid)
}

fn send_raise(slot: &Path) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};

    // `SK_DEVTOOLS_RAISE=off`, for automated runs on a machine somebody is using: everything but
    // the window coming forward still happens.
    if std::env::var("SK_DEVTOOLS_RAISE").is_ok_and(|v| v == "off") {
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
    let mut sock =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400))
            .map_err(|e| format!("connect {port}: {e}"))?;
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let req = crate::json::Json::obj(vec![
        ("token", crate::json::Json::s(token)),
        ("cmd", crate::json::Json::s("raise")),
    ])
    .to_json();
    sock.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    sock.write_all(b"\n").map_err(|e| e.to_string())?;
    sock.flush().ok();
    let mut line = String::new();
    BufReader::new(&sock)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn allow_foreground(pid: u32) {
    #[cfg(target_os = "windows")]
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = pid;
}

fn spawn(window: Option<u64>) -> Result<u32, String> {
    // The inspector is this host exe started again in `--devtools` mode; an embedded Skathi uses
    // the sk-app.exe the embedder named.
    let exe = if crate::embed::active() {
        crate::embed::inspector_exe().ok_or(
            "Skathi is embedded, so there is no host exe to start as the inspector; \
             name one with embed::Builder::inspector_exe (a sk-app.exe of this Skathi version)",
        )?
    } else {
        std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?
    };
    let target = crate::slot::dir();
    let slot = child_slot(std::process::id(), window);
    std::fs::create_dir_all(&slot).map_err(|e| format!("{}: {e}", slot.display()))?;
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
    // scenario pin). `SK_DIST` is replaced, not removed, to keep its handshake out of the target's slot.
    cmd.env("SK_DIST", &slot);
    cmd.env("SK_DEVTOOLS_TARGET", &target);
    // The inspector needs a port of its own (the parent raises it through it), so this is forced:
    // `SK_DEVTOOLS=0` in the shell would otherwise leave an unreachable child.
    cmd.env("SK_DEVTOOLS", "1");
    for name in ["SK_WINDOW", "SK_SCENARIO", "SK_APP_ROOT", "SK_PERF"] {
        cmd.env_remove(name);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    let proc = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
    let pid = proc.id();
    CHILDREN.with(|c| {
        c.borrow_mut().push(Child {
            window,
            slot,
            pid,
            proc,
        })
    });
    Ok(pid)
}

pub fn close_children() {
    CHILDREN.with(|c| {
        for child in c.borrow_mut().iter_mut() {
            let _ = child.proc.kill();
        }
        c.borrow_mut().clear();
    });
}

pub fn children() -> Vec<(Option<u64>, u32)> {
    reap();
    CHILDREN.with(|c| {
        c.borrow()
            .iter()
            .map(|child| (child.window, child.pid))
            .collect()
    })
}

static PARENT_GONE: AtomicBool = AtomicBool::new(false);

/// It probes the target's devtools port rather than the pid, which can be recycled. Three
/// consecutive refusals are required, so a host mid-restart does not make the inspector vanish.
pub fn watch_parent() {
    let Some(mode) = mode() else { return };
    let Some(target) = mode.target.clone() else {
        return;
    };
    let expect_pid = mode.parent;
    std::thread::Builder::new()
        .name("sk-devtools-parent".into())
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
    let Ok(text) = std::fs::read_to_string(slot.join("devtools.json")) else {
        return false;
    };
    let Some(doc) = crate::json::parse(&text) else {
        return false;
    };
    let Some(port) = doc.get("port").and_then(crate::json::Json::as_f64) else {
        return false;
    };
    // A handshake rewritten by a DIFFERENT process on the same slot means the
    // app we were opened for is gone even though a host is listening.
    if let (Some(want), Some(have)) = (
        expect_pid,
        doc.get("pid").and_then(crate::json::Json::as_f64),
    ) {
        if have as u32 != want {
            return false;
        }
    }
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port as u16));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
}

pub fn poll_frame(cx: &mut App) {
    if PARENT_GONE.load(Ordering::Relaxed) {
        cx.quit();
    }
}
