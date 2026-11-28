//! The JS side: `Process`, its streams and promises. OS work runs on threads of each child (one
//! waits for it, one per pipe), whose results the stdlib pump settles on the JS thread.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use sk_runtime::host::{self, Settler};
use sk_runtime::redjs::capi;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::child::{self, Child, Exit, Request, Signal, Stdio};
use super::resolve::{self, Refusal};
use crate::permissions::{self, Permissions};
use crate::stdlib::support::{self, *};

const TA_U8: u32 = 2;
/// Bytes a pipe holds for JS before its reader stops reading, and the child blocks on writing.
pub(super) const QUEUE_LIMIT: usize = 256 * 1024;
pub(super) const READ_SIZE: usize = 64 * 1024;
/// What `Process.exec()` keeps of each stream unless `maxBuffer` says otherwise.
const MAX_BUFFER: usize = 1024 * 1024;
const OUT: usize = 0;
const ERR: usize = 1;

const SLOT_STDIN: &str = "process.stdin";
const SLOT_STDOUT: &str = "process.stdout";
const SLOT_STDERR: &str = "process.stderr";
const SLOT_EXITED: &str = "process.exited";

#[derive(Default)]
pub(super) struct PipeState {
    pub(super) chunks: VecDeque<Vec<u8>>,
    pub(super) bytes: usize,
    pub(super) done: Option<Result<(), String>>,
    pub(super) cancelled: bool,
    /// `Process.exec()`: everything up to this many bytes is kept here instead.
    pub(super) collect: Option<usize>,
    pub(super) collected: Vec<u8>,
    pub(super) over: bool,
}

#[derive(Default)]
pub(super) struct Pipe {
    pub(super) state: Mutex<PipeState>,
    pub(super) room: Condvar,
}

impl Pipe {
    fn lock(&self) -> std::sync::MutexGuard<'_, PipeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cancel(&self) {
        let mut s = self.lock();
        s.cancelled = true;
        s.chunks.clear();
        s.bytes = 0;
        drop(s);
        self.room.notify_all();
    }
}

enum Input {
    Data(Vec<u8>),
    Close,
}

#[derive(Default)]
struct Flags {
    timed_out: AtomicBool,
    aborted: AtomicBool,
}

struct Shared {
    id: u64,
    child: Arc<Child>,
    out: [Option<Arc<Pipe>>; 2],
    stdin: Mutex<Option<Sender<Input>>>,
    flags: Flags,
    timer: Arc<(Mutex<bool>, Condvar)>,
}

impl Shared {
    fn close_stdin(&self) {
        self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    fn stop_timer(&self) {
        *self.timer.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.timer.1.notify_all();
    }
}

/// The instance data of a `Process`. A process whose object is collected is killed.
struct Handle {
    shared: Arc<Shared>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shared.child.end();
        self.shared.close_stdin();
        for pipe in self.shared.out.iter().flatten() {
            pipe.cancel();
        }
    }
}

support::host_state!(handle, handle_new, Handle, PROCESS, "Process");

enum Event {
    Data(u64),
    Exited(u64, Exit),
    Wrote(u64, Result<(), String>),
}

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn post(event: Event) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    sk_runtime::frames::wake();
    crate::background::prompt();
}

/// What the JS thread still owes a child: promises to settle and values it keeps alive.
struct Entry {
    g: usize,
    shared: Arc<Shared>,
    exit: Option<Exit>,
    exited: Option<Settler>,
    pulls: [Option<(u64, Settler)>; 2],
    write: Option<Settler>,
    exec: Option<Settler>,
    abort: Option<(u64, u64)>,
}

impl Entry {
    fn new(g: *mut c_void, shared: Arc<Shared>) -> Entry {
        Entry {
            g: g as usize,
            shared,
            exit: None,
            exited: None,
            pulls: [None, None],
            write: None,
            exec: None,
            abort: None,
        }
    }

    fn idle(&self) -> bool {
        self.exited.is_none()
            && self.pulls.iter().all(Option::is_none)
            && self.write.is_none()
            && self.exec.is_none()
    }
}

