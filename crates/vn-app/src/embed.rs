//! vue-native as a LIBRARY: another program — a terminal UI, a game launcher,
//! a service with a tray — runs the whole engine on a thread of its own and
//! talks to the app through async functions and events (docs/EMBEDDING.md).
//!
//! ```text
//!   embedder's threads                   the vue-native thread (gpui + JSC)
//!  ┌──────────────────────────┐        ┌──────────────────────────────────────┐
//!  │ Builder::spawn ──────────┼──────▶ │ run_embedded: the exe's boot without │
//!  │                          │        │   the process (argv, single instance,│
//!  │ Handle::emit ──┐         │        │   protocols, the exit)               │
//!  │ Handle::close ─┤ QUEUE ──┼─wake──▶│ pump source: events, settles → JS    │
//!  │ Reply::resolve ┘         │        │ appHost.call("x") → __vn_host_call ──┼─▶ handler(Call)
//!  │ Handle::wait ◀───────────┼─EXIT───│ shutdown::finish: report, then park  │
//!  └──────────────────────────┘        └──────────────────────────────────────┘
//! ```
//!
//! **One per process, once.** The JS realm cannot be torn down (vn-runtime's
//! invariants) and gpui is one `Application`, so [`Builder::spawn`] works once;
//! after the app closes its thread sleeps for the rest of the process.
//!
//! **Handlers run on the vue-native thread**, inside the JS call, so a handler
//! that blocks freezes the UI. One that has work to do moves its [`Reply`] to a
//! thread of its own and answers from there; a dropped `Reply` rejects.
//!
//! What stays the exe's: the host flags on the command line (an embedder's argv
//! is its own), `singleInstance` and the declared protocols (the embedder is
//! the instance), and ending the process. F12 opens the inspector only when
//! [`Builder::inspector_exe`] names a vn-app.exe to start it from.
//!
//! JS: `appHost` (packages/vue-native/src/apphost.ts).

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use vn_runtime::redjs::api::Value;
use vn_runtime::{JsRuntime, host_fn};

mod host_context;
pub use host_context::{HostContext, HostCookie, SameSite};

type Setup = Box<dyn FnOnce(&HostContext) -> Result<(), String> + Send + 'static>;
type PrepareDataDir = Box<dyn FnOnce(&Path) -> Result<(), String> + Send + 'static>;

#[derive(Default)]
struct SetupHook(Mutex<Option<Setup>>);

impl SetupHook {
    fn run(&self, context: &HostContext) -> Result<(), String> {
        let setup = self.0.lock().map_err(|_| "host setup lock was poisoned".to_string())?.take();
        let Some(setup) = setup else { return Ok(()) };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| setup(context)))
            .unwrap_or_else(|_| Err("host setup panicked".into()))
    }
}

/// The vue-native thread's stack: the exe's own (`/STACK` in build.rs), which
/// is what JSC's recursion limit is sized for.
const STACK: usize = 8 * 1024 * 1024;

/// How many queued items one drain may deliver, as in vn-desktop.
const DRAIN_CAP: usize = 16;

/// A host function: called with the [`Call`] on the vue-native thread.
pub type Handler = Arc<dyn Fn(Call) + Send + Sync + 'static>;

struct Config {
    data_dir: Option<PathBuf>,
    #[cfg(feature = "storages")]
    storage_encrypted: Option<bool>,
    visible: bool,
    desktop_launch: bool,
    setup: SetupHook,
    functions: HashMap<String, Handler>,
    activate: bool,
    #[cfg_attr(not(vn_devtools), allow(dead_code))]
    inspector_exe: Option<PathBuf>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

/// Is this vue-native embedded in another program?
pub(crate) fn active() -> bool {
    CONFIG.get().is_some()
}

/// This embedding binary already prepared the desktop launch policy in spawn.
pub(crate) fn desktop_launch_prepared() -> bool {
    CONFIG.get().is_some_and(|config| config.desktop_launch)
}

/// Does the app take the foreground at startup? Always, unless embedded with
/// [`Builder::activate`]`(false)`.
pub(crate) fn activates() -> bool {
    CONFIG.get().is_none_or(|c| requests_activation(c.activate, c.visible))
}

pub(crate) fn visible() -> bool {
    CONFIG.get().is_none_or(|c| c.visible)
}

/// Seals a dormant profile's localStorage before application-owned migration.
/// It uses the SDK storage format and OS provider, with no plaintext fallback.
pub fn encrypt_local_storage_at(directory: &Path) -> Result<bool, String> {
    #[cfg(feature = "storages")]
    { vn_storage::encrypt_local_storage_at(directory) }
    #[cfg(not(feature = "storages"))]
    { let _ = directory; Err("enable the storages feature to encrypt localStorage".into()) }
}

#[cfg(feature = "storages")]
pub(crate) fn storage_encrypted() -> Option<bool> {
    CONFIG.get()?.storage_encrypted
}

/// The persistent root chosen before storage and cookies are installed.
pub(crate) fn data_dir() -> Option<&'static Path> {
    CONFIG.get()?.data_dir.as_deref()
}

