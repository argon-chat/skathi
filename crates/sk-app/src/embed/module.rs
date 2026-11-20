//! Native modules: typed functions, tasks and events an embedder gives the app as `native:<name>`.
//!
//! Arguments are read from the engine's values straight into serde types and results built
//! straight into engine values, with the meaning a JSON round trip gives them (`marshal`). A
//! function runs inside the JS call; a task reads its arguments in the call, runs on a worker
//! and settles a promise in a later frame.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value as Json;
use sk_runtime::JsRuntime;

use super::marshal;
use crate::js::{self, Context, HostArgs, Value};

#[cfg(feature = "types")]
use specta::{datatype::DataType, Types};

/// What every argument, result and event type implements: `serde` plus, with the `types` feature,
/// [`specta::Type`] (`#[derive(skathi::Type)]`).
#[cfg(feature = "types")]
pub trait Typed: specta::Type {}
#[cfg(feature = "types")]
impl<T: specta::Type + ?Sized> Typed for T {}
#[cfg(not(feature = "types"))]
pub trait Typed {}
#[cfg(not(feature = "types"))]
impl<T: ?Sized> Typed for T {}

/// Why a call failed, as the app sees it.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    message: String,
    data: Option<String>,
    kind: u32,
}

impl Failure {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            data: None,
            kind: 0,
        }
    }

    fn type_error(message: impl Into<String>) -> Self {
        Self {
            kind: 1,
            ..Self::new(message)
        }
    }

    /// An `Err` value: a string is the message; anything else rides along as `error.data`.
    fn from_err(json: String) -> Self {
        let message = match serde_json::from_str::<Json>(&json) {
            Ok(Json::String(text)) => return Self::new(text),
            Ok(Json::Object(fields)) => match fields.get("message") {
                Some(Json::String(text)) => text.clone(),
                _ => json.clone(),
            },
            _ => json.clone(),
        };
        Self {
            data: Some(json),
            ..Self::new(message)
        }
    }
}

impl From<Failure> for Fail {
    fn from(failure: Failure) -> Self {
        Fail::Failure(failure)
    }
}

/// Why a call failed: a JS exception thrown while reading the arguments, or a [`Failure`].
#[doc(hidden)]
#[derive(Debug)]
pub enum Fail {
    Thrown(Value),
    Failure(Failure),
}

/// A task's work, made in the call from its arguments and run on a worker.
#[doc(hidden)]
pub type Work = Box<dyn FnOnce() -> Result<marshal::Owned, Failure> + Send + 'static>;

/// A Rust closure the app can call: `Fn(A1, ..., An) -> R` for up to eight arguments, each
/// `Deserialize`, with `R: Serialize`. An `Err(e)` result throws with `e`.
pub trait Function<Args>: Send + Sync + 'static {
    #[doc(hidden)]
    const ARITY: usize;
    #[doc(hidden)]
    fn invoke(&self, context: &Context, args: &HostArgs) -> Result<Value, Fail>;
    #[cfg(feature = "types")]
    #[doc(hidden)]
    fn signature(types: &mut Types) -> (Vec<DataType>, DataType);
}

/// A [`Function`] run on a worker: its arguments are read in the call, so they are also `Send`.
/// An `Err(e)` result rejects with `e`.
pub trait Task<Args>: Send + Sync + 'static {
    #[doc(hidden)]
    const ARITY: usize;
    #[doc(hidden)]
    fn prepare(self: Arc<Self>, context: &Context, args: &HostArgs) -> Result<Work, Fail>;
    #[cfg(feature = "types")]
    #[doc(hidden)]
    fn signature(types: &mut Types) -> (Vec<DataType>, DataType);
}

