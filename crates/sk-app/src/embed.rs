//! Skathi as a library: another program runs the whole engine on a thread of its own and gives
//! the app native modules (`import { … } from "native:<name>"`): typed functions, tasks and
//! events. On macOS, AppKit owns the process main thread: use `Builder::run_on_main_thread` with
//! an application cleanup callback instead.
//!
//! **One per process, once.** The JS realm cannot be torn down and gpui is one `Application`, so
//! [`Builder::spawn`] works once; after the app closes its thread sleeps for the rest of the process.
//!
//! **A module function runs on the Skathi thread**, inside the JS call, so one that blocks
//! freezes the UI: slow work is a [`Module::task`], which runs on a worker and returns a promise.
//!
//! The exe keeps the host command-line flags, `singleInstance`, protocol handling and ending the
//! process. F12 opens the inspector only when [`Builder::inspector_exe`] names a sk-app.exe.

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use sk_runtime::redjs::api::Value;
use sk_runtime::{host_fn, JsRuntime};

mod host_context;
mod module;
#[cfg(feature = "types")]
mod types;
pub use host_context::{HostContext, HostCookie, SameSite};
pub use module::{Declared, Function, IntoModule, Job, Module, Typed};
pub use serde;
pub use sk_tree::surface::{
    surface, D3D11Device, D3D12Device, D3D12Fence, SharedTexture, SurfaceError, SurfaceEvent,
    SurfaceHandle, SurfaceInfo, SurfaceStats, SurfaceWatch,
};
#[cfg(feature = "types")]
pub use specta::{self, Type};

type Setup = Box<dyn FnOnce(&HostContext) -> Result<(), String> + Send + 'static>;
type PrepareDataDir = Box<dyn FnOnce(&Path) -> Result<(), String> + Send + 'static>;

#[derive(Default)]
struct SetupHook(Mutex<Option<Setup>>);

impl SetupHook {
    fn run(&self, context: &HostContext) -> Result<(), String> {
        let setup = self
            .0
            .lock()
            .map_err(|_| "host setup lock was poisoned".to_string())?
            .take();
        let Some(setup) = setup else { return Ok(()) };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| setup(context)))
            .unwrap_or_else(|_| Err("host setup panicked".into()))
    }
}

/// The exe's own stack size (`/STACK` in build.rs), which JSC's recursion limit is sized for.
#[cfg(not(target_os = "macos"))]
const STACK: usize = 8 * 1024 * 1024;

const DRAIN_CAP: usize = 16;

pub(crate) const HOST_TYPES_FLAG: &str = "--sk-host-types";

#[cfg(feature = "types")]
fn host_types(modules: &[Module]) -> Result<String, String> {
    types::declarations(modules)
}

#[cfg(not(feature = "types"))]
fn host_types(modules: &[Module]) -> Result<String, String> {
    if modules.is_empty() {
        return Ok(String::new());
    }
    Err("this program was built without skathi's types feature: turn it on to type its native modules".into())
}

/// A host function: called with the [`Call`] on the Skathi thread.
pub type Handler = Arc<dyn Fn(Call) + Send + Sync + 'static>;

struct Config {
    data_dir: Option<PathBuf>,
    #[cfg(feature = "storages")]
    storage_encrypted: Option<bool>,
    visible: bool,
    desktop_launch: bool,
    setup: SetupHook,
    functions: HashMap<String, Handler>,
    modules: Vec<Module>,
    executor: Option<module::Executor>,
    activate: bool,
    #[cfg_attr(not(sk_devtools), allow(dead_code))]
    inspector_exe: Option<PathBuf>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    desktop_id: Option<String>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn active() -> bool {
    CONFIG.get().is_some()
}

/// `spawn` already prepared the desktop launch policy.
pub(crate) fn desktop_launch_prepared() -> bool {
    CONFIG.get().is_some_and(|config| config.desktop_launch)
}

/// Always, unless embedded with [`Builder::activate`]`(false)`.
pub(crate) fn activates() -> bool {
    CONFIG
        .get()
        .is_none_or(|c| requests_activation(c.activate, c.visible))
}

pub(crate) fn visible() -> bool {
    CONFIG.get().is_none_or(|c| c.visible)
}

/// Seals a dormant profile's localStorage with the OS provider, no plaintext fallback.
pub fn encrypt_local_storage_at(directory: &Path) -> Result<bool, String> {
    #[cfg(feature = "storages")]
    {
        sk_storage::encrypt_local_storage_at(directory)
    }
    #[cfg(not(feature = "storages"))]
    {
        let _ = directory;
        Err("enable the storages feature to encrypt localStorage".into())
    }
}

#[cfg(feature = "storages")]
pub(crate) fn storage_encrypted() -> Option<bool> {
    CONFIG.get()?.storage_encrypted
}

pub(crate) fn data_dir() -> Option<&'static Path> {
    CONFIG.get()?.data_dir.as_deref()
}