fn resolve_data_dir(configured: Option<PathBuf>, override_dir: Option<std::ffi::OsString>) -> Result<Option<PathBuf>, Error> {
    let directory = override_dir.filter(|path| !path.is_empty()).map(PathBuf::from).or(configured);
    if let Some(path) = &directory {
        if !path.is_absolute() {
            return Err(Error::DataDirectory(format!("data directory must be absolute: {}", path.display())));
        }
    }
    Ok(directory)
}

fn prepare_directory(directory: Option<&Path>, prepare: Option<PrepareDataDir>) -> Result<(), Error> {
    let Some(directory) = directory else {
        return if prepare.is_some() { Err(Error::DataDirectory("prepare_data_dir requires a data directory".into())) } else { Ok(()) };
    };
    if let Some(prepare) = prepare {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(directory)))
            .unwrap_or_else(|_| Err("data directory preparation panicked".into()))
            .map_err(Error::DataDirectory)?;
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| Error::DataDirectory(format!("could not create {}: {error}", directory.display())))
}

fn requests_activation(activate: bool, visible: bool) -> bool {
    activate && visible
}

/// The exe F12 starts as the inspector, embedded (src/inspector.rs).
#[cfg_attr(not(vn_devtools), allow(dead_code))]
pub(crate) fn inspector_exe() -> Option<PathBuf> {
    CONFIG.get()?.inspector_exe.clone()
}

// ---------------------------------------------------------------------------
// the builder
// ---------------------------------------------------------------------------