thread_local! {
    static ENTRIES: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
}

fn with_entry<R>(id: u64, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    ENTRIES.with(|e| e.borrow_mut().get_mut(&id).map(f))
}

#[cfg(test)]
thread_local! {
    pub(super) static TEST_PERMISSIONS: RefCell<Option<&'static Permissions>> = const { RefCell::new(None) };
}

fn allowed() -> &'static Permissions {
    #[cfg(test)]
    if let Some(p) = TEST_PERMISSIONS.with(|p| *p.borrow()) {
        return p;
    }
    permissions::current()
}

unsafe fn dom<T>(g: *mut c_void, name: &str, message: &str) -> Js<T> {
    unsafe {
        host::throw(g, name, message);
        Err(Thrown)
    }
}

unsafe fn pending_exception(g: *mut c_void) -> u64 {
    unsafe {
        let mut error = UNDEF;
        glue::rjs_exception_pending(g, &mut error);
        error
    }
}

fn protect(g: *mut c_void, v: u64) {
    // SAFETY: a live value of this context.
    unsafe { capi::JSValueProtect(g as _, v as usize as _) };
}

fn unprotect(g: *mut c_void, v: u64) {
    // SAFETY: protected by `protect`.
    unsafe { capi::JSValueUnprotect(g as _, v as usize as _) };
}

pub(super) fn reader(id: u64, mut file: File, pipe: Arc<Pipe>, child: Arc<Child>) {
    let mut buf = vec![0u8; READ_SIZE];
    let end = loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == ErrorKind::BrokenPipe => break Ok(()),
            Err(e) => break Err(e.to_string()),
        };
        let mut s = pipe.lock();
        if let Some(cap) = s.collect {
            if !s.over && s.collected.len() + n > cap {
                s.over = true;
                drop(s);
                child.kill(Signal::Kill);
            } else if !s.over {
                s.collected.extend_from_slice(&buf[..n]);
            }
            continue;
        }
        while s.bytes >= QUEUE_LIMIT && !s.cancelled {
            s = pipe.room.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        if s.cancelled {
            continue;
        }
        s.chunks.push_back(buf[..n].to_vec());
        s.bytes += n;
        drop(s);
        post(Event::Data(id));
    };
    pipe.lock().done = Some(end);
    post(Event::Data(id));
}

fn writer(id: u64, file: File, input: Receiver<Input>) {
    let mut file = Some(file);
    for message in input {
        match message {
            Input::Data(bytes) => {
                let wrote = match file.as_mut() {
                    Some(f) => f.write_all(&bytes).map_err(|e| e.to_string()),
                    None => Err("stdin is closed".into()),
                };
                if wrote.is_err() {
                    file = None;
                }
                post(Event::Wrote(id, wrote));
            }
            Input::Close => {
                drop(file.take());
                post(Event::Wrote(id, Ok(())));
                return;
            }
        }
    }
}

unsafe fn make_u8(g: *mut c_void, bytes: &[u8]) -> Js<u64> {
    unsafe {
        let (array, out) = support::typed_array(g, TA_U8, bytes.len())?;
        out.copy_from_slice(bytes);
        Ok(array)
    }
}

/// Hands the pipe's bytes, its end or its error to `controller`; false when there is nothing yet.
unsafe fn deliver(g: *mut c_void, pipe: &Pipe, controller: u64) -> bool {
    unsafe {
        let mut s = pipe.lock();
        if !s.chunks.is_empty() {
            let mut bytes = Vec::with_capacity(s.bytes);
            for chunk in s.chunks.drain(..) {
                bytes.extend_from_slice(&chunk);
            }
            s.bytes = 0;
            drop(s);
            pipe.room.notify_all();
            if let Ok(array) = make_u8(g, &bytes) {
                let _ = call(g, get(g, controller, "enqueue"), controller, &[array]);
            } else {
                pending_exception(g);
            }
            return true;
        }
        let done = s.done.clone();
        drop(s);
        match done {
            Some(Ok(())) => {
                let _ = call(g, get(g, controller, "close"), controller, &[]);
            }
            Some(Err(message)) => {
                let error = host::error(g, "NotReadableError", &message);
                let _ = call(g, get(g, controller, "error"), controller, &[error]);
            }
            None => return false,
        }
        true
    }
}

