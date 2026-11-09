//! The EMBEDDED inspector: `F12`, `--devtools` mode, and the child registry.
//!
//! ```text
//!   the app                                    the inspector
//!  ┌──────────────────────────────┐           ┌──────────────────────────────┐
//!  │ user presses F12             │  spawn    │ vn-app.exe --devtools        │
//!  │  intercept_keystrokes ───────┼──────────▶│   --target <slot>            │
//!  │                              │  (no      │   --window <id>              │
//!  │ devtools listener :49xxx ◀───┼───────────│   --parent <pid>             │
//!  └──────────────────────────────┘  focus)   │ inflates and runs the bundle │
//!                                             │ embedded in THIS executable  │
//!                                             └──────────────────────────────┘
//! ```
//!
//! ## Why a second process and not a panel
//!
//! [`docs/DEVTOOLS-UI.md`] §1 argued this once and the argument has not moved:
//! the failures worth inspecting are the ones that stop the app — a wedged
//! frame pump, a poisoned JS bridge, a restyle that never terminates — and a
//! panel drawn by that machinery goes down with it. What is new in v2 is only
//! *where the inspector comes from*: it used to be a second dist folder that
//! `bun devtools/ui.ts` built and launched, and it is now a bundle inside this
//! very executable, so `F12` needs nothing on disk and works in a shipped app
//! built with `--features prod` — which since m22 is every shipped app, unless
//! its own JS turned the devtools off (docs/DEVTOOLS.md).
//!
//! Re-spawning **ourselves** is what makes that cheap. There is no second
//! binary to ship, no second build to keep in step, and no multiwindow refactor
//! (docs/MULTIWINDOW.md §5: ~25 thread-locals would have to grow a window key
//! before an inspector could live in a second window of this process). The
//! child is an ordinary vue-native host whose app happens to be the inspector,
//! which is also why every last thing in DEVTOOLS-UI.md §§2-5 still applies to
//! it unchanged.
//!
//! ## Focus
//!
//! The child's window opens with `focus: false` (`SW_SHOWNOACTIVATE` on
//! Windows) and the child never calls `cx.activate`. Pressing F12 therefore
//! does not take the caret out of whatever you were typing into — which is the
//! whole reason a browser's devtools opens docked and a second window is
//! normally a nuisance.
//!
//! Pressing F12 **again** raises the inspector that already exists, and that is
//! a focus change the user just asked for. It goes through the OS's own
//! handshake for handing the foreground on: the app is the foreground process,
//! so it calls `AllowSetForegroundWindow(child pid)` and then asks the child to
//! activate itself over the devtools wire. Without that call Windows would
//! refuse the child's `SetForegroundWindow` and flash its taskbar button
//! instead.

use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::App;
use vn_runtime::JsRuntime;

/// The inspector, prod-bundled by `build.rs` out of `apps/devtools` and
/// stored as raw deflate (`build.rs` `deflate_bundle`): ~340 KB of JS that
/// only an inspector process ever runs, in every binary that has devtools.
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

// ---------------------------------------------------------------------------
// `--devtools` mode
// ---------------------------------------------------------------------------

/// What this process was asked to inspect, when it was started with
/// `--devtools`.
pub struct Mode {
    /// The slot directory of the app under inspection.
    pub target: Option<String>,
    /// The gpui window id F12 was pressed in, when the target has more than one.
    pub window: Option<u64>,
    /// The pid to die with.
    pub parent: Option<u32>,
}

static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();

/// `None` in an ordinary host, which is every host that was not started by
/// [`open_for_window`] or by hand with `--devtools`.
///
/// Read off the ONE argv parser (`vn_desktop::args`), never off argv directly.
/// Until m25 this was `argv.iter().any(|a| a == "--devtools")` — `--devtools`
/// ANYWHERE flipped the process into the inspector, which a custom-protocol
/// link (`myapp://x" --devtools "…`) could have reached. The parser honours it
/// only as a LEADING host flag, and never in a protocol launch
/// (docs/LAUNCH.md §2, research/reports/m25/desktop-apis-design.md finding 6).
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