// A dev build has no payload loader; it only refuses one.
#[cfg_attr(not(feature = "prod"), allow(dead_code))]
enum PayloadSource {
    File(PathBuf),
    Bytes(Vec<u8>),
    Static(&'static [u8]),
}

#[cfg(any(feature = "prod", test))]
impl PayloadSource {
    fn load(self) -> Result<(crate::payload::PayloadBytes, String), Error> {
        use crate::payload::PayloadBytes;
        match self {
            Self::File(path) => {
                let file = crate::mapped_file::MappedFile::open(&path)
                    .map_err(|e| Error::Payload(format!("{}: {e}", path.display())))?;
                Ok((PayloadBytes::File(file), path.display().to_string()))
            }
            Self::Bytes(bytes) => Ok((PayloadBytes::Owned(bytes), "bytes".into())),
            Self::Static(bytes) => Ok((PayloadBytes::Embedded(bytes), "static bytes".into())),
        }
    }
}

/// Why [`Builder::spawn`] did not start vue-native.
#[derive(Debug)]
pub enum Error {
    /// vue-native already ran in this process (it runs once; module docs).
    AlreadyStarted,
    /// A `--features prod` build with no app: no payload was given, none is
    /// stamped into this exe, and `VN_PAYLOAD` is unset.
    NoPayload,
    /// A dev build boots the dev dist (`VN_DIST`, packages/vue-native/dist)
    /// and has no payload loader; a payload needs `--features prod`.
    PayloadNeedsProd,
    /// The payload could not be read or parsed.
    Payload(String),
    /// Something read this process's arguments before `spawn`.
    ArgsAlreadyRead,
    /// Desktop launch policy forwarded this invocation, or refused a duplicate.
    /// The embedding main should return this exit code (0 means forwarded).
    LaunchExit(i32),
    /// The configured profile directory could not be prepared.
    DataDirectory(String),
    /// The thread could not be started.
    Spawn(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyStarted => write!(f, "vue-native already ran in this process; it runs once"),
            Error::NoPayload => write!(f, "no app: give the builder a payload (vn build writes dist/.vn/app.vnpk)"),
            Error::PayloadNeedsProd => {
                write!(f, "a payload needs vn-app's `prod` feature; a dev build boots the dev dist instead")
            }
            Error::Payload(e) => write!(f, "{e}"),
            Error::ArgsAlreadyRead => write!(f, "vue-native's arguments were read before spawn"),
            Error::LaunchExit(code) => write!(f, "desktop launch finished with exit code {code}"),
            Error::DataDirectory(e) => write!(f, "data directory: {e}"),
            Error::Spawn(e) => write!(f, "the vue-native thread did not start: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Configures and starts an embedded vue-native.
///
/// ```no_run
/// let ui = vn_app::embed::Builder::new()
///     .payload_file("apps/hello/dist/.vn/app.vnpk")
///     .function("db.query", |call| {
///         let reply = call.into_reply();
///         std::thread::spawn(move || reply.resolve(r#"{"rows":[]}"#));
///     })
///     .spawn()?;
/// ui.emit("status", r#"{"online":true}"#)?;
/// let exit = ui.wait();
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Builder {
    data_dir: Option<PathBuf>,
    storage_encrypted: Option<bool>,
    prepare_data_dir: Option<PrepareDataDir>,
    visible: bool,
    desktop_launch: bool,
    setup: Option<Setup>,
    payload: Option<PayloadSource>,
    args: Vec<String>,
    functions: HashMap<String, Handler>,
    activate: bool,
    inspector_exe: Option<PathBuf>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        Builder { data_dir: None, storage_encrypted: None, prepare_data_dir: None, visible: true, desktop_launch: false, setup: None, payload: None, args: Vec::new(), functions: HashMap::new(), activate: true, inspector_exe: None }
    }

    /// Sets the persistent profile root before cookies, localStorage,
    /// IndexedDB, permission decisions and host secrets are loaded. HTTP and
    /// bytecode caches live in its `cache/http` and `cache/jsc` subdirectories.
    /// `VN_DATA_DIR` overrides this path for isolated dev/test processes.
    /// Paths must be absolute. Without either option, existing defaults apply.
    pub fn data_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(directory.into());
        self
    }

    /// Sets storage encryption before any cookie, database or app code runs.
    /// `true` requires an OS encryption provider and seals existing plaintext
    /// localStorage before boot. Without this option the sticky/default policy applies.
    pub fn storage_encrypted(mut self, encrypted: bool) -> Self {
        self.storage_encrypted = Some(encrypted);
        self
    }

    /// Prepares/migrates the configured profile after desktop single-instance
    /// handling, before directory creation or any storage/VM initialization.
    /// An error or panic stops startup; no default profile is substituted.
    pub fn prepare_data_dir<F>(mut self, prepare: F) -> Self
    where F: FnOnce(&Path) -> Result<(), String> + Send + 'static,
    {
        self.prepare_data_dir = Some(Box::new(prepare));
        self
    }

    /// Lets this embedding binary own the payload's single-instance and custom
    /// protocol policy. Uses the real process command line, including a deep
    /// link's verbatim raw URL; `args` applies only to the default embedding mode.
    /// A forwarded launch returns `Error::LaunchExit(0)` before creating a VM or window.
    pub fn desktop_launch(mut self, enabled: bool) -> Self {
        self.desktop_launch = enabled;
        self
    }

    /// Runs once on the vue-native thread, after storage installation and before
    /// any application JavaScript or window. An error or panic prevents startup;
    /// the returned Handle reports an unclean Exit instead of opening the app.
    /// Keep this short; prepare expensive plugin work before `spawn`.
    pub fn setup<F>(mut self, setup: F) -> Self
    where
        F: FnOnce(&HostContext) -> Result<(), String> + Send + 'static,
    {
        self.setup = Some(Box::new(setup));
        self
    }

    /// The app, as the `.vnpk` file `vn build` writes (`dist/.vn/app.vnpk`).
    /// A `--features prod` build only. Read-only mapped for the app lifetime;
    /// publish updates by atomic replacement, never by modifying it in place.
    pub fn payload_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.payload = Some(PayloadSource::File(path.into()));
        self
    }

    /// The app, as owned bytes of a `.vnpk`.
    /// For `include_bytes!`, use [`Self::payload_static`] to avoid a heap copy.
    /// The provided Vec is moved into the payload owner without another clone.
    pub fn payload_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.payload = Some(PayloadSource::Bytes(bytes.into()));
        self
    }

    /// A static `.vnpk`, such as `include_bytes!`, borrowed without copying.
    /// The executable's read-only mapping owns these bytes for the process lifetime.
    pub fn payload_static(mut self, bytes: &'static [u8]) -> Self {
        self.payload = Some(PayloadSource::Static(bytes));
        self
    }