unsafe fn which(g: *mut c_void, v: u64) -> Js<usize> {
    unsafe {
        Ok(if support::number(g, v)? == 1.0 {
            ERR
        } else {
            OUT
        })
    }
}

// The stream callbacks are bound to the `Process` (this) and the pipe (first argument).

std_fn!(fn source_pull(g, args) {
    let state = handle(g, args.this().0)?;
    let which = which(g, args.arg(0).0)?;
    let controller = args.arg(1).0;
    let shared = state.shared.clone();
    let Some(pipe) = shared.out[which].clone() else { return Ok(UNDEF) };
    if deliver(g, &pipe, controller) {
        return Ok(UNDEF);
    }
    let (promise, settler) = host::deferred(g);
    protect(g, controller);
    ENTRIES.with(|e| {
        let mut entries = e.borrow_mut();
        let entry = entries.entry(shared.id).or_insert_with(|| Entry::new(g, shared.clone()));
        if let Some((old, settler)) = entry.pulls[which].replace((controller, settler)) {
            settler.resolve(g, UNDEF);
            unprotect(g, old);
        }
    });
    Ok(promise)
});

std_fn!(fn source_cancel(g, args) {
    let state = handle(g, args.this().0)?;
    let which = which(g, args.arg(0).0)?;
    let shared = state.shared.clone();
    if let Some(pipe) = &shared.out[which] {
        pipe.cancel();
    }
    let pending = with_entry(shared.id, |e| e.pulls[which].take()).flatten();
    if let Some((controller, settler)) = pending {
        settler.resolve(g, UNDEF);
        unprotect(g, controller);
    }
    Ok(UNDEF)
});

unsafe fn send(g: *mut c_void, this: u64, input: Input) -> Js<u64> {
    unsafe {
        let shared = handle(g, this)?.shared.clone();
        let closing = matches!(input, Input::Close);
        let sent = {
            let mut sender = shared.stdin.lock().unwrap_or_else(|e| e.into_inner());
            let ok = sender.as_ref().is_some_and(|s| s.send(input).is_ok());
            if closing {
                sender.take();
            }
            ok
        };
        if !sent {
            return Ok(host::rejected(
                g,
                "InvalidStateError",
                "Process.stdin: the process's stdin is closed",
            ));
        }
        let (promise, settler) = host::deferred(g);
        ENTRIES.with(|e| {
            let mut entries = e.borrow_mut();
            let entry = entries
                .entry(shared.id)
                .or_insert_with(|| Entry::new(g, shared.clone()));
            entry.write = Some(settler);
        });
        Ok(promise)
    }
}

std_fn!(fn sink_write(g, args) {
    let bytes = support::bytes_arg(g, args.arg(0).0, "Process.stdin: a chunk")?.to_vec();
    send(g, args.this().0, Input::Data(bytes))
});

std_fn!(fn sink_close(g, args) { send(g, args.this().0, Input::Close) });

std_fn!(fn sink_abort(g, args) {
    handle(g, args.this().0)?.shared.close_stdin();
    Ok(UNDEF)
});

std_fn!(fn on_abort(g, args) {
    let id = support::number(g, args.arg(0).0)? as u64;
    with_entry(id, |e| {
        e.shared.flags.aborted.store(true, Ordering::Release);
        e.shared.child.kill(Signal::Term);
    });
    Ok(UNDEF)
});

unsafe fn bound(g: *mut c_void, f: u64, bind: &[u64]) -> Js<u64> {
    unsafe {
        call(g, get(g, f, "bind"), f, bind).map_err(|e| {
            glue::rjs_throw(g, e);
            Thrown
        })
    }
}