macro_rules! function {
    ($n:expr; $($A:ident $a:ident $i:expr),*) => {
        impl<F, R, $($A,)*> Function<($($A,)*)> for F
        where
            F: Fn($($A),*) -> R + Send + Sync + 'static,
            R: Serialize + Typed,
            $($A: DeserializeOwned + Typed,)*
        {
            const ARITY: usize = $n;

            #[allow(unused_variables)]
            #[inline]
            fn invoke(&self, context: &Context, args: &HostArgs) -> Result<Value, Fail> {
                $(let $a = argument::<$A>(context, args, $i)?;)*
                answer(context, &(self)($($a),*))
            }

            #[cfg(feature = "types")]
            #[allow(unused_variables)]
            fn signature(types: &mut Types) -> (Vec<DataType>, DataType) {
                (vec![$(types.define::<$A>()),*], types.define::<R>())
            }
        }

        impl<F, R, $($A,)*> Task<($($A,)*)> for F
        where
            F: Fn($($A),*) -> R + Send + Sync + 'static,
            R: Serialize + Typed,
            $($A: DeserializeOwned + Typed + Send + 'static,)*
        {
            const ARITY: usize = $n;

            #[allow(unused_variables)]
            fn prepare(self: Arc<Self>, context: &Context, args: &HostArgs) -> Result<Work, Fail> {
                $(let $a = argument::<$A>(context, args, $i)?;)*
                Ok(Box::new(move || answer_owned(&(self)($($a),*))))
            }

            #[cfg(feature = "types")]
            #[allow(unused_variables)]
            fn signature(types: &mut Types) -> (Vec<DataType>, DataType) {
                (vec![$(types.define::<$A>()),*], types.define::<R>())
            }
        }
    };
}

function!(0;);
function!(1; A1 a1 1);
function!(2; A1 a1 1, A2 a2 2);
function!(3; A1 a1 1, A2 a2 2, A3 a3 3);
function!(4; A1 a1 1, A2 a2 2, A3 a3 3, A4 a4 4);
function!(5; A1 a1 1, A2 a2 2, A3 a3 3, A4 a4 4, A5 a5 5);
function!(6; A1 a1 1, A2 a2 2, A3 a3 3, A4 a4 4, A5 a5 5, A6 a6 6);
function!(7; A1 a1 1, A2 a2 2, A3 a3 3, A4 a4 4, A5 a5 5, A6 a6 6, A7 a7 7);
function!(8; A1 a1 1, A2 a2 2, A3 a3 3, A4 a4 4, A5 a5 5, A6 a6 6, A7 a7 7, A8 a8 8);

#[inline]
fn argument<A: DeserializeOwned>(
    context: &Context,
    args: &HostArgs,
    position: u32,
) -> Result<A, Fail> {
    private::argument(context, args, position, |js| A::deserialize(js))
}

fn outcome<V>(result: Result<V, marshal::OutError>) -> Result<V, Fail> {
    result.map_err(private::fail)
}

#[inline]
fn answer<R: Serialize + ?Sized>(context: &Context, value: &R) -> Result<Value, Fail> {
    outcome(marshal::to_js(context, value, true))
}

/// A task's result, made on its worker.
fn answer_owned<R: Serialize + ?Sized>(value: &R) -> Result<marshal::Owned, Failure> {
    worker_outcome(marshal::owned(value, true))
}

fn worker_outcome(
    result: Result<marshal::Owned, marshal::OutError>,
) -> Result<marshal::Owned, Failure> {
    outcome(result).map_err(|e| match e {
        Fail::Failure(failure) => failure,
        Fail::Thrown(_) => Failure::new("the result cannot be serialized"),
    })
}

/// What `#[skathi::module]` expands to; not an API.
pub mod private {
    pub use super::super::marshal::{
        answer_compound, encode_bytes, encode_serde, missing, owned_compound, read_struct,
        unit_variant, Element, Error, Fields, FromJs, IntoJs, Js, Keys, Open, OutError, Owned,
        Pick, ReadDirect, ReadSerde, Slice, Str, Tree, WriteDirect, WriteSerde,
    };
    pub use super::{Fail, Work};
    pub use crate::js::{Context, HostArgs, Value};

    use super::{marshal, Failure};

    /// Argument `position` (from 1), read by `read`; a wrong one is a TypeError that names it.
    #[inline]
    pub fn argument<T>(
        context: &Context,
        args: &HostArgs,
        position: u32,
        read: impl FnOnce(Js<'_, '_>) -> Result<T, Error>,
    ) -> Result<T, Fail> {
        let walk = marshal::Walk::new(context);
        read(walk.arg(position - 1, args.arg(position - 1))).map_err(|e| match e {
            Error::Thrown(v) => Fail::Thrown(v),
            Error::NotJson(m) => Fail::Thrown(context.error(1, &m)),
            e => Fail::Failure(Failure::type_error(format!("argument {position}: {e}"))),
        })
    }

    pub fn fail(e: OutError) -> Fail {
        match e {
            OutError::Err(json) => Fail::Failure(Failure::from_err(json)),
            OutError::Thrown(v) => Fail::Thrown(v),
            e => Fail::Failure(Failure::new(format!(
                "the result cannot be serialized: {e}"
            ))),
        }
    }

    /// A task's work: the call on a worker, its result encoded there.
    pub fn work(run: impl FnOnce() -> Result<Owned, OutError> + Send + 'static) -> Work {
        Box::new(move || super::worker_outcome(run()))
    }
}

/// An event's data, made on the thread that emits it.
pub(crate) fn event_data<T: Serialize + ?Sized>(data: &T) -> Option<marshal::Owned> {
    marshal::owned(data, false).ok()
}

enum Invoke {
    Function(Box<dyn Fn(&Context, &HostArgs) -> Result<Value, Fail> + Send + Sync>),
    Task(Box<dyn Fn(&Context, &HostArgs) -> Result<Work, Fail> + Send + Sync>),
}

pub(crate) struct Entry {
    pub(crate) name: String,
    #[cfg(feature = "types")]
    pub(crate) task: bool,
    pub(crate) arity: usize,
    pub(crate) params: Vec<String>,
    invoke: Invoke,
    #[cfg(feature = "types")]
    pub(crate) signature: Signature,
}

#[cfg(feature = "types")]
type Signature = fn(&mut Types) -> (Vec<DataType>, DataType);
#[cfg(not(feature = "types"))]
type Signature = ();

pub(crate) struct Event {
    pub(crate) name: String,
    #[cfg(feature = "types")]
    pub(crate) ty: fn(&mut Types) -> DataType,
}

/// What `Builder::module` takes: a type with `#[skathi::module]`, or a closure that fills the
/// [`Module`] (`|m: &mut Module| …`).
pub trait IntoModule {
    fn register(self, module: &mut Module);
}

impl<F: FnOnce(&mut Module)> IntoModule for F {
    fn register(self, module: &mut Module) {
        self(module)
    }
}

/// One `native:<name>` module, filled in by `Builder::module`.
pub struct Module {
    pub(crate) name: String,
    pub(crate) functions: Vec<Entry>,
    pub(crate) events: Vec<Event>,
    problem: Option<String>,
}

const RESERVED: &[&str] = &["available", "on", "off", "default", "then", "__esModule"];

/// Cannot be imported by name or declared as a function.
const KEYWORDS: &str = "await break case catch class const continue debugger delete do else enum \
    export extends false finally for function if implements import in instanceof interface let new \
    null package private protected public return static super switch this throw true try typeof var \
    void while with yield";

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

pub(crate) fn valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
}