    /// `appLaunch.argv` after the exe: what the app is launched with. None of
    /// it is read as a host flag.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// A function the app calls as `appHost.call(name, ...args)`: the handler
    /// gets the arguments as a JSON array and answers through the [`Call`].
    pub fn function<F>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Fn(Call) + Send + Sync + 'static,
    {
        self.functions.insert(name.into(), Arc::new(handler));
        self
    }

    /// Whether the primary window is shown at startup (default true). Hidden
    /// applications still boot JS and pump background work; the embedding app
    /// owns the tray or other UI that shows the window later. Hidden starts do
    /// not activate the application or raise an already running desktop instance.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Whether the app's window takes the foreground when it opens (default
    /// `true`, as the exe does). `false` leaves the embedder's console or
    /// window where it is.
    pub fn activate(mut self, on: bool) -> Self {
        self.activate = on;
        self
    }

    /// A vn-app.exe of this vue-native version, started as the inspector on
    /// F12 and `appDevtools.open()`. Without it they say why nothing opened;
    /// the devtools protocol itself (ctl.ts, the MCP server) works either way.
    pub fn inspector_exe(mut self, path: impl Into<PathBuf>) -> Self {
        self.inspector_exe = Some(path.into());
        self
    }

    /// Starts vue-native on a thread of its own and returns at once; the
    /// window opens a moment later. Events emitted before the app is ready
    /// wait for it.
    pub fn spawn(mut self) -> Result<Handle, Error> {
        let data_dir = resolve_data_dir(self.data_dir.take(), std::env::var_os("VN_DATA_DIR"))?;
        #[cfg(not(feature = "storages"))]
        if self.storage_encrypted.is_some() {
            return Err(Error::DataDirectory("storage_encrypted requires the storages feature".into()));
        }
        if STARTED.swap(true, Ordering::SeqCst) {
            return Err(Error::AlreadyStarted);
        }
        if let Err(e) = self.load_payload() {
            STARTED.store(false, Ordering::SeqCst);
            return Err(e);
        }
        if self.desktop_launch {
            // Preserve GetCommandLineW protocol recovery; rejoining Builder.args
            // would corrupt quotes and repeated spaces inside a deep link.
            let args = vn_desktop::args::process();
            let launch = crate::boot_mode::launch_config();
            if let vn_desktop::launch::Outcome::Exit(code) =
                vn_desktop::launch::start_with_activation(args, launch.as_deref(), crate::boot_mode::app_name(), None, requests_activation(self.activate, self.visible))
            {
                return Err(Error::LaunchExit(code));
            }
            vn_desktop::launch::register_declared();
        } else {
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            if !vn_desktop::args::set_embedded(exe, self.args) {
                return Err(Error::ArgsAlreadyRead);
            }
        }
        prepare_directory(data_dir.as_deref(), self.prepare_data_dir.take())?;
        let config = Config {
            data_dir,
            #[cfg(feature = "storages")]
            storage_encrypted: self.storage_encrypted,
            visible: self.visible, desktop_launch: self.desktop_launch,
            setup: SetupHook(Mutex::new(self.setup)), functions: self.functions,
            activate: self.activate, inspector_exe: self.inspector_exe,
        };
        if CONFIG.set(config).is_err() {
            return Err(Error::AlreadyStarted);
        }
        std::thread::Builder::new()
            .name("vue-native".into())
            .stack_size(STACK)
            .spawn(run_thread)
            .map_err(Error::Spawn)?;
        Ok(Handle { _private: () })
    }

    #[cfg(feature = "prod")]
    fn load_payload(&mut self) -> Result<(), Error> {
        let Some(source) = self.payload.take() else {
            // The embedder's own exe may carry one (`vn build` stamps any PE),
            // or `VN_PAYLOAD` may name one: the exe's own rules.
            return if crate::payload::get().is_some() { Ok(()) } else { Err(Error::NoPayload) };
        };
        let (bytes, label) = source.load()?;
        crate::payload::set_external(bytes, label).map_err(Error::Payload)
    }

    #[cfg(not(feature = "prod"))]
    fn load_payload(&self) -> Result<(), Error> {
        match self.payload {
            Some(_) => Err(Error::PayloadNeedsProd),
            None => Ok(()),
        }
    }
}