unsafe fn readable(g: *mut c_void, process: u64, which: usize) -> Js<u64> {
    unsafe {
        let source = new_object(g, UNDEF);
        let n = enc_f64(which as f64);
        define(
            g,
            source,
            "pull",
            bound(g, make_fn(g, "pull", 2, source_pull), &[process, n])?,
            0,
        );
        define(
            g,
            source,
            "cancel",
            bound(g, make_fn(g, "cancel", 2, source_cancel), &[process, n])?,
            0,
        );
        let strategy = new_object(g, UNDEF);
        define(g, strategy, "highWaterMark", enc_f64(0.0), 0);
        support::construct(
            g,
            get(g, glue::rjs_global(g), "ReadableStream"),
            &[source, strategy],
        )
    }
}

unsafe fn writable(g: *mut c_void, process: u64) -> Js<u64> {
    unsafe {
        let sink = new_object(g, UNDEF);
        for (name, arity, f) in [
            ("write", 1, sink_write as HostFn),
            ("close", 0, sink_close as HostFn),
            ("abort", 1, sink_abort as HostFn),
        ] {
            define(
                g,
                sink,
                name,
                bound(g, make_fn(g, name, arity, f), &[process])?,
                0,
            );
        }
        support::construct(g, get(g, glue::rjs_global(g), "WritableStream"), &[sink])
    }
}

struct Options {
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    stdio: [Stdio; 3],
    signal: Option<u64>,
    timeout: Option<Duration>,
    max_buffer: usize,
}

unsafe fn strings(g: *mut c_void, v: u64, what: &str) -> Js<Vec<String>> {
    unsafe {
        if v == UNDEF {
            return Ok(Vec::new());
        }
        let is_array = to_bool(
            g,
            call(
                g,
                get(g, get(g, glue::rjs_global(g), "Array"), "isArray"),
                UNDEF,
                &[v],
            )
            .unwrap_or(UNDEF),
        );
        if !is_array {
            return type_error(g, &format!("{what} must be an array of strings"));
        }
        let n = support::count(g, get(g, v, "length"), what)?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let s = support::string(g, get_i(g, v, i as u32), &format!("{what}[{i}]"))?;
            if s.contains('\0') {
                return type_error(g, &format!("{what}[{i}] contains a NUL character"));
            }
            out.push(s);
        }
        Ok(out)
    }
}