#[cfg(feature = "types")]
fn event_type<T: Typed>(types: &mut Types) -> DataType {
    types.define::<T>()
}

impl Module {
    pub(crate) fn new(name: String) -> Self {
        let problem = (!valid_module_name(&name)).then(|| {
            format!("native module name {name:?}: use letters, digits, '_', '-', '.' or '/'")
        });
        Self {
            name,
            functions: Vec::new(),
            events: Vec::new(),
            problem,
        }
    }

    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    fn claim(&mut self, name: &str) -> bool {
        if self.problem.is_some() {
            return false;
        }
        let taken = self.functions.iter().any(|f| f.name == name);
        if !is_identifier(name)
            || RESERVED.contains(&name)
            || KEYWORDS.split_whitespace().any(|k| k == name)
            || taken
        {
            self.problem = Some(format!(
                "native:{}: {name:?} is {}",
                self.name,
                if taken {
                    "registered twice"
                } else {
                    "not a usable export name: use a JS identifier that is not a keyword, available, on, off, default or then"
                }
            ));
            return false;
        }
        true
    }

    #[cfg_attr(not(feature = "types"), allow(unused_variables))]
    fn add(
        &mut self,
        name: &str,
        arity: usize,
        invoke: Invoke,
        signature: Signature,
    ) -> Declared<'_> {
        if !self.claim(name) {
            return Declared { entry: None };
        }
        self.functions.push(Entry {
            name: name.to_owned(),
            #[cfg(feature = "types")]
            task: matches!(invoke, Invoke::Task(_)),
            arity,
            params: Vec::new(),
            invoke,
            #[cfg(feature = "types")]
            signature,
        });
        Declared {
            entry: self.functions.last_mut(),
        }
    }

    /// A synchronous export: runs on the UI thread inside the call and returns its value. Keep it
    /// short; anything slow is a [`Self::task`].
    pub fn function<Args, H: Function<Args>>(&mut self, name: &str, handler: H) -> Declared<'_> {
        let signature: Signature;
        #[cfg(feature = "types")]
        {
            signature = H::signature;
        }
        #[cfg(not(feature = "types"))]
        {
            signature = ();
        }
        self.add(
            name,
            H::ARITY,
            Invoke::Function(Box::new(move |context, args| handler.invoke(context, args))),
            signature,
        )
    }

    /// An export that runs on a worker thread and returns a `Promise` in the app.
    pub fn task<Args, H: Task<Args>>(&mut self, name: &str, handler: H) -> Declared<'_> {
        let signature: Signature;
        #[cfg(feature = "types")]
        {
            signature = H::signature;
        }
        #[cfg(not(feature = "types"))]
        {
            signature = ();
        }
        let handler = Arc::new(handler);
        self.add(
            name,
            H::ARITY,
            Invoke::Task(Box::new(move |context, args| {
                handler.clone().prepare(context, args)
            })),
            signature,
        )
    }

    /// A function `#[skathi::module]` generated: `types` gives the signature, `invoke` runs it.
    #[doc(hidden)]
    pub fn __function<Args, H: Function<Args>>(
        &mut self,
        name: &str,
        types: H,
        invoke: impl Fn(&Context, &HostArgs) -> Result<Value, Fail> + Send + Sync + 'static,
    ) -> Declared<'_> {
        drop(types);
        let signature: Signature;
        #[cfg(feature = "types")]
        {
            signature = H::signature;
        }
        #[cfg(not(feature = "types"))]
        {
            signature = ();
        }
        self.add(
            name,
            H::ARITY,
            Invoke::Function(Box::new(invoke)),
            signature,
        )
    }

    /// A task `#[skathi::module]` generated: `types` gives the signature, `prepare` reads the
    /// arguments and returns the work.
    #[doc(hidden)]
    pub fn __task<Args, H: Task<Args>>(
        &mut self,
        name: &str,
        types: H,
        prepare: impl Fn(&Context, &HostArgs) -> Result<Work, Fail> + Send + Sync + 'static,
    ) -> Declared<'_> {
        drop(types);
        let signature: Signature;
        #[cfg(feature = "types")]
        {
            signature = H::signature;
        }
        #[cfg(not(feature = "types"))]
        {
            signature = ();
        }
        self.add(name, H::ARITY, Invoke::Task(Box::new(prepare)), signature)
    }

    /// Declares an event `Handle::emit_to` may send with a `T` payload; the app subscribes with `on`.
    pub fn event<T: Serialize + Typed + 'static>(&mut self, name: &str) -> &mut Self {
        if self.problem.is_none() && self.events.iter().any(|e| e.name == name) {
            self.problem = Some(format!(
                "native:{}: event {name:?} is declared twice",
                self.name
            ));
        }
        if name.is_empty() {
            self.problem = Some(format!("native:{}: an event needs a name", self.name));
        }
        self.events.push(Event {
            name: name.to_owned(),
            #[cfg(feature = "types")]
            ty: event_type::<T>,
        });
        self
    }

    /// The declared event called `event`, as the module keeps it.
    pub(crate) fn declared(&'static self, event: &str) -> Option<&'static str> {
        self.events
            .iter()
            .find(|e| e.name == event)
            .map(|e| e.name.as_str())
    }
}