/// The vue-native thread.
fn run_thread() {
    // `run_embedded` ends in `shutdown::finish`, which never returns; only a
    // panic out of gpui's loop gets past it. The thread must not end even then
    // (its thread-locals are the JS realm), so it reports and sleeps.
    if let Err(p) = std::panic::catch_unwind(crate::run_embedded) {
        let why = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".into());
        eprintln!("[embed] the vue-native thread panicked: {why}");
        finished(false, format!("panicked: {why}"));
    }
    loop {
        std::thread::park();
    }
}

// ---------------------------------------------------------------------------
// the handle
// ---------------------------------------------------------------------------

/// How the app ended.
#[derive(Debug, Clone)]
pub struct Exit {
    /// `false` when the teardown ran past its deadline (docs/EMBEDDING.md) or
    /// the vue-native thread panicked.
    pub clean: bool,
    /// What closed it and how long each teardown step took — the exe's
    /// `[shutdown]` line.
    pub summary: String,
}

static EXIT: Mutex<Option<Exit>> = Mutex::new(None);
static EXIT_CV: Condvar = Condvar::new();
static CLOSE: AtomicBool = AtomicBool::new(false);
static SHOW: AtomicBool = AtomicBool::new(false);

/// The app has closed (src/shutdown.rs). The first report wins.
pub(crate) fn finished(clean: bool, summary: String) {
    let Ok(mut exit) = EXIT.lock() else { return };
    if exit.is_none() {
        *exit = Some(Exit { clean, summary });
        EXIT_CV.notify_all();
    }
}

/// Why [`Handle::emit`] did not queue an event.
#[derive(Debug, PartialEq, Eq)]
pub enum EmitError {
    /// `data` is not one JSON value.
    NotJson,
    /// The app has closed.
    Closed,
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::NotJson => write!(f, "the event's data is not JSON"),
            EmitError::Closed => write!(f, "the app has closed"),
        }
    }
}

impl std::error::Error for EmitError {}

/// The embedder's side of a running vue-native. Cheap to clone; every method
/// works from any thread.
#[derive(Clone)]
pub struct Handle {
    _private: (),
}

impl Handle {
    /// Delivers `appHost.on(event)`'s handlers `data` (one JSON value) in the
    /// app's next frame.
    pub fn emit(&self, event: &str, data: &str) -> Result<(), EmitError> {
        if self.is_closed() {
            return Err(EmitError::Closed);
        }
        if crate::json::parse(data).is_none() {
            return Err(EmitError::NotJson);
        }
        let json = format!("{{\"event\":{},\"data\":{data}}}", crate::json::quote(event));
        push(Item::Event { json });
        Ok(())
    }

    /// Queues a show/activation on the UI thread, including the first show of
    /// an initially hidden window. Returns whether it was accepted; closed or
    /// closing applications refuse it. Uses the toolkit's placement/restore path.
    pub fn show(&self) -> bool {
        if self.is_closed() || crate::shutdown::is_closing() || CLOSE.load(Ordering::Acquire) {
            return false;
        }
        SHOW.store(true, Ordering::Release);
        wake();
        true
    }

    /// Closes the app as `appWindow.close()` does: the windows go at once and
    /// the teardown runs behind them; [`Handle::wait`] returns when it is done.
    pub fn close(&self) {
        CLOSE.store(true, Ordering::Release);
        wake();
    }

    /// Has the app closed?
    pub fn is_closed(&self) -> bool {
        EXIT.lock().is_ok_and(|e| e.is_some())
    }

    /// Blocks until the app has closed.
    pub fn wait(&self) -> Exit {
        let mut exit = EXIT.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(e) = exit.as_ref() {
                return e.clone();
            }
            exit = EXIT_CV.wait(exit).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// [`Handle::wait`], for at most `timeout`.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Exit> {
        let exit = EXIT.lock().unwrap_or_else(|p| p.into_inner());
        let (exit, _) = EXIT_CV
            .wait_timeout_while(exit, timeout, |e| e.is_none())
            .unwrap_or_else(|p| p.into_inner());
        exit.clone()
    }
}

// ---------------------------------------------------------------------------
// calls
// ---------------------------------------------------------------------------

/// One `appHost.call(name, ...args)`.
pub struct Call {
    name: String,
    args: String,
    reply: Reply,
}

impl Call {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The call's arguments, as one JSON array.
    pub fn args(&self) -> &str {
        &self.args
    }

    /// Resolves the call's promise with `json` (one JSON value).
    pub fn resolve(self, json: impl Into<String>) {
        self.reply.resolve(json)
    }