fn resolve_data_dir(
    configured: Option<PathBuf>,
    override_dir: Option<std::ffi::OsString>,
) -> Result<Option<PathBuf>, Error> {
    let directory = override_dir
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or(configured);
    if let Some(path) = &directory {
        if !path.is_absolute() {
            return Err(Error::DataDirectory(format!(
                "data directory must be absolute: {}",
                path.display()
            )));
        }
    }
    Ok(directory)
}

fn prepare_directory(
    directory: Option<&Path>,
    prepare: Option<PrepareDataDir>,
) -> Result<(), Error> {
    let Some(directory) = directory else {
        return if prepare.is_some() {
            Err(Error::DataDirectory(
                "prepare_data_dir requires a data directory".into(),
            ))
        } else {
            Ok(())
        };
    };
    if let Some(prepare) = prepare {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(directory)))
            .unwrap_or_else(|_| Err("data directory preparation panicked".into()))
            .map_err(Error::DataDirectory)?;
    }
    std::fs::create_dir_all(directory).map_err(|error| {
        Error::DataDirectory(format!("could not create {}: {error}", directory.display()))
    })
}

fn requests_activation(activate: bool, visible: bool) -> bool {
    activate && visible
}

#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub(crate) fn inspector_exe() -> Option<PathBuf> {
    CONFIG.get()?.inspector_exe.clone()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn desktop_id() -> Option<String> {
    CONFIG.get()?.desktop_id.clone()
}

// A dev build has no payload loader; it only refuses one.
#[cfg_attr(not(feature = "prod"), allow(dead_code))]
enum PayloadSource {
    File(PathBuf),
    Bytes(Vec<u8>),
    Static(&'static [u8]),
}

#[cfg(any(feature = "prod", test))]
impl PayloadSource {
    /// The bytes, a label, and the folder of its asset packs (`None`: the executable's).
    fn load(self) -> Result<(crate::payload::PayloadBytes, String, Option<PathBuf>), Error> {
        use crate::payload::PayloadBytes;
        match self {
            Self::File(path) => {
                let file = crate::mapped_file::MappedFile::open(&path)
                    .map_err(|e| Error::Payload(format!("{}: {e}", path.display())))?;
                let dir = path.parent().map(Path::to_path_buf);
                Ok((PayloadBytes::File(file), path.display().to_string(), dir))
            }
            Self::Bytes(bytes) => Ok((PayloadBytes::Owned(bytes), "bytes".into(), None)),
            Self::Static(bytes) => Ok((PayloadBytes::Embedded(bytes), "static bytes".into(), None)),
        }
    }
}

/// Why [`Builder::spawn`] did not start Skathi.
#[derive(Debug)]
pub enum Error {
    /// It runs once per process.
    AlreadyStarted,
    /// A `--features prod` build with no payload given, stamped in, or in `SK_PAYLOAD`.
    NoPayload,
    /// A dev build boots the dev dist and has no payload loader.
    PayloadNeedsProd,
    /// The payload could not be read or parsed.
    Payload(String),
    /// Something read this process's arguments before `spawn`.
    ArgsAlreadyRead,
    /// Desktop launch policy forwarded this invocation or refused a duplicate: the embedding main
    /// should return this exit code (0 means forwarded).
    LaunchExit(i32),
    /// The configured profile directory could not be prepared.
    DataDirectory(String),
    /// A native module is misdeclared: a bad or repeated name.
    Module(String),
    Spawn(std::io::Error),
    /// AppKit needs the process main thread: use `run_on_main_thread`.
    MainThreadRequired,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyStarted => write!(f, "Skathi already ran in this process; it runs once"),
            Error::NoPayload => write!(
                f,
                "no app: give the builder a payload (`sk publish --payload-only` writes dist/release/.sk/app.skpk)"
            ),
            Error::PayloadNeedsProd => {
                write!(f, "a payload needs sk-app's `prod` feature; a dev build boots the dev dist instead")
            }
            Error::Payload(e) => write!(f, "{e}"),
            Error::ArgsAlreadyRead => write!(f, "Skathi's arguments were read before spawn"),
            Error::LaunchExit(code) => write!(f, "desktop launch finished with exit code {code}"),
            Error::DataDirectory(e) => write!(f, "data directory: {e}"),
            Error::Module(e) => write!(f, "{e}"),
            Error::Spawn(e) => write!(f, "the Skathi thread did not start: {e}"),
            Error::MainThreadRequired => write!(f, "AppKit requires the process main thread; call Builder::run_on_main_thread from main"),
        }
    }
}