/// A registered function or task; name its parameters for the generated declarations.
pub struct Declared<'a> {
    entry: Option<&'a mut Entry>,
}

impl Declared<'_> {
    /// Parameter names for the `.d.ts` (`arg1`, `arg2`, … otherwise).
    pub fn params<I, S>(self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let Some(entry) = self.entry {
            entry.params = names.into_iter().map(Into::into).collect();
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".into())
}

fn labeled(module: &Module, entry: &Entry, failure: Failure) -> Failure {
    if failure.kind == 1 {
        Failure {
            message: format!("native:{}.{}: {}", module.name, entry.name, failure.message),
            ..failure
        }
    } else {
        failure
    }
}

fn panicked(module: &Module, entry: &Entry, payload: &(dyn std::any::Any + Send)) -> Failure {
    Failure::new(format!(
        "native:{}.{} panicked: {}",
        module.name,
        entry.name,
        panic_message(payload)
    ))
}

/// Runs a call's part on the JS thread: a panic becomes a [`Failure`], a type error is labeled.
fn guarded<T>(
    module: &Module,
    entry: &Entry,
    run: impl FnOnce() -> Result<T, Fail>,
) -> Result<T, Fail> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(Fail::Failure(failure))) => Err(Fail::Failure(labeled(module, entry, failure))),
        Ok(Err(thrown)) => Err(thrown),
        Err(payload) => Err(Fail::Failure(panicked(module, entry, payload.as_ref()))),
    }
}

fn thrown(context: &Context, fail: Fail) -> js::Error {
    js::Error::Exception(match fail {
        Fail::Thrown(value) => value,
        Fail::Failure(failure) => error_value(context, &failure),
    })
}

/// Runs one task's work; see `Builder::task_executor`.
pub type Job = Box<dyn FnOnce() + Send + 'static>;
pub(crate) type Executor = Arc<dyn Fn(Job) + Send + Sync + 'static>;

const POOL_THREADS: usize = 4;
static POOL: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

fn pool_run(job: Job) {
    let sender = POOL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..POOL_THREADS {
            let rx = rx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("skathi-task-{i}"))
                .spawn(move || loop {
                    let job = match rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                });
        }
        Mutex::new(tx)
    });
    if let Ok(sender) = sender.lock() {
        // A send fails only with every worker gone; the job's drop rejects its promise.
        let _ = sender.send(job);
    }
}

enum Item {
    Task {
        token: u32,
        result: Result<marshal::Owned, Failure>,
    },
    Event {
        module: &'static str,
        event: &'static str,
        data: marshal::Owned,
    },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static QUEUED: AtomicUsize = AtomicUsize::new(0);
static TASKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);

pub(super) fn queued() -> bool {
    QUEUED.load(Ordering::Acquire) != 0
}

// Only the first item wakes the JS thread: it drains until the queue is empty, and wakes itself
// again when it stops early.
fn push(item: Item) {
    let first = match QUEUE.lock() {
        Ok(mut queue) => {
            queue.push_back(item);
            QUEUED.fetch_add(1, Ordering::AcqRel) == 0
        }
        Err(_) => false,
    };
    if first {
        super::wake();
    }
}

/// Settles one task's promise; dropped without settling (an executor that discards the job), it rejects.
struct TaskReply {
    token: u32,
    settled: bool,
}

impl TaskReply {
    fn settle(mut self, result: Result<marshal::Owned, Failure>) {
        self.settled = true;
        push(Item::Task {
            token: self.token,
            result,
        });
    }
}

impl Drop for TaskReply {
    fn drop(&mut self) {
        if !self.settled {
            push(Item::Task {
                token: self.token,
                result: Err(Failure::new("the task was dropped before it ran")),
            });
        }
    }
}