unsafe fn options(g: *mut c_void, bag: u64, what: &str, exec: bool) -> Js<Options> {
    unsafe {
        if bag != UNDEF && !is_cell(bag) {
            return type_error(g, &format!("{what}: options must be an object"));
        }
        let cwd = match support::option(g, bag, "cwd") {
            UNDEF => None,
            v => {
                let dir = PathBuf::from(support::string(g, v, &format!("{what}: cwd"))?);
                if !dir.is_absolute() {
                    return type_error(g, &format!("{what}: cwd must be an absolute path"));
                }
                if !dir.is_dir() {
                    return dom(
                        g,
                        "NotFoundError",
                        &format!("{what}: cwd {} is not a folder", dir.display()),
                    );
                }
                Some(dir)
            }
        };
        let mut env = Vec::new();
        match support::option(g, bag, "env") {
            UNDEF => {}
            v if is_cell(v) => {
                let entries = call(
                    g,
                    get(g, get(g, glue::rjs_global(g), "Object"), "entries"),
                    UNDEF,
                    &[v],
                )
                .map_err(|e| {
                    glue::rjs_throw(g, e);
                    Thrown
                })?;
                for i in 0..support::count(g, get(g, entries, "length"), "env")? {
                    let pair = get_i(g, entries, i as u32);
                    let key = support::string(g, get_i(g, pair, 0), &format!("{what}: env"))?;
                    let value =
                        support::string(g, get_i(g, pair, 1), &format!("{what}: env.{key}"))?;
                    if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
                        return type_error(g, &format!("{what}: env.{key}: a name is not empty and has no \"=\", and neither has a NUL character"));
                    }
                    if resolve::forbidden_key(&key) {
                        return dom(g, "NotAllowedError", &format!("{what}: env.{key} decides what the program loads or runs, and app code cannot set it"));
                    }
                    env.push((key, value));
                }
            }
            _ => return type_error(g, &format!("{what}: env must be an object of strings")),
        }
        let mut stdio = [Stdio::Pipe; 3];
        for (i, name) in ["stdin", "stdout", "stderr"].iter().enumerate() {
            let picked = support::choice(
                g,
                support::option(g, bag, name),
                &["pipe", "ignore"],
                0,
                &format!("{what}: {name}"),
            )?;
            stdio[i] = if picked == 0 {
                Stdio::Pipe
            } else {
                Stdio::Null
            };
        }
        if exec {
            stdio[0] = Stdio::Null;
        }
        let signal = match support::option(g, bag, "signal") {
            UNDEF => None,
            v if is_cell(v) => {
                if to_bool(g, get(g, v, "aborted")) {
                    return dom(
                        g,
                        "AbortError",
                        &format!("{what}: the signal is already aborted"),
                    );
                }
                Some(v)
            }
            _ => return type_error(g, &format!("{what}: signal must be an AbortSignal")),
        };
        let mut timeout = None;
        let mut max_buffer = MAX_BUFFER;
        if exec {
            let v = support::option(g, bag, "timeout");
            if v != UNDEF {
                let ms = support::number(g, v)?;
                if !(ms >= 0.0 && ms.is_finite()) {
                    return range_error(
                        g,
                        &format!("{what}: timeout must be a number of milliseconds"),
                    );
                }
                if ms > 0.0 {
                    timeout = Some(Duration::from_secs_f64(ms / 1000.0));
                }
            }
            let v = support::option(g, bag, "maxBuffer");
            if v != UNDEF {
                max_buffer = support::count(g, v, &format!("{what}: maxBuffer"))?;
            }
        }
        Ok(Options {
            cwd,
            env,
            stdio,
            signal,
            timeout,
            max_buffer,
        })
    }
}

fn appdir() -> Option<String> {
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some() {
        std::env::var("APPDIR").ok()
    } else {
        None
    }
}

/// Starts the program, or throws what refused it.
unsafe fn start(
    g: *mut c_void,
    what: &str,
    program: &str,
    argv: &[String],
    options: &Options,
    collect: Option<usize>,
) -> Js<Arc<Shared>> {
    unsafe {
        let search = std::env::var_os("PATH");
        let path = match resolve::resolve(allowed(), program, argv, search.as_deref()) {
            Ok(path) => path,
            Err(Refusal::NotAllowed(m)) => {
                return dom(g, "NotAllowedError", &format!("{what}: {m}"))
            }
            Err(Refusal::NotFound(m)) => return dom(g, "NotFoundError", &format!("{what}: {m}")),
        };
        if child::live() >= child::LIVE_LIMIT {
            return dom(
                g,
                "QuotaExceededError",
                &format!(
                    "{what}: {} processes are already running",
                    child::LIVE_LIMIT
                ),
            );
        }
        let env = resolve::child_env(std::env::vars_os(), &options.env, appdir().as_deref());
        let request = Request {
            program: &path,
            args: argv,
            env,
            cwd: options.cwd.as_deref(),
            stdin: options.stdio[0],
            stdout: options.stdio[1],
            stderr: options.stdio[2],
        };
        let spawned = match child::spawn(request) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return dom(
                    g,
                    "NotFoundError",
                    &format!("{what}: {}: {e}", path.display()),
                )
            }
            Err(e) => {
                return dom(
                    g,
                    "OperationError",
                    &format!("{what}: {} could not be started: {e}", path.display()),
                )
            }
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let pipe = |file: &Option<File>| {
            file.as_ref().map(|_| {
                Arc::new(Pipe {
                    state: Mutex::new(PipeState {
                        collect,
                        ..PipeState::default()
                    }),
                    room: Condvar::new(),
                })
            })
        };
        let out = [pipe(&spawned.stdout), pipe(&spawned.stderr)];
        let (sender, input) = mpsc::channel();
        let shared = Arc::new(Shared {
            id,
            child: spawned.child.clone(),
            out: out.clone(),
            stdin: Mutex::new(spawned.stdin.is_some().then_some(sender)),
            flags: Flags::default(),
            timer: Arc::new((Mutex::new(false), Condvar::new())),
        });
        let thread = |name: &str, run: Box<dyn FnOnce() + Send>| {
            let _ = std::thread::Builder::new().name(name.into()).spawn(run);
        };
        for (file, pipe) in [spawned.stdout, spawned.stderr].into_iter().zip(out) {
            if let (Some(file), Some(pipe)) = (file, pipe) {
                let child = spawned.child.clone();
                thread(
                    "skathi-process-read",
                    Box::new(move || reader(id, file, pipe, child)),
                );
            }
        }
        if let Some(file) = spawned.stdin {
            thread(
                "skathi-process-write",
                Box::new(move || writer(id, file, input)),
            );
        }
        let child = spawned.child.clone();
        thread(
            "skathi-process-wait",
            Box::new(move || {
                let exit = child.wait();
                post(Event::Exited(id, exit));
            }),
        );
        if let Some(limit) = options.timeout {
            let shared = shared.clone();
            thread(
                "skathi-process-timeout",
                Box::new(move || {
                    let (lock, cv) = &*shared.timer;
                    let stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let (_stopped, waited) = cv
                        .wait_timeout_while(stopped, limit, |stopped| !*stopped)
                        .unwrap_or_else(|e| e.into_inner());
                    if waited.timed_out() {
                        shared.flags.timed_out.store(true, Ordering::Release);
                        shared.child.kill(Signal::Kill);
                    }
                }),
            );
        }
        Ok(shared)
    }
}