/// Sets up the environment a `--devtools` process needs, before anything reads
/// it. Called first thing in `main`.
///
/// Two variables, both of which would otherwise be inherited from the app that
/// spawned us and both of which would be actively wrong:
///
/// * `VN_DIST` — the slot this process publishes its OWN handshake into. It
///   must not be the target's, or the inspector would overwrite the very
///   `devtools.json` it is trying to read. [`open_for_window`] always passes
///   one; this is the fallback for `vn-app --devtools` typed by hand.
/// * `VN_DEVTOOLS_TARGET` — what `--target` means to `__vn_devtools_targets()`.
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

/// Evaluates the embedded inspector. The `--devtools` half of `boot_mode::boot`.
///
/// The bundle is `bun prod.ts`'s output, so it carries its own stylesheet
/// (`__VN_CSS__` pushes ahead of the IIFE) and mounts itself — exactly what
/// [`crate::prod::boot`] evaluates, from a different `include_str!`.
pub fn boot(rt: &JsRuntime) -> Result<(), String> {
    let mode = mode();
    // What the UI needs and cannot derive: which window of the target this
    // inspector was opened for, and whether it is a child at all (a child adds
    // a "close with the app" affordance a hand-launched one must not).
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

/// The inspector's own window block — the same JSON `apps/devtools/vn.config.json`
/// declares, baked in beside the bundle.
pub fn window_config() -> Option<String> {
    Some(INSPECTOR_WINDOW_JSON.to_string())
}

/// `%LOCALAPPDATA%\Temp\vue-native\devtools\<key>` — the slot a child publishes
/// its handshake into. Deterministic, so the parent can read it back to raise
/// the window instead of opening a second one.
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

// ---------------------------------------------------------------------------
// the child registry (the APP side)
// ---------------------------------------------------------------------------

struct Child {
    /// Which of OUR windows this inspector was opened for.
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
/// …and separately, that the KEY is off. Two flags rather than one, because
/// they are two different reasons and a process can hit both — a harness that
/// sets `VN_DEVTOOLS_KEY=off` on an app that later calls `disable()` would
/// otherwise be told only about whichever it reached first.
static KEY_OFF_SAID: AtomicBool = AtomicBool::new(false);

/// How long after an F12 the next one is ignored.
///
/// Two reasons, and the first is the ordinary one: HOLDING F12 auto-repeats at
/// ~30 Hz on Windows, and every repeat is a real `KeyDown` — without this, a
/// leaned-on key would raise the inspector thirty times a second. A browser
/// ignores the repeat too.
///
/// The second is empirical and worth writing down: one synthesized keystroke
/// through `Window::dispatch_keystroke` has been observed reaching the
/// interceptor twice, intermittently, on a host whose app had a focused text
/// input. It is gpui's key path, not ours — but the CORRECT behaviour under a
/// duplicate is the same as under a repeat, so one guard covers both instead
/// of a special case for something that only happens sometimes.
const F12_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

thread_local! {
    static LAST_F12: std::cell::Cell<Option<(Option<u64>, std::time::Instant)>> =
        const { std::cell::Cell::new(None) };
}

/// Installs the F12 key. Called once, from `main`, before any window exists.
///
/// `App::intercept_keystrokes` rather than a key binding or a per-element
/// listener, because the requirement is that an app *cannot accidentally eat
/// it*: interceptors run before action resolution and before every element's
/// own key listener (`Window::dispatch_key_event`), in whichever window has
/// focus. The app still SEES the `keydown` afterwards — propagation is not
/// stopped — which is what a browser does too: a page's `keydown` handler fires
/// for F12 and cannot cancel the devtools.
pub fn install_hotkey(cx: &mut App) {
    // An inspector does not open an inspector on F12. (It can still be
    // inspected — by attaching to it from another one, which is what the
    // targets list is for.)
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

/// F12 with no modifiers. `Shift+F12` and friends are left alone: they are
/// ordinary keystrokes an app is entitled to bind.
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
    // Two gates, and they mean different things — see `crate::window::Config`.
    //
    // The MASTER switch first: the app turned its devtools off (in JS, in
    // `vn.config.json`, or through `VN_DEVTOOLS=0`), so there is no listener
    // for an inspector to attach to and opening one would show an empty window.
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
    // …then the KEY, which a harness turns off on its own with the protocol
    // left alone.
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

/// Reaps children that exited on their own, so a closed inspector window means
/// the next F12 opens a new one instead of trying to raise a corpse.
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
    // The OS's own foreground handshake: WE are the foreground process (the
    // user just pressed a key in our window), so we are allowed to hand the
    // foreground to a named process. Without this the child's own
    // `SetForegroundWindow` is refused and Windows flashes its taskbar button.
    allow_foreground(pid);
    // Off-thread: this runs inside a keystroke interceptor, on the frame pump's
    // own thread, and a TCP round trip there would be a visible hitch in the
    // app being debugged.
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

    // `VN_DEVTOOLS_RAISE=off` — for an automated run on a machine somebody is
    // using. Everything else about the second F12 still happens (no duplicate
    // process, the log line); the window simply does not come forward.
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
    // The inspector is this host exe started again in `--devtools` mode. An
    // embedded vue-native is not in its own exe — this one is the embedder's —
    // so it uses the vn-app.exe the embedder named (src/embed.rs).
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
    // Children that crashed left their slots behind (crate::slot — shared
    // with a packaged app's own `%TEMP%\vue-native\apps\<name>\<pid>`).
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
    // The child is a host in its own right, and every one of these would make
    // it the WRONG host: our dist slot, our window size, our scenario pin, our
    // voice identity. `VN_DIST` is replaced rather than removed — it is what
    // keeps the child's handshake out of the target's slot.
    cmd.env("VN_DIST", &slot);
    cmd.env("VN_DEVTOOLS_TARGET", &target);
    // An inspector has to have a port OF ITS OWN: it is how the parent raises it
    // on a second F12, and how another inspector could attach to it. Forced
    // rather than inherited, because `VN_DEVTOOLS=0` in the developer's shell
    // (or an `appDevtools.disable()` this app later reversed) would otherwise
    // produce a child window nothing can reach or close politely.
    cmd.env("VN_DEVTOOLS", "1");
    for name in ["VN_WINDOW", "VN_SCENARIO", "VN_VOICE_IDENTITY", "VN_APP_ROOT", "VN_PERF"] {
        cmd.env_remove(name);
    }
    cmd.stdin(std::process::Stdio::null());
    // The child's stderr is its own; inheriting it would interleave two hosts'
    // logs in one terminal, and the inspector has a console of its own.
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    let proc = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
    let pid = proc.id();
    CHILDREN.with(|c| c.borrow_mut().push(Child { window, slot, pid, proc }));
    Ok(pid)
}

/// Kills every inspector this process opened. Called from `App::on_app_quit`.
///
/// A debugger that outlives its subject is a window nobody can explain — and,
/// unlike the crash case (which the child's own parent watch covers), a
/// deliberate quit is a moment where we know exactly what to do.
pub fn close_children() {
    CHILDREN.with(|c| {
        for child in c.borrow_mut().iter_mut() {
            let _ = child.proc.kill();
        }
        c.borrow_mut().clear();
    });
}

/// How many inspectors this process has open. For the `windows` command and
/// the e2e specs, which is the only way to prove "no duplicate" from outside.
pub fn children() -> Vec<(Option<u64>, u32)> {
    reap();
    CHILDREN.with(|c| c.borrow().iter().map(|child| (child.window, child.pid)).collect())
}

// ---------------------------------------------------------------------------
// the parent watch (the INSPECTOR side)
// ---------------------------------------------------------------------------

static PARENT_GONE: AtomicBool = AtomicBool::new(false);

/// Starts the watch that closes this inspector when the app it is inspecting
/// exits. Called from `main` in `--devtools` mode.
///
/// It probes the TARGET's own devtools port rather than the pid, for the same
/// reason discovery does (docs/DEVTOOLS-UI.md §3): a pid can be recycled, a
/// refused connect on loopback cannot lie, and it is instant. Three consecutive
/// refusals, because a host mid-restart is a thing that happens and a debugger
/// that vanishes on a hiccup is worse than one that lingers for six seconds.
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