pub(crate) fn emit(module: &'static str, event: &'static str, data: marshal::Owned) {
    push(Item::Event {
        module,
        event,
        data,
    });
}

struct Pending {
    resolve: Value,
    reject: Value,
}

#[derive(Default)]
struct State {
    executor: Option<Executor>,
    modules: HashMap<String, Value>,
    listeners: HashMap<(&'static str, &'static str), Vec<Value>>,
    pending: HashMap<u32, Pending>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn type_error(context: &Context, message: &str) -> js::Error {
    js::Error::Exception(context.error(1, message))
}

fn parse(context: &Context, json: &str) -> js::Result<Value> {
    marshal::parse(context, json).map_err(|e| match e {
        marshal::Error::Thrown(v) => js::Error::Exception(v),
        _ => js::Error::BadArg,
    })
}

fn error_value(context: &Context, failure: &Failure) -> Value {
    let error = context.error(failure.kind, &failure.message);
    if let Some(data) = failure.data.as_deref() {
        if let Ok(value) = parse(context, data) {
            let _ = context.set(error, &context.atom("data"), value);
        }
    }
    error
}

fn sync_function(
    context: &Context,
    module: &'static Module,
    entry: &'static Entry,
    invoke: &'static (dyn Fn(&Context, &HostArgs) -> Result<Value, Fail> + Send + Sync),
) -> js::Result<Value> {
    js::function(
        context,
        &entry.name,
        entry.arity as u32,
        move |context, args| {
            guarded(module, entry, || invoke(context, &args)).map_err(|f| thrown(context, f))
        },
    )
}

fn task_function(
    context: &Context,
    module: &'static Module,
    entry: &'static Entry,
    prepare: &'static (dyn Fn(&Context, &HostArgs) -> Result<Work, Fail> + Send + Sync),
) -> js::Result<Value> {
    js::function(
        context,
        &entry.name,
        entry.arity as u32,
        move |context, args| {
            let work = match guarded(module, entry, || prepare(context, &args)) {
                Err(Fail::Thrown(e)) => return Err(js::Error::Exception(e)),
                work => work,
            };
            let (promise, resolve, reject) = context.promise()?;
            let work = match work {
                Ok(work) => work,
                Err(Fail::Thrown(e)) => return Err(js::Error::Exception(e)),
                // Arguments of the wrong type reject, as when they were read on the worker.
                Err(Fail::Failure(failure)) => {
                    context.call(
                        reject,
                        Value::undefined(),
                        &[error_value(context, &failure)],
                    )?;
                    return Ok(promise);
                }
            };
            context.protect(resolve);
            context.protect(reject);
            let token = loop {
                let t = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
                if t != 0 {
                    break t;
                }
            };
            let executor = STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.pending.insert(token, Pending { resolve, reject });
                s.executor.clone()
            });
            TASKS.fetch_add(1, Ordering::AcqRel);
            let reply = TaskReply {
                token,
                settled: false,
            };
            let job: Job = Box::new(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                    .unwrap_or_else(|payload| Err(panicked(module, entry, payload.as_ref())));
                reply.settle(result)
            });
            match executor {
                Some(executor) => {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| executor(job)))
                        .is_err()
                    {
                        eprintln!("[embed] the task executor panicked");
                    }
                }
                None => pool_run(job),
            }
            Ok(promise)
        },
    )
}

fn event_name(
    context: &Context,
    module: &'static Module,
    value: Value,
) -> js::Result<&'static str> {
    let name = (context.kind(value) == 5)
        .then(|| context.read_string(value).ok())
        .flatten();
    match name.and_then(|name| module.declared(&name)) {
        Some(name) => Ok(name),
        _ => {
            let declared: Vec<String> = module
                .events
                .iter()
                .map(|e| format!("{:?}", e.name))
                .collect();
            Err(type_error(
                context,
                &format!(
                    "native:{} has no such event (it has {})",
                    module.name,
                    if declared.is_empty() {
                        "none".into()
                    } else {
                        declared.join(", ")
                    }
                ),
            ))
        }
    }
}

fn subscribe(
    context: &Context,
    module: &'static Module,
    off: Value,
    args: HostArgs,
) -> js::Result<Value> {
    let event = event_name(context, module, args.arg(0))?;
    let callback = args.arg(1);
    if context.kind(callback) != 9 {
        return Err(type_error(
            context,
            &format!("native:{}.on({event:?}, …) needs a function", module.name),
        ));
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let list = s.listeners.entry((&module.name, event)).or_default();
        if !list.contains(&callback) {
            context.protect(callback);
            list.push(callback);
        }
    });
    let bind = context.get(off, &context.atom("bind"))?;
    context.call(bind, off, &[Value::undefined(), args.arg(0), callback])
}

fn unsubscribe(context: &Context, module: &'static Module, args: HostArgs) -> js::Result<Value> {
    let event = event_name(context, module, args.arg(0))?;
    let callback = args.arg(1);
    let removed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let list = s.listeners.get_mut(&(module.name.as_str(), event))?;
        let at = list.iter().position(|c| *c == callback)?;
        Some(list.remove(at))
    });
    if let Some(callback) = removed {
        context.unprotect(callback);
    }
    Ok(Value::undefined())
}