unsafe fn listen_abort(g: *mut c_void, entry: &mut Entry, signal: Option<u64>) {
    unsafe {
        let Some(signal) = signal else { return };
        let listener = make_fn(g, "abort", 2, on_abort);
        let Ok(listener) = call(
            g,
            get(g, listener, "bind"),
            listener,
            &[UNDEF, enc_f64(entry.shared.id as f64)],
        ) else {
            return;
        };
        if call(
            g,
            get(g, signal, "addEventListener"),
            signal,
            &[js_string(g, "abort"), listener],
        )
        .is_ok()
        {
            protect(g, signal);
            protect(g, listener);
            entry.abort = Some((signal, listener));
        }
    }
}

unsafe fn forget(g: *mut c_void, entry: Entry) {
    unsafe {
        if let Some((signal, listener)) = entry.abort {
            let _ = call(
                g,
                get(g, signal, "removeEventListener"),
                signal,
                &[js_string(g, "abort"), listener],
            );
            unprotect(g, signal);
            unprotect(g, listener);
        }
    }
}

std_fn!(fn process_spawn(g, args) {
    let what = "Process.spawn";
    let program = support::string(g, args.arg(0).0, &format!("{what}: the program"))?;
    let argv = strings(g, args.arg(1).0, &format!("{what}: args"))?;
    let options = options(g, args.arg(2).0, what, false)?;
    let shared = start(g, what, &program, &argv, &options, None)?;
    let process = handle_new(g, Handle { shared: shared.clone() });
    let streams = [
        (SLOT_STDIN, options.stdio[0], None),
        (SLOT_STDOUT, options.stdio[1], Some(OUT)),
        (SLOT_STDERR, options.stdio[2], Some(ERR)),
    ];
    for (slot, mode, which) in streams {
        let stream = match (mode, which) {
            (Stdio::Null, _) => NULL,
            (Stdio::Pipe, None) => writable(g, process)?,
            (Stdio::Pipe, Some(which)) => readable(g, process, which)?,
        };
        support::slot_set(g, process, slot, stream);
    }
    let (exited, settler) = host::deferred(g);
    support::slot_set(g, process, SLOT_EXITED, exited);
    let mut entry = Entry::new(g, shared.clone());
    entry.exited = Some(settler);
    listen_abort(g, &mut entry, options.signal);
    ENTRIES.with(|e| e.borrow_mut().insert(shared.id, entry));
    Ok(process)
});