    /// Rejects the call's promise with an `AppHostError` saying `message`.
    pub fn reject(self, message: impl Into<String>) {
        self.reply.reject(message)
    }

    /// The answer half, to move to another thread.
    pub fn into_reply(self) -> Reply {
        self.reply
    }
}

/// Settles one call's promise, from any thread. Dropped unanswered, it
/// rejects, so a promise never hangs on a host that forgot it.
pub struct Reply {
    token: u32,
    settled: bool,
}

impl Reply {
    /// Resolves with `json` (one JSON value); text that is not JSON rejects.
    pub fn resolve(mut self, json: impl Into<String>) {
        self.settled = true;
        let json = json.into();
        if crate::json::parse(&json).is_some() {
            push(Item::Settle { token: self.token, ok: true, text: json });
        } else {
            push(Item::Settle { token: self.token, ok: false, text: "the host answered with text that is not JSON".into() });
        }
    }

    pub fn reject(mut self, message: impl Into<String>) {
        self.settled = true;
        push(Item::Settle { token: self.token, ok: false, text: message.into() });
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let why = if std::thread::panicking() {
            "the host function panicked"
        } else {
            "the host dropped this call without answering"
        };
        push(Item::Settle { token: self.token, ok: false, text: why.into() });
    }
}

// ---------------------------------------------------------------------------
// the queue into JS — vn-desktop's settle channel, in miniature
// ---------------------------------------------------------------------------

enum Item {
    Settle { token: u32, ok: bool, text: String },
    Event { json: String },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static QUEUED: AtomicUsize = AtomicUsize::new(0);
/// Calls made and not yet delivered back: the event loop's in-flight work.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);
static RT_G: AtomicUsize = AtomicUsize::new(0);
static RT_ID: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static DISPATCHER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn push(item: Item) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(item);
        QUEUED.fetch_add(1, Ordering::AcqRel);
    }
    wake();
}

fn wake() {
    let id = RT_ID.load(Ordering::Acquire);
    if id != 0 {
        vn_runtime::redjs::web::wake(vn_runtime::redjs::context::ContextId(id));
    }
}

fn is_ours(g: *mut c_void) -> bool {
    !g.is_null() && g as usize == RT_G.load(Ordering::Acquire)
}

unsafe fn src_inflight(g: *mut c_void) -> bool {
    is_ours(g) && IN_FLIGHT.load(Ordering::Acquire) != 0
}

unsafe fn src_ready(g: *mut c_void) -> bool {
    is_ours(g) && QUEUED.load(Ordering::Acquire) != 0 && DISPATCHER.with(|d| d.get()) != 0
}

unsafe fn src_drain(g: *mut c_void) -> usize {
    if !unsafe { src_ready(g) } {
        return 0;
    }
    let budget = Duration::from_micros(vn_runtime::redjs::web::pump_budget_us() as u64);
    let start = std::time::Instant::now();
    let mut n = 0;
    while n < DRAIN_CAP {
        let Some(item) = QUEUE.lock().ok().and_then(|mut q| q.pop_front()) else { break };
        QUEUED.fetch_sub(1, Ordering::AcqRel);
        match item {
            Item::Settle { token, ok, text } => {
                IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
                deliver(token, ok, &text);
            }
            Item::Event { json } => deliver(0, true, &json),
        }
        n += 1;
        if start.elapsed() >= budget {
            break;
        }
    }
    n
}

fn deliver(token: u32, ok: bool, text: &str) {
    let dispatcher = DISPATCHER.with(|d| d.get());
    if dispatcher == 0 {
        return;
    }
    JsRuntime::with(|rt| {
        let a = [Value::number(f64::from(token)), Value::boolean(ok), rt.ctx().string(text)];
        if let Err(vn_runtime::redjs::api::Error::Exception(e)) = rt.ctx().call(Value(dispatcher), Value::undefined(), &a) {
            let err = rt.describe(e);
            eprintln!("[appHost dispatch] {}\n{}", err.header, err.stack);
        }
    });
}

/// After storage installation, before any application JS or window creation.
pub(crate) fn setup(runtime: &JsRuntime) -> Result<(), String> {
    let Some(config) = CONFIG.get() else { return Ok(()) };
    config.setup.run(&HostContext::installed(Some(runtime)))
}