fn build(context: &Context, module: &'static Module) -> js::Result<Value> {
    let object = context.object();
    context.set(object, &context.atom("available"), Value::boolean(true))?;
    for entry in &module.functions {
        let f = match &entry.invoke {
            Invoke::Task(prepare) => task_function(context, module, entry, &**prepare)?,
            Invoke::Function(invoke) => sync_function(context, module, entry, &**invoke)?,
        };
        context.set(object, &context.atom(&entry.name), f)?;
    }
    let off = js::function(context, "off", 2, move |context, args| {
        unsubscribe(context, module, args)
    })?;
    context.protect(off);
    let on = js::function(context, "on", 2, move |context, args| {
        subscribe(context, module, off, args)
    })?;
    context.set(object, &context.atom("on"), on)?;
    context.set(object, &context.atom("off"), off)?;
    Ok(object)
}

fn deliver_event(context: &Context, module: &str, event: &'static str, data: marshal::Owned) {
    // The listeners are copied, since one may unsubscribe itself or another while it runs, and
    // kept alive: on this stack, which the collector scans, or protected when there are more.
    let mut few = [Value::undefined(); 4];
    let mut more = Vec::new();
    let count = STATE.with(|s| {
        let s = s.borrow();
        let list = s
            .listeners
            .get(&(module, event))
            .map_or(&[][..], |l| &l[..]);
        match list.len() {
            n @ ..=4 => few[..n].copy_from_slice(list),
            _ => more.extend_from_slice(list),
        }
        list.len()
    });
    if count == 0 {
        return;
    }
    let Ok(data) = data.to_js(context) else {
        return;
    };
    let listeners = if more.is_empty() {
        &few[..count]
    } else {
        more.iter().for_each(|&c| context.protect(c));
        &more[..]
    };
    for &callback in listeners {
        if let Err(js::Error::Exception(e)) = context.call(callback, Value::undefined(), &[data]) {
            JsRuntime::with(|rt| {
                let err = rt.describe(e);
                eprintln!(
                    "[native:{module}] a handler for {event:?} threw: {}\n{}",
                    err.header, err.stack
                );
            });
        }
    }
    more.iter().for_each(|&c| context.unprotect(c));
}

fn settle_task(context: &Context, token: u32, result: Result<marshal::Owned, Failure>) {
    TASKS.fetch_sub(1, Ordering::AcqRel);
    let Some(pending) = STATE.with(|s| s.borrow_mut().pending.remove(&token)) else {
        return;
    };
    let (callback, value) = match result.map(|data| data.to_js(context)) {
        Ok(Ok(value)) => (pending.resolve, value),
        Ok(Err(marshal::Error::Thrown(e))) => (pending.reject, e),
        Ok(Err(other)) => (pending.reject, context.error(0, &other.to_string())),
        Err(failure) => (pending.reject, error_value(context, &failure)),
    };
    let _ = context.call(callback, Value::undefined(), &[value]);
    context.unprotect(pending.resolve);
    context.unprotect(pending.reject);
}

// Queued items count as in flight: the pump keeps draining them within its budget instead of
// leaving the rest for a wake that already came.
unsafe fn src_inflight(g: *mut c_void) -> bool {
    super::is_ours(g) && (TASKS.load(Ordering::Acquire) != 0 || QUEUED.load(Ordering::Acquire) != 0)
}

unsafe fn src_ready(g: *mut c_void) -> bool {
    super::is_ours(g) && QUEUED.load(Ordering::Acquire) != 0
}

const DRAIN_CAP: usize = 16;

unsafe fn src_drain(g: *mut c_void) -> usize {
    if !unsafe { src_ready(g) } {
        return 0;
    }
    drain()
}

fn drain() -> usize {
    let budget = Duration::from_micros(sk_runtime::redjs::web::pump_budget_us() as u64);
    let start = std::time::Instant::now();
    let mut n = 0;
    let mut batch = Vec::with_capacity(DRAIN_CAP);
    while n < DRAIN_CAP {
        if let Ok(mut queue) = QUEUE.lock() {
            let take = queue.len().min(DRAIN_CAP - n);
            batch.extend(queue.drain(..take));
        }
        if batch.is_empty() {
            break;
        }
        QUEUED.fetch_sub(batch.len(), Ordering::AcqRel);
        n += batch.len();
        JsRuntime::with(|rt| {
            let context = rt.ctx();
            for item in batch.drain(..) {
                match item {
                    Item::Task { token, result } => settle_task(context, token, result),
                    Item::Event {
                        module,
                        event,
                        data,
                    } => deliver_event(context, module, event, data),
                }
            }
        });
        if start.elapsed() >= budget {
            break;
        }
    }
    if QUEUED.load(Ordering::Acquire) != 0 {
        super::wake();
    }
    n
}