std_fn!(fn process_exec(g, args) {
    let what = "Process.exec";
    let run = || -> Js<u64> {
        let program = support::string(g, args.arg(0).0, &format!("{what}: the program"))?;
        let argv = strings(g, args.arg(1).0, &format!("{what}: args"))?;
        let options = options(g, args.arg(2).0, what, true)?;
        let shared = start(g, what, &program, &argv, &options, Some(options.max_buffer))?;
        let (promise, settler) = host::deferred(g);
        let mut entry = Entry::new(g, shared.clone());
        entry.exec = Some(settler);
        listen_abort(g, &mut entry, options.signal);
        ENTRIES.with(|e| e.borrow_mut().insert(shared.id, entry));
        Ok(promise)
    };
    Ok(run().unwrap_or_else(|Thrown| host::rejected_with(g, pending_exception(g))))
});

std_fn!(fn process_kill(g, args) {
    let signal = support::choice(g, args.arg(0).0, &["SIGTERM", "SIGKILL"], 0, "Process.kill: the signal")?;
    let state = handle(g, args.this().0)?;
    state.shared.child.kill(if signal == 0 { Signal::Term } else { Signal::Kill });
    Ok(UNDEF)
});

std_fn!(fn process_pid(g, args) { Ok(enc_f64(handle(g, args.this().0)?.shared.child.pid as f64)) });
std_fn!(fn process_stdin(g, args) { handle(g, args.this().0)?; Ok(support::slot_get(g, args.this().0, SLOT_STDIN)) });
std_fn!(fn process_stdout(g, args) { handle(g, args.this().0)?; Ok(support::slot_get(g, args.this().0, SLOT_STDOUT)) });
std_fn!(fn process_stderr(g, args) { handle(g, args.this().0)?; Ok(support::slot_get(g, args.this().0, SLOT_STDERR)) });
std_fn!(fn process_exited(g, args) { handle(g, args.this().0)?; Ok(support::slot_get(g, args.this().0, SLOT_EXITED)) });

pub(super) unsafe fn class(g: *mut c_void) -> u64 {
    unsafe {
        let class = support::class(
            g,
            &ClassSpec {
                name: "Process",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[("kill", 0, process_kill)],
                getters: &[
                    ("pid", process_pid),
                    ("stdin", process_stdin),
                    ("stdout", process_stdout),
                    ("stderr", process_stderr),
                    ("exited", process_exited),
                ],
                accessors: &[],
            },
        );
        static_fn(g, class.ctor, "spawn", 3, process_spawn);
        static_fn(g, class.ctor, "exec", 3, process_exec);
        class.ctor
    }
}

fn exit_code(exit: &Exit) -> u64 {
    match exit {
        Exit::Code(code) => enc_f64(*code as f64),
        Exit::Signal(_) => NULL,
    }
}

unsafe fn text(g: *mut c_void, pipe: &Option<Arc<Pipe>>) -> u64 {
    unsafe {
        let bytes = pipe
            .as_ref()
            .map(|p| std::mem::take(&mut p.lock().collected))
            .unwrap_or_default();
        js_string(g, &String::from_utf8_lossy(&bytes))
    }
}