impl std::error::Error for Error {}

/// Configures and starts an embedded Skathi.
///
/// ```no_run
/// let ui = sk_app::embed::Builder::new()
///     .payload_file("apps/hello/dist/release/.sk/app.skpk")
///     .module("db", |m: &mut sk_app::embed::Module| {
///         m.task("query", |sql: String| -> Result<Vec<String>, String> { Ok(vec![sql]) });
///         m.event::<bool>("online");
///     })
///     .spawn()?;
/// ui.emit_to("db", "online", &true)?;
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
    modules: Vec<Module>,
    executor: Option<module::Executor>,
    activate: bool,
    inspector_exe: Option<PathBuf>,
    desktop_id: Option<String>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            data_dir: None,
            storage_encrypted: None,
            prepare_data_dir: None,
            visible: true,
            desktop_launch: false,
            setup: None,
            payload: None,
            args: Vec::new(),
            functions: HashMap::new(),
            modules: Vec::new(),
            executor: None,
            activate: true,
            inspector_exe: None,
            desktop_id: None,
        }
    }

    /// Sets the persistent profile root, before storage and cookies load. HTTP and bytecode caches
    /// live in its `cache/http` and `cache/jsc`. `SK_DATA_DIR` overrides it. Must be absolute.
    pub fn data_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(directory.into());
        self
    }

    /// Sets storage encryption before any cookie, database or app code runs. `true` requires an OS
    /// provider and seals existing plaintext localStorage before boot.
    pub fn storage_encrypted(mut self, encrypted: bool) -> Self {
        self.storage_encrypted = Some(encrypted);
        self
    }

    /// Prepares or migrates the profile after single-instance handling, before any storage or VM
    /// initialization. An error or panic stops startup.
    pub fn prepare_data_dir<F>(mut self, prepare: F) -> Self
    where
        F: FnOnce(&Path) -> Result<(), String> + Send + 'static,
    {
        self.prepare_data_dir = Some(Box::new(prepare));
        self
    }

    /// Lets this binary own the payload's single-instance and protocol policy, using the real command
    /// line (a deep link's raw URL included). A forwarded launch returns `Error::LaunchExit(0)`.
    pub fn desktop_launch(mut self, enabled: bool) -> Self {
        self.desktop_launch = enabled;
        self
    }

    /// Runs once on the Skathi thread after storage install, before any app JS or window. An
    /// error or panic prevents startup. Keep it short.
    pub fn setup<F>(mut self, setup: F) -> Self
    where
        F: FnOnce(&HostContext) -> Result<(), String> + Send + 'static,
    {
        self.setup = Some(Box::new(setup));
        self
    }

    /// The `.skpk` file `sk build` writes (`--features prod` only), mapped read-only for the app's
    /// lifetime: replace it atomically, never modify it in place.
    pub fn payload_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.payload = Some(PayloadSource::File(path.into()));
        self
    }

    /// Owned `.skpk` bytes. Prefer [`Self::payload_static`] for `include_bytes!`.
    pub fn payload_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.payload = Some(PayloadSource::Bytes(bytes.into()));
        self
    }

    /// A static `.skpk`, borrowed without copying.
    pub fn payload_static(mut self, bytes: &'static [u8]) -> Self {
        self.payload = Some(PayloadSource::Static(bytes));
        self
    }

    /// `appLaunch.argv` after the exe; none of it is read as a host flag.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// A native module the app imports as `native:<name>`: a value of a `#[skathi::module]` type,
    /// or a closure `|m: &mut Module| …` that registers its functions, tasks and events.
    pub fn module(mut self, name: impl Into<String>, exports: impl IntoModule) -> Self {
        let mut module = Module::new(name.into());
        exports.register(&mut module);
        self.modules.push(module);
        self
    }

    /// Runs module tasks on the embedder's own executor (a tokio or rayon pool) instead of
    /// Skathi's four worker threads. A job it drops unrun rejects its promise.
    pub fn task_executor<F>(mut self, executor: F) -> Self
    where
        F: Fn(Job) + Send + Sync + 'static,
    {
        self.executor = Some(Arc::new(executor));
        self
    }

    /// A function the app calls as `appHost.call(name, ...args)`; the handler gets the arguments
    /// as a JSON array and answers through the [`Call`].
    #[deprecated(
        since = "0.4.0",
        note = "use native modules: `Builder::module` and `import { … } from \"native:<name>\"`"
    )]
    pub fn function<F>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Fn(Call) + Send + Sync + 'static,
    {
        self.functions.insert(name.into(), Arc::new(handler));
        self
    }

    /// Whether the primary window is shown at startup (default true). Hidden apps still boot JS and
    /// pump background work, and do not activate or raise an already running instance.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Whether the app's window takes the foreground when it opens (default `true`).
    pub fn activate(mut self, on: bool) -> Self {
        self.activate = on;
        self
    }

    /// A sk-app.exe of this Skathi version, started as the inspector on F12 and
    /// `appDevtools.open()`. Without it they say why nothing opened; the devtools protocol works either way.
    pub fn inspector_exe(mut self, path: impl Into<PathBuf>) -> Self {
        self.inspector_exe = Some(path.into());
        self
    }

    /// The app's desktop entry id on Linux (`gl.example.App`): the Wayland app id and X11 class of
    /// its windows, which the desktop matches to the entry's icon and dock slot. Ignored elsewhere.
    pub fn desktop_id(mut self, id: impl Into<String>) -> Self {
        self.desktop_id = Some(id.into());
        self
    }

    /// Starts Skathi on a thread of its own and returns at once; events emitted before the app
    /// is ready wait for it. On macOS this returns `Error::MainThreadRequired`: use
    /// `Self::run_on_main_thread` instead.
    pub fn spawn(self) -> Result<Handle, Error> {
        self.answer_host_types();
        #[cfg(target_os = "macos")]
        {
            Err(Error::MainThreadRequired)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let handle = self.prepare()?;
            std::thread::Builder::new()
                .name("skathi".into())
                .stack_size(STACK)
                .spawn(run_thread)
                .map_err(Error::Spawn)?;
            Ok(handle)
        }
    }

    /// Runs AppKit and JSC on the process main thread until the app closes, then owns the rest of the
    /// process: the cleanup callback runs on a separate lifecycle thread after shutdown hooks (or the
    /// deadline), should stop workers and flush logs, and returns the exit code. A panic in it exits
    /// with code 1. Startup validation errors return normally; calls from a background thread fail
    /// before consuming the one-run state.
    #[cfg(target_os = "macos")]
    pub fn run_on_main_thread<F>(self, on_exit: F) -> Result<(), Error>
    where
        F: FnOnce(Exit) -> i32 + Send + 'static,
    {
        self.answer_host_types();
        require_main_thread()?;
        let handle = self.prepare()?;
        std::thread::Builder::new()
            .name("skathi-lifecycle".into())
            .spawn(move || {
                let exit = handle.wait();
                let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_exit(exit)))
                    .unwrap_or_else(|_| {
                        eprintln!("[embed] the application shutdown callback panicked");
                        1
                    });
                std::process::exit(code);
            })
            .map_err(Error::Spawn)?;
        run_thread();
        Ok(())
    }

    /// `<exe> --sk-host-types` (what `sk types` and `sk dev --exe` run): prints the modules'
    /// declarations and ends the process before anything starts.
    fn answer_host_types(&self) {
        if std::env::args_os()
            .nth(1)
            .is_none_or(|a| a != HOST_TYPES_FLAG)
        {
            return;
        }
        let result = self
            .module_problem()
            .map_or_else(|| host_types(&self.modules), Err);
        match result {
            Ok(text) => {
                print!("{text}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }

    fn module_problem(&self) -> Option<String> {
        for (i, m) in self.modules.iter().enumerate() {
            if let Some(problem) = m.problem() {
                return Some(problem.to_owned());
            }
            if self.modules[..i].iter().any(|other| other.name == m.name) {
                return Some(format!("native:{} is registered twice", m.name));
            }
        }
        None
    }

    fn prepare(mut self) -> Result<Handle, Error> {
        if let Some(problem) = self.module_problem() {
            return Err(Error::Module(problem));
        }
        let data_dir = resolve_data_dir(self.data_dir.take(), std::env::var_os("SK_DATA_DIR"))?;
        #[cfg(not(feature = "storages"))]
        if self.storage_encrypted.is_some() {
            return Err(Error::DataDirectory(
                "storage_encrypted requires the storages feature".into(),
            ));
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
            let args = sk_desktop::args::process();
            let launch = crate::boot_mode::launch_config();
            if let sk_desktop::launch::Outcome::Exit(code) =
                sk_desktop::launch::start_with_activation(
                    args,
                    launch.as_deref(),
                    crate::boot_mode::app_name(),
                    None,
                    requests_activation(self.activate, self.visible),
                )
            {
                return Err(Error::LaunchExit(code));
            }
            sk_desktop::launch::register_declared();
        } else {
            let exe = std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !sk_desktop::args::set_embedded(exe, self.args) {
                return Err(Error::ArgsAlreadyRead);
            }
        }
        prepare_directory(data_dir.as_deref(), self.prepare_data_dir.take())?;
        let config = Config {
            data_dir,
            #[cfg(feature = "storages")]
            storage_encrypted: self.storage_encrypted,
            visible: self.visible,
            desktop_launch: self.desktop_launch,
            setup: SetupHook(Mutex::new(self.setup)),
            functions: self.functions,
            modules: self.modules,
            executor: self.executor,
            activate: self.activate,
            inspector_exe: self.inspector_exe,
            desktop_id: self.desktop_id,
        };
        if CONFIG.set(config).is_err() {
            return Err(Error::AlreadyStarted);
        }
        Ok(Handle { _private: () })
    }

    #[cfg(feature = "prod")]
    fn load_payload(&mut self) -> Result<(), Error> {
        let Some(source) = self.payload.take() else {
            return if crate::payload::get().is_some() {
                Ok(())
            } else {
                Err(Error::NoPayload)
            };
        };
        let (bytes, label, dir) = source.load()?;
        crate::payload::set_external(bytes, label, dir.as_deref()).map_err(Error::Payload)
    }

    #[cfg(not(feature = "prod"))]
    fn load_payload(&self) -> Result<(), Error> {
        match self.payload {
            Some(_) => Err(Error::PayloadNeedsProd),
            None => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
fn require_main_thread() -> Result<(), Error> {
    unsafe extern "C" {
        fn pthread_main_np() -> std::ffi::c_int;
    }
    if unsafe { pthread_main_np() } == 0 {
        Err(Error::MainThreadRequired)
    } else {
        Ok(())
    }
}

fn run_thread() {
    // `run_embedded` ends in `shutdown::finish`, which never returns; only a panic out of gpui's
    // loop gets past it. The thread must not end even then (its thread-locals are the JS realm).
    if let Err(p) = std::panic::catch_unwind(crate::run_embedded) {
        let why = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".into());
        eprintln!("[embed] the Skathi thread panicked: {why}");
        finished(false, format!("panicked: {why}"));
    }
    loop {
        std::thread::park();
    }
}

/// How the app ended.
#[derive(Debug, Clone)]
pub struct Exit {
    /// `false` when the teardown ran past its deadline or the Skathi thread panicked.
    pub clean: bool,
    /// What closed it and how long each teardown step took (the exe's `[shutdown]` line).
    pub summary: String,
}

static EXIT: Mutex<Option<Exit>> = Mutex::new(None);
static EXIT_CV: Condvar = Condvar::new();
static CLOSE: AtomicBool = AtomicBool::new(false);
static SHOW: AtomicBool = AtomicBool::new(false);

/// The first report wins.
pub(crate) fn finished(clean: bool, summary: String) {
    let Ok(mut exit) = EXIT.lock() else { return };
    if exit.is_none() {
        *exit = Some(Exit { clean, summary });
        EXIT_CV.notify_all();
    }
}

/// Why [`Handle::emit_to`] did not queue an event.
#[derive(Debug, PartialEq, Eq)]
pub enum EmitError {
    /// `data` is not one JSON value.
    NotJson,
    /// The app has closed.
    Closed,
    /// No module of that name declared that event with `Module::event`.
    NotDeclared,
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::NotJson => write!(f, "the event's data is not JSON"),
            EmitError::Closed => write!(f, "the app has closed"),
            EmitError::NotDeclared => write!(
                f,
                "no native module declares that event: declare it with Module::event"
            ),
        }
    }
}

impl std::error::Error for EmitError {}

/// The embedder's side of a running Skathi. Cheap to clone; every method works from any thread.
#[derive(Clone)]
pub struct Handle {
    _private: (),
}

impl Handle {
    /// Delivers `data` to the `on(event)` handlers of `native:<module>` in the app's next frame.
    /// The event must be declared with [`Module::event`].
    pub fn emit_to<T: serde::Serialize + ?Sized>(
        &self,
        module: &str,
        event: &str,
        data: &T,
    ) -> Result<(), EmitError> {
        if self.is_closed() {
            return Err(EmitError::Closed);
        }
        let declared = CONFIG.get().is_some_and(|c| {
            c.modules
                .iter()
                .any(|m| m.name == module && m.declares(event))
        });
        if !declared {
            return Err(EmitError::NotDeclared);
        }
        let json = serde_json::to_string(data).map_err(|_| EmitError::NotJson)?;
        module::emit(module, event, json);
        Ok(())
    }

    /// Delivers `data` (one JSON value) to `appHost.on(event)` handlers in the app's next frame.
    #[deprecated(
        since = "0.4.0",
        note = "declare the event on a native module and send it with `Handle::emit_to`"
    )]
    pub fn emit(&self, event: &str, data: &str) -> Result<(), EmitError> {
        if self.is_closed() {
            return Err(EmitError::Closed);
        }
        if crate::json::parse(data).is_none() {
            return Err(EmitError::NotJson);
        }
        let json = format!(
            "{{\"event\":{},\"data\":{data}}}",
            crate::json::quote(event)
        );
        push(Item::Event { json });
        Ok(())
    }

    /// Queues a show/activation on the UI thread, including the first show of an initially hidden
    /// window. Returns whether it was accepted.
    pub fn show(&self) -> bool {
        if self.is_closed() || crate::shutdown::is_closing() || CLOSE.load(Ordering::Acquire) {
            return false;
        }
        SHOW.store(true, Ordering::Release);
        wake();
        true
    }

    /// Closes the app as `appWindow.close()` does: the windows go at once, the teardown runs behind them.
    pub fn close(&self) {
        CLOSE.store(true, Ordering::Release);
        wake();
    }

    pub fn is_closed(&self) -> bool {
        EXIT.lock().is_ok_and(|e| e.is_some())
    }

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

    /// One JSON array.
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

    pub fn into_reply(self) -> Reply {
        self.reply
    }
}

/// Settles one call's promise, from any thread. Dropped unanswered, it rejects, so a promise never hangs.
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
            push(Item::Settle {
                token: self.token,
                ok: true,
                text: json,
            });
        } else {
            push(Item::Settle {
                token: self.token,
                ok: false,
                text: "the host answered with text that is not JSON".into(),
            });
        }
    }

    pub fn reject(mut self, message: impl Into<String>) {
        self.settled = true;
        push(Item::Settle {
            token: self.token,
            ok: false,
            text: message.into(),
        });
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
        push(Item::Settle {
            token: self.token,
            ok: false,
            text: why.into(),
        });
    }
}