/// Page reload: the old page's subscriptions, unsettled promises and undelivered events are dropped.
fn reset(context: &Context) {
    if let Ok(mut queue) = QUEUE.lock() {
        let before = queue.len();
        queue.retain(|item| matches!(item, Item::Task { .. }));
        QUEUED.fetch_sub(before - queue.len(), Ordering::AcqRel);
    }
    let (listeners, pending) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        (
            std::mem::take(&mut s.listeners),
            std::mem::take(&mut s.pending),
        )
    });
    for callback in listeners.into_values().flatten() {
        context.unprotect(callback);
    }
    for p in pending.into_values() {
        context.unprotect(p.resolve);
        context.unprotect(p.reject);
    }
}

/// Builds every module's object before app code and installs `__sk_native_module(name)`, which
/// the module loader asks for `native:<name>`.
pub(crate) fn install(
    context: &Context,
    modules: &'static [Module],
    executor: Option<Executor>,
) -> js::Result<()> {
    marshal::install(context)?;
    STATE.with(|s| s.borrow_mut().executor = executor);
    for module in modules {
        let object = build(context, module)?;
        context.protect(object);
        STATE.with(|s| s.borrow_mut().modules.insert(module.name.clone(), object));
    }
    let lookup = js::function(context, "__sk_native_module", 1, |context, args| {
        let value = args.arg(0);
        let name = (context.kind(value) == 5)
            .then(|| context.read_string(value).ok())
            .flatten()
            .unwrap_or_default();
        Ok(STATE
            .with(|s| s.borrow().modules.get(&name).copied())
            .unwrap_or_else(Value::undefined))
    })?;
    context.global_set("__sk_native_module", lookup)?;
    js::on_reload(reset);
    sk_runtime::redjs::web::register_pump_source(sk_runtime::redjs::web::PumpSource {
        inflight: src_inflight,
        ready: src_ready,
        drain: src_drain,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    #[cfg_attr(feature = "types", derive(specta::Type))]
    struct Point {
        x: i32,
        y: i32,
    }

    #[test]
    fn module_names_and_exports_are_checked() {
        let mut m = Module::new("notes".into());
        m.function("get", |id: u32| id);
        assert!(m.problem().is_none());
        m.function("get", |id: u32| id);
        assert!(m.problem().is_some());
        let mut m = Module::new("notes".into());
        m.task("on", || 1);
        assert!(m.problem().is_some());
        let mut m = Module::new("notes".into());
        m.function("delete", |id: u32| id);
        assert!(m.problem().is_some());
        assert!(Module::new("bad name".into()).problem().is_some());
    }

    #[test]
    fn functions_tasks_and_events_reach_js_as_values_errors_and_promises() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let mut m = Module::new("demo".into());
                m.function("flip", |p: Point| Point { x: p.y, y: p.x });
                m.function("fail", || -> Result<u8, String> { Err("refused".into()) });
                m.function("boom", || -> u8 { panic!("kaboom") });
                m.task("slow", |n: u32| -> Result<u32, String> {
                    if n == 0 {
                        Err("zero".into())
                    } else {
                        Ok(n * 2)
                    }
                });
                m.task("crash", || -> u8 { panic!("worker kaboom") });
                m.task("halve", |p: Point| p.x / 2);
                m.function("add", |a: i32, b: Option<i32>| a + b.unwrap_or(10));
                m.function("found", |id: u32| -> Result<Point, String> {
                    if id == 1 {
                        Ok(Point { x: 1, y: 1 })
                    } else {
                        Err(format!("no point {id}"))
                    }
                });
                m.function("structured", || -> Result<(), Point> {
                    Err(Point { x: 0, y: 7 })
                });
                #[derive(serde::Serialize)]
                #[cfg_attr(feature = "types", derive(specta::Type))]
                enum Status {
                    Ok(u8),
                }
                m.function("status", || Status::Ok(1));
                m.function("nativePath", |f: crate::embed::HostFile| {
                    f.0.to_string_lossy().into_owned()
                });
                m.function("shout", |s: marshal::Str, n: u8| {
                    // SAFETY: used inside the call.
                    unsafe { s.get() }.to_uppercase().repeat(n as usize)
                });
                m.function("sum", |b: marshal::Slice<u8>| {
                    // SAFETY: used inside the call.
                    unsafe { b.get() }.iter().map(|&b| b as u32).sum::<u32>()
                });
                m.function("reversed", |mut b: crate::embed::Bytes| {
                    b.reverse();
                    b
                });
                m.task("doubled", |b: crate::embed::Bytes| {
                    crate::embed::Bytes(b.iter().map(|b| b * 2).collect())
                });
                m.event::<Point>("moved");
                let modules: &'static [Module] = Box::leak(vec![m].into_boxed_slice());
                install(context, modules, None).unwrap();
                let eval = |src: &str| -> String {
                    let v = context.eval(src).unwrap();
                    context.read_string(v).unwrap()
                };
                eval(
                    "globalThis.__nm = __sk_native_module('demo');
                     globalThis.__seen = [];
                     globalThis.__off = __nm.on('moved', (p) => __seen.push(p.x + p.y));
                     ''",
                );
                let disk = std::env::current_exe().unwrap();
                let f = unsafe {
                    sk_runtime::redjs::webapi::file::file_from_path(context.raw(), &disk, "")
                        .unwrap()
                };
                context
                    .set(context.global(), &context.atom("__moduleFile"), Value(f))
                    .unwrap();
                assert_eq!(
                    eval("__nm.nativePath(__moduleFile)"),
                    disk.to_string_lossy()
                );
                assert_eq!(
                    eval("try { __nm.nativePath(new File([], 'made-up')); } catch(e) { 'denied' }"),
                    "denied"
                );
                assert_eq!(
                    eval("try { __nm.nativePath({path:'/made-up'}); } catch(e) { 'denied' }"),
                    "denied"
                );
                let settle = || {
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    while TASKS.load(Ordering::Acquire) != 0 || QUEUED.load(Ordering::Acquire) != 0
                    {
                        assert!(std::time::Instant::now() < deadline, "tasks did not settle");
                        drain();
                        context.drain_microtasks();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    context.drain_microtasks();
                };
                assert_eq!(
                    eval("JSON.stringify(__nm.flip({ x: 1, y: 2 }))"),
                    r#"{"x":2,"y":1}"#
                );
                assert_eq!(
                    eval("try { __nm.fail(); 'no' } catch (e) { e instanceof Error ? e.message : 'not an Error' }"),
                    "refused"
                );
                assert_eq!(
                    eval("try { __nm.boom(); 'no' } catch (e) { String(e.message.includes('kaboom')) }"),
                    "true"
                );
                assert_eq!(
                    eval("try { __nm.flip('x'); 'no' } catch (e) { e.name }"),
                    "TypeError"
                );
                assert_eq!(
                    eval("try { __nm.on('nope', () => {}); 'no' } catch (e) { e.name }"),
                    "TypeError"
                );
                assert_eq!(eval("typeof __sk_native_module('missing')"), "undefined");
                assert_eq!(
                    eval("[__nm.add(2, 3), __nm.add(2), __nm.add(2, 3, 99)].join()"),
                    "5,12,5"
                );
                assert_eq!(
                    eval("JSON.stringify([__nm.found(1), __nm.status()])"),
                    r#"[{"x":1,"y":1},{"Ok":1}]"#
                );
                assert_eq!(
                    eval("[__nm.shout('ab', 2), __nm.shout('éß中', 1)].join()"),
                    "ABAB,ÉSS中"
                );
                assert_eq!(
                    eval("try { __nm.shout(1, 1); 'no' } catch (e) { e.message }"),
                    "native:demo.shout: argument 1: invalid type: integer `1`, expected a string"
                );
                assert_eq!(
                    eval("try { __nm.found(2); 'no' } catch (e) { e.message + '|' + e.name }"),
                    "no point 2|Error"
                );
                assert_eq!(
                    eval("try { __nm.structured(); 'no' } catch (e) { JSON.stringify(e.data) }"),
                    r#"{"x":0,"y":7}"#
                );
                assert_eq!(
                    eval("[__nm.sum(new Uint8Array([1, 2, 3])), __nm.sum([4, 5]), __nm.sum(new Uint8Array([9, 9]).buffer)].join()"),
                    "6,9,18"
                );
                assert_eq!(
                    eval("const r = __nm.reversed(new Uint8Array([1, 2, 3])); r.constructor.name + ':' + r.join()"),
                    "Uint8Array:3,2,1"
                );
                eval(
                    "globalThis.__settled = {};
                     __nm.doubled(new Uint8Array([1, 2])).then((v) => __settled.bytes = v.constructor.name + ':' + v.join());
                     __nm.slow(21).then((v) => __settled.ok = v);
                     __nm.slow(0).catch((e) => __settled.err = e.message);
                     __nm.crash().catch((e) => __settled.crash = e.message.includes('worker kaboom'));
                     __nm.halve('x').catch((e) => __settled.bad = e.name + ': ' + e.message);
                     ''",
                );
                emit("demo", "moved", event_data(&Point { x: 3, y: 4 }).unwrap());
                settle();
                assert_eq!(
                    eval("[__settled.ok, __settled.err, __settled.crash, __settled.bytes].join()"),
                    "42,zero,true,Uint8Array:2,4"
                );
                assert_eq!(
                    eval("__settled.bad"),
                    "TypeError: native:demo.halve: argument 1: invalid type: string \"x\", expected struct Point"
                );
                assert_eq!(eval("JSON.stringify(__seen)"), "[7]");
                eval("__off(); ''");
                emit("demo", "moved", event_data(&Point { x: 1, y: 1 }).unwrap());
                settle();
                assert_eq!(eval("JSON.stringify(__seen)"), "[7]");
                emit("demo", "moved", event_data(&Point { x: 2, y: 2 }).unwrap());
                reset(context);
                eval("__nm.on('moved', (p) => __seen.push(p.x + p.y)); ''");
                emit("demo", "moved", event_data(&Point { x: 5, y: 5 }).unwrap());
                settle();
                assert_eq!(eval("JSON.stringify(__seen)"), "[7,10]");
            })
        });
    }
}