/// In the frame's JS wave (lib.rs `js_wave`): a close the embedder asked for.
pub(crate) fn pump() {
    let show = SHOW.swap(false, Ordering::AcqRel);
    if CLOSE.swap(false, Ordering::AcqRel) {
        vn_tree::log::record("embed", "[embed] the embedder closed the app");
        crate::window::request_close();
    } else if show && !crate::shutdown::is_closing() {
        crate::window::request_activate();
    }
}

// ---------------------------------------------------------------------------
// host functions — `appHost` (packages/vue-native/src/apphost.ts)
// ---------------------------------------------------------------------------

host_fn!(fn vn_host_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    let old = DISPATCHER.with(|d| d.replace(f.0));
    if old != 0 {
        unsafe { vn_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    // Whatever was emitted before the app was ready is deliverable now.
    wake();
    Value::undefined().0
});

// `__vn_host_call(name, argsJson) -> token`, 0 for a name the embedder did not
// register (the shim rejects with the list it did).
host_fn!(fn vn_host_call(g, args) {
    let name = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }.unwrap_or_default();
    let call_args = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(1).0) }
        .filter(|a| crate::json::parse(a).is_some())
        .unwrap_or_else(|| "[]".into());
    let Some(handler) = CONFIG.get().and_then(|c| c.functions.get(&name)).cloned() else {
        return Value::number(0.0).0;
    };
    let token = loop {
        let t = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        if t != 0 {
            break t;
        }
    };
    IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    let call = Call { name, args: call_args, reply: Reply { token, settled: false } };
    // The embedder's code, not ours: a panic in it rejects this call (the
    // Reply's drop says so) instead of poisoning the runtime the way a panic
    // in a host fn of our own does.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(call))).is_err() {
        eprintln!("[embed] the host function behind appHost.call panicked");
    }
    Value::number(f64::from(token)).0
});