enum Item {
    Settle { token: u32, ok: bool, text: String },
    Event { json: String },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static QUEUED: AtomicUsize = AtomicUsize::new(0);
/// Calls made and not yet settled: the event loop's in-flight work.
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
        sk_runtime::redjs::web::wake(sk_runtime::redjs::context::ContextId(id));
    }
    sk_runtime::frames::wake();
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
    let budget = Duration::from_micros(sk_runtime::redjs::web::pump_budget_us() as u64);
    let start = std::time::Instant::now();
    let mut n = 0;
    while n < DRAIN_CAP {
        let Some(item) = QUEUE.lock().ok().and_then(|mut q| q.pop_front()) else {
            break;
        };
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
        let a = [
            Value::number(f64::from(token)),
            Value::boolean(ok),
            rt.ctx().string(text),
        ];
        if let Err(sk_runtime::redjs::api::Error::Exception(e)) =
            rt.ctx().call(Value(dispatcher), Value::undefined(), &a)
        {
            let err = rt.describe(e);
            eprintln!("[appHost dispatch] {}\n{}", err.header, err.stack);
        }
    });
}

pub(crate) fn setup(runtime: &JsRuntime) -> Result<(), String> {
    let Some(config) = CONFIG.get() else {
        return Ok(());
    };
    config.setup.run(&HostContext::installed(Some(runtime)))
}