/// `Process.exec()` settles once the child has exited and both of its streams have ended.
unsafe fn finish_exec(g: *mut c_void, entry: &mut Entry) {
    unsafe {
        let Some(exit) = entry.exit.clone() else {
            return;
        };
        let ended = entry
            .shared
            .out
            .iter()
            .flatten()
            .all(|p| p.lock().done.is_some());
        if !ended {
            return;
        }
        let Some(settler) = entry.exec.take() else {
            return;
        };
        entry.shared.stop_timer();
        entry.shared.child.end();
        let shared = &entry.shared;
        let over = shared
            .out
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.lock().over));
        if shared.flags.aborted.load(Ordering::Acquire) {
            return settler.reject(
                g,
                host::error(
                    g,
                    "AbortError",
                    "Process.exec: the signal aborted it, and the process was killed",
                ),
            );
        }
        if shared.flags.timed_out.load(Ordering::Acquire) {
            return settler.reject(
                g,
                host::error(
                    g,
                    "TimeoutError",
                    "Process.exec: the process passed its timeout and was killed",
                ),
            );
        }
        if let Some(which) = over {
            let name = if which == OUT { "stdout" } else { "stderr" };
            throw_error(g, support::RANGE_ERROR, &format!("Process.exec: {name} passed maxBuffer, and the process was killed; raise maxBuffer or use Process.spawn() and read the stream"));
            return settler.reject(g, pending_exception(g));
        }
        let result = new_object(g, UNDEF);
        let code = exit_code(&exit);
        let signal = match &exit {
            Exit::Signal(name) => js_string(g, name),
            Exit::Code(_) => NULL,
        };
        define(g, result, "code", code, 1 | 4);
        define(g, result, "signal", signal, 1 | 4);
        define(g, result, "stdout", text(g, &shared.out[OUT]), 1 | 4);
        define(g, result, "stderr", text(g, &shared.out[ERR]), 1 | 4);
        settler.resolve(g, result);
    }
}

/// Settles what the threads of every child finished since the last pump.
pub(super) fn pump() {
    let events = std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()));
    for event in events {
        let id = match &event {
            Event::Data(id) | Event::Exited(id, _) | Event::Wrote(id, _) => *id,
        };
        // Taken out while JS runs: a callback may start or settle another process.
        let Some(mut entry) = ENTRIES.with(|e| e.borrow_mut().remove(&id)) else {
            continue;
        };
        let g = entry.g as *mut c_void;
        // SAFETY: on the JS thread of the context the entry was made in.
        unsafe {
            match event {
                Event::Data(_) => {
                    for which in [OUT, ERR] {
                        let Some(pipe) = entry.shared.out[which].clone() else {
                            continue;
                        };
                        if let Some((controller, settler)) = entry.pulls[which].take() {
                            if deliver(g, &pipe, controller) {
                                settler.resolve(g, UNDEF);
                                unprotect(g, controller);
                            } else {
                                entry.pulls[which] = Some((controller, settler));
                            }
                        }
                    }
                }
                Event::Exited(_, exit) => {
                    // A helper the program left behind must not hold its output open.
                    if entry.exec.is_some() {
                        entry.shared.child.end();
                    }
                    if let Some(settler) = entry.exited.take() {
                        settler.resolve(g, exit_code(&exit));
                    }
                    entry.exit = Some(exit);
                }
                Event::Wrote(_, wrote) => {
                    if let Some(settler) = entry.write.take() {
                        match wrote {
                            Ok(()) => settler.resolve(g, UNDEF),
                            Err(m) => settler.reject(
                                g,
                                host::error(g, "InvalidStateError", &format!("Process.stdin: {m}")),
                            ),
                        }
                    }
                }
            }
            finish_exec(g, &mut entry);
            if entry.idle() {
                forget(g, entry);
            } else {
                ENTRIES.with(|e| {
                    let mut entries = e.borrow_mut();
                    match entries.remove(&id) {
                        Some(newer) => {
                            entries.insert(id, merge(g, entry, newer));
                        }
                        None => {
                            entries.insert(id, entry);
                        }
                    }
                });
            }
        }
    }
}

/// A pull or write registered while the entry was out.
unsafe fn merge(g: *mut c_void, mut entry: Entry, newer: Entry) -> Entry {
    unsafe {
        for (slot, pull) in entry.pulls.iter_mut().zip(newer.pulls) {
            if pull.is_some() {
                *slot = pull;
            }
        }
        if newer.write.is_some() {
            entry.write = newer.write;
        }
        forget(
            g,
            Entry {
                pulls: [None, None],
                write: None,
                ..newer
            },
        );
        entry
    }
}