host_fn!(fn vn_host_functions(_g, _args) {
    let mut names: Vec<&String> = CONFIG.get().map(|c| c.functions.keys().collect()).unwrap_or_default();
    names.sort();
    let json = crate::json::Json::arr(names.into_iter().map(|n| crate::json::Json::s(n.as_str())).collect()).to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

/// Registers `__vn_host_*`, embedded only: in the exe they do not exist and
/// `appHost.available` is `false`.
pub(crate) fn install(rt: &JsRuntime) {
    if !active() {
        return;
    }
    rt.register("__vn_host_set_dispatcher", 1, vn_host_set_dispatcher as *const _);
    rt.register("__vn_host_call", 2, vn_host_call as *const _);
    rt.register("__vn_host_functions", 0, vn_host_functions as *const _);
    let g = rt.raw();
    RT_G.store(g as usize, Ordering::Release);
    if let Some(id) = vn_runtime::redjs::context::id_of(g) {
        RT_ID.store(id.0, Ordering::Release);
        let _ = vn_runtime::redjs::web::waker(id);
    }
    vn_runtime::redjs::web::register_pump_source(vn_runtime::redjs::web::PumpSource {
        inflight: src_inflight,
        ready: src_ready,
        drain: src_drain,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_queue() -> Vec<(u32, bool, String)> {
        let mut q = QUEUE.lock().unwrap();
        let out = q
            .drain(..)
            .map(|i| match i {
                Item::Settle { token, ok, text } => (token, ok, text),
                Item::Event { json } => (0, true, json),
            })
            .collect();
        QUEUED.store(0, Ordering::Release);
        out
    }

    #[test]
    fn payload_source_moves_builder_bytes_and_maps_files_without_cloning() {
        let bytes = vec![1, 2, 3, 4];
        let pointer = bytes.as_ptr();
        let mut builder = Builder::new().payload_bytes(bytes);
        let (owned, label) = builder.payload.take().unwrap().load().unwrap();
        let crate::payload::PayloadBytes::Owned(owned) = owned else { panic!("owned bytes expected") };
        assert_eq!(label, "bytes");
        assert_eq!(owned.as_ptr(), pointer);
        assert!(builder.payload.is_none());
        static STATIC_PAYLOAD: &[u8] = b"static payload bytes";
        let mut builder = Builder::new().payload_static(STATIC_PAYLOAD);
        let (borrowed, label) = builder.payload.take().unwrap().load().unwrap();
        let crate::payload::PayloadBytes::Embedded(borrowed) = borrowed else { panic!("static bytes expected") };
        assert_eq!(borrowed.as_ptr(), STATIC_PAYLOAD.as_ptr());
        assert_eq!(borrowed.len(), STATIC_PAYLOAD.len());
        assert_eq!(label, "static bytes");
        let mut files = crate::mapped_file::tests::Files::new();
        let path = files.write("app.vnpk", b"file bytes");
        let mut builder = Builder::new().payload_file(&path);
        let (mapped, label) = builder.payload.take().unwrap().load().unwrap();
        let crate::payload::PayloadBytes::File(mapped) = mapped else { panic!("mapped file expected") };
        assert_eq!(mapped.as_bytes(), b"file bytes");
        assert_eq!(label, path.display().to_string());
        assert!(builder.payload.is_none());
        #[cfg(target_os = "windows")]
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(mapped);
        std::fs::write(&path, b"released").unwrap();
    }

    #[test]
    fn data_directory_override_and_preparation_precede_storage() {
        let base = std::env::temp_dir().join(format!("vn-data-dir-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let configured = base.join("configured");
        let overridden = base.join("override");
        assert_eq!(resolve_data_dir(Some(configured.clone()), Some(overridden.clone().into_os_string())).unwrap(), Some(overridden));
        assert_eq!(resolve_data_dir(Some(configured.clone()), Some("".into())).unwrap(), Some(configured.clone()));
        assert_eq!(resolve_data_dir(None, None).unwrap(), None);
        assert!(resolve_data_dir(Some(PathBuf::from("relative")), None).is_err());
        assert!(prepare_directory(None, Some(Box::new(|_| Ok(())))).is_err());
        let refused = base.join("refused");
        assert!(prepare_directory(Some(&refused), Some(Box::new(|_| Err("migration refused".into())))).is_err());
        assert!(!refused.exists());
        prepare_directory(Some(&configured), Some(Box::new(|path| {
            assert!(!path.exists());
            std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
            std::fs::write(path.join("prepared"), "profile").map_err(|e| e.to_string())
        }))).unwrap();
        assert_eq!(std::fs::read_to_string(configured.join("prepared")).unwrap(), "profile");
        std::fs::remove_file(configured.join("prepared")).unwrap();
        std::fs::remove_dir(configured).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn startup_visibility_controls_initial_focus_and_secondary_activation() {
        let normal = Builder::new();
        assert!(normal.visible && requests_activation(normal.activate, normal.visible));
        let hidden = Builder::new().visible(false);
        assert!(!hidden.visible && !requests_activation(hidden.activate, hidden.visible));
        let inactive = Builder::new().activate(false);
        assert!(inactive.visible && !requests_activation(inactive.activate, inactive.visible));
    }

    #[test]
    fn setup_receives_installed_context_once_and_propagates_failure() {
        let called = Arc::new(AtomicUsize::new(0));
        let observed = called.clone();
        let hook = SetupHook(Mutex::new(Some(Box::new(move |ctx| {
            assert_eq!(ctx.storage_dir().ok(), HostContext::installed(None).storage_dir().ok());
            observed.fetch_add(1, Ordering::SeqCst);
            Err("bootstrap refused".into())
        }))));
        assert_eq!(hook.run(&HostContext::installed(None)), Err("bootstrap refused".into()));
        assert!(hook.run(&HostContext::installed(None)).is_ok());
        assert_eq!(called.load(Ordering::SeqCst), 1);
        let panic = SetupHook(Mutex::new(Some(Box::new(|_| panic!("plugin panic")))));
        assert_eq!(panic.run(&HostContext::installed(None)), Err("host setup panicked".into()));
    }

    // One test, because the queue is the process's.
    #[test]
    fn replies_settle_once_and_a_dropped_one_rejects() {
        drain_queue();
        Reply { token: 7, settled: false }.resolve(r#"{"a":1}"#);
        Reply { token: 8, settled: false }.resolve("not json");
        Reply { token: 9, settled: false }.reject("nope");
        drop(Reply { token: 10, settled: false });
        let got = drain_queue();
        assert_eq!(got[0], (7, true, r#"{"a":1}"#.to_string()));
        assert_eq!(got[1].0, 8);
        assert!(!got[1].1 && got[1].2.contains("not JSON"));
        assert_eq!(got[2], (9, false, "nope".to_string()));
        assert_eq!(got[3].0, 10);
        assert!(!got[3].1 && got[3].2.contains("without answering"));

        let h = Handle { _private: () };
        assert_eq!(h.emit("x", "{bad"), Err(EmitError::NotJson));
        h.emit("status", r#"{"online":true}"#).unwrap();
        assert_eq!(drain_queue()[0].2, r#"{"event":"status","data":{"online":true}}"#);
    }
}