/// Dock click with no window visible (`applicationShouldHandleReopen:`): as [`Handle::show`].
#[cfg(target_os = "macos")]
pub(crate) fn reopen() {
    let _accepted = Handle { _private: () }.show();
}

/// Applies a show or close the embedder asked for, from the frame's JS wave.
pub(crate) fn pump() {
    let show = SHOW.swap(false, Ordering::AcqRel);
    if CLOSE.swap(false, Ordering::AcqRel) {
        sk_tree::log::record("embed", "[embed] the embedder closed the app");
        crate::window::request_close();
    } else if show && !crate::shutdown::is_closing() {
        crate::window::request_activate();
    }
}

host_fn!(fn sk_host_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { sk_runtime::redjs::ffi::redjs_protect(g, f.0) };
    let old = DISPATCHER.with(|d| d.replace(f.0));
    if old != 0 {
        unsafe { sk_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    wake();
    Value::undefined().0
});

// Returns a token, 0 for a name the embedder did not register (the shim rejects with the list it did).
host_fn!(fn sk_host_call(g, args) {
    let name = unsafe { sk_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }.unwrap_or_default();
    let call_args = unsafe { sk_runtime::redjs::webapi::util::read_string(g, args.arg(1).0) }
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
    // A panic in the embedder's code rejects this call (via Reply's drop) rather than poisoning the runtime.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(call))).is_err() {
        eprintln!("[embed] the host function behind appHost.call panicked");
    }
    Value::number(f64::from(token)).0
});

host_fn!(fn sk_host_functions(_g, _args) {
    let mut names: Vec<&String> = CONFIG.get().map(|c| c.functions.keys().collect()).unwrap_or_default();
    names.sort();
    let json = crate::json::Json::arr(names.into_iter().map(|n| crate::json::Json::s(n.as_str())).collect()).to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

/// Embedded only: in the exe `__sk_host_*` do not exist and `appHost.available` is `false`.
pub(crate) fn install(rt: &JsRuntime) {
    if !active() {
        return;
    }
    rt.register(
        "__sk_host_set_dispatcher",
        1,
        sk_host_set_dispatcher as *const _,
    );
    rt.register("__sk_host_call", 2, sk_host_call as *const _);
    rt.register("__sk_host_functions", 0, sk_host_functions as *const _);
    let g = rt.raw();
    RT_G.store(g as usize, Ordering::Release);
    if let Some(id) = sk_runtime::redjs::context::id_of(g) {
        RT_ID.store(id.0, Ordering::Release);
        let _ = sk_runtime::redjs::web::waker(id);
    }
    sk_runtime::redjs::web::register_pump_source(sk_runtime::redjs::web::PumpSource {
        inflight: src_inflight,
        ready: src_ready,
        drain: src_drain,
    });
    if let Some(config) = CONFIG.get() {
        if let Err(e) = module::install(rt.ctx(), &config.modules, config.executor.clone()) {
            eprintln!("[embed] the native modules could not be installed: {e}");
        }
    }
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
        let (owned, _, _) = builder.payload.take().unwrap().load().unwrap();
        let crate::payload::PayloadBytes::Owned(owned) = owned else {
            panic!("owned bytes expected")
        };
        assert_eq!(owned.as_ptr(), pointer);
        assert!(builder.payload.is_none());
        static STATIC_PAYLOAD: &[u8] = b"static payload bytes";
        let mut builder = Builder::new().payload_static(STATIC_PAYLOAD);
        let (borrowed, _, _) = builder.payload.take().unwrap().load().unwrap();
        let crate::payload::PayloadBytes::Embedded(borrowed) = borrowed else {
            panic!("static bytes expected")
        };
        assert_eq!(borrowed.as_ptr(), STATIC_PAYLOAD.as_ptr());
        assert_eq!(borrowed.len(), STATIC_PAYLOAD.len());
        let mut files = crate::mapped_file::tests::Files::new();
        let path = files.write("app.skpk", b"file bytes");
        let mut builder = Builder::new().payload_file(&path);
        let (mapped, _, dir) = builder.payload.take().unwrap().load().unwrap();
        let crate::payload::PayloadBytes::File(mapped) = mapped else {
            panic!("mapped file expected")
        };
        assert_eq!(mapped.as_bytes(), b"file bytes");
        assert_eq!(dir.as_deref(), path.parent());
        assert!(builder.payload.is_none());
        #[cfg(target_os = "windows")]
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(mapped);
        std::fs::write(&path, b"released").unwrap();
    }

    #[test]
    fn data_directory_override_and_preparation_precede_storage() {
        let base = std::env::temp_dir().join(format!(
            "sk-data-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let configured = base.join("configured");
        let overridden = base.join("override");
        assert_eq!(
            resolve_data_dir(
                Some(configured.clone()),
                Some(overridden.clone().into_os_string())
            )
            .unwrap(),
            Some(overridden)
        );
        assert_eq!(
            resolve_data_dir(Some(configured.clone()), Some("".into())).unwrap(),
            Some(configured.clone())
        );
        assert_eq!(resolve_data_dir(None, None).unwrap(), None);
        assert!(resolve_data_dir(Some(PathBuf::from("relative")), None).is_err());
        assert!(prepare_directory(None, Some(Box::new(|_| Ok(())))).is_err());
        let refused = base.join("refused");
        assert!(prepare_directory(
            Some(&refused),
            Some(Box::new(|_| Err("migration refused".into())))
        )
        .is_err());
        assert!(!refused.exists());
        prepare_directory(
            Some(&configured),
            Some(Box::new(|path| {
                assert!(!path.exists());
                std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
                std::fs::write(path.join("prepared"), "profile").map_err(|e| e.to_string())
            })),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(configured.join("prepared")).unwrap(),
            "profile"
        );
        std::fs::remove_file(configured.join("prepared")).unwrap();
        std::fs::remove_dir(configured).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_rejects_worker_entry_before_startup_state_changes() {
        std::thread::spawn(|| {
            assert!(matches!(
                require_main_thread(),
                Err(Error::MainThreadRequired)
            ));
            assert!(matches!(
                Builder::new().spawn(),
                Err(Error::MainThreadRequired)
            ));
            assert!(matches!(
                Builder::new().run_on_main_thread(|_| 0),
                Err(Error::MainThreadRequired)
            ));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn setup_receives_installed_context_once_and_propagates_failure() {
        let called = Arc::new(AtomicUsize::new(0));
        let observed = called.clone();
        let hook = SetupHook(Mutex::new(Some(Box::new(move |ctx| {
            assert_eq!(
                ctx.storage_dir().ok(),
                HostContext::installed(None).storage_dir().ok()
            );
            observed.fetch_add(1, Ordering::SeqCst);
            Err("bootstrap refused".into())
        }))));
        assert_eq!(
            hook.run(&HostContext::installed(None)),
            Err("bootstrap refused".into())
        );
        assert!(hook.run(&HostContext::installed(None)).is_ok());
        assert_eq!(called.load(Ordering::SeqCst), 1);
        let panic = SetupHook(Mutex::new(Some(Box::new(|_| panic!("plugin panic")))));
        assert!(panic.run(&HostContext::installed(None)).is_err());
    }

    // One test, because the queue is the process's.
    #[test]
    #[allow(deprecated)]
    fn replies_settle_once_and_a_dropped_one_rejects() {
        drain_queue();
        Reply {
            token: 7,
            settled: false,
        }
        .resolve(r#"{"a":1}"#);
        Reply {
            token: 8,
            settled: false,
        }
        .resolve("not json");
        Reply {
            token: 9,
            settled: false,
        }
        .reject("nope");
        drop(Reply {
            token: 10,
            settled: false,
        });
        let got = drain_queue();
        assert_eq!(got[0], (7, true, r#"{"a":1}"#.to_string()));
        assert_eq!(got[1].0, 8);
        assert!(!got[1].1);
        assert_eq!(got[2], (9, false, "nope".to_string()));
        assert_eq!(got[3].0, 10);
        assert!(!got[3].1);

        let h = Handle { _private: () };
        assert_eq!(h.emit("x", "{bad"), Err(EmitError::NotJson));
        h.emit("status", r#"{"online":true}"#).unwrap();
        assert_eq!(
            drain_queue()[0].2,
            r#"{"event":"status","data":{"online":true}}"#
        );
    }
}
