//! Native modules: typed functions, tasks and events an embedder gives the app as `native:<name>`.
//!
//! Arguments and results cross as JSON (serde on this side, `JSON.parse`/`JSON.stringify` on the
//! VM thread); the app sees ordinary values. A function runs inside the JS call; a task runs on a
//! worker and settles a promise in a later frame.

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

/// A Rust closure the app can call: `Fn(A1, ..., An) -> R` for up to eight arguments, each
/// `Deserialize`, with `R: Serialize`. An `Err(e)` result throws (or rejects) with `e`.
pub trait Function<Args>: Send + Sync + 'static {
    #[doc(hidden)]
    const ARITY: usize;
    #[doc(hidden)]
    fn invoke(&self, args: Vec<Json>) -> Result<String, Failure>;
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

            #[allow(unused_mut, unused_variables)]
            fn invoke(&self, mut args: Vec<Json>) -> Result<String, Failure> {
                args.resize($n, Json::Null);
                let mut args = args.into_iter();
                $(let $a = argument::<$A>(args.next().unwrap_or(Json::Null), $i)?;)*
                answer(&(self)($($a),*))
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

fn argument<A: DeserializeOwned>(value: Json, position: usize) -> Result<A, Failure> {
    serde_json::from_value(value)
        .map_err(|e| Failure::type_error(format!("argument {position}: {e}")))
}

fn answer<R: Serialize + ?Sized>(value: &R) -> Result<String, Failure> {
    match value.serialize(ResultProbe) {
        Ok(Probed::Ok(json)) => Ok(json),
        Ok(Probed::Err(json)) => Err(Failure::from_err(json)),
        Err(_) => serde_json::to_string(value)
            .map_err(|e| Failure::new(format!("the result cannot be serialized: {e}"))),
    }
}

/// Recognizes a top-level `Result` by serde's own encoding of it; anything else fails at once.
struct ResultProbe;
enum Probed {
    Ok(String),
    Err(String),
}
#[derive(Debug)]
struct NotResult;
impl std::fmt::Display for NotResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a Result")
    }
}
impl std::error::Error for NotResult {}
impl serde::ser::Error for NotResult {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        NotResult
    }
}

macro_rules! not_result {
    ($($method:ident($($arg:ty),*);)*) => {
        $(fn $method(self, $(_: $arg),*) -> Result<Probed, NotResult> { Err(NotResult) })*
    };
}

impl serde::Serializer for ResultProbe {
    type Ok = Probed;
    type Error = NotResult;
    type SerializeSeq = serde::ser::Impossible<Probed, NotResult>;
    type SerializeTuple = serde::ser::Impossible<Probed, NotResult>;
    type SerializeTupleStruct = serde::ser::Impossible<Probed, NotResult>;
    type SerializeTupleVariant = serde::ser::Impossible<Probed, NotResult>;
    type SerializeMap = serde::ser::Impossible<Probed, NotResult>;
    type SerializeStruct = serde::ser::Impossible<Probed, NotResult>;
    type SerializeStructVariant = serde::ser::Impossible<Probed, NotResult>;

    not_result! {
        serialize_bool(bool);
        serialize_i8(i8);
        serialize_i16(i16);
        serialize_i32(i32);
        serialize_i64(i64);
        serialize_u8(u8);
        serialize_u16(u16);
        serialize_u32(u32);
        serialize_u64(u64);
        serialize_f32(f32);
        serialize_f64(f64);
        serialize_char(char);
        serialize_str(&str);
        serialize_bytes(&[u8]);
        serialize_none();
        serialize_unit();
        serialize_unit_struct(&'static str);
        serialize_unit_variant(&'static str, u32, &'static str);
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<Probed, NotResult> {
        Err(NotResult)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Probed, NotResult> {
        Err(NotResult)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Probed, NotResult> {
        if name != "Result" {
            return Err(NotResult);
        }
        let json = serde_json::to_string(value).map_err(|_| NotResult)?;
        match variant {
            "Ok" => Ok(Probed::Ok(json)),
            "Err" => Ok(Probed::Err(json)),
            _ => Err(NotResult),
        }
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, NotResult> {
        Err(NotResult)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, NotResult> {
        Err(NotResult)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, NotResult> {
        Err(NotResult)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, NotResult> {
        Err(NotResult)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, NotResult> {
        Err(NotResult)
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, NotResult> {
        Err(NotResult)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, NotResult> {
        Err(NotResult)
    }
}

type Invoke = Arc<dyn Fn(Vec<Json>) -> Result<String, Failure> + Send + Sync>;

pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) task: bool,
    pub(crate) arity: usize,
    pub(crate) params: Vec<String>,
    invoke: Invoke,
    #[cfg(feature = "types")]
    pub(crate) signature: fn(&mut Types) -> (Vec<DataType>, DataType),
}

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

    fn add<Args, H: Function<Args>>(&mut self, name: &str, handler: H, task: bool) -> Declared<'_> {
        if !self.claim(name) {
            return Declared { entry: None };
        }
        let handler = Arc::new(handler);
        self.functions.push(Entry {
            name: name.to_owned(),
            task,
            arity: H::ARITY,
            params: Vec::new(),
            invoke: Arc::new(move |args| handler.invoke(args)),
            #[cfg(feature = "types")]
            signature: H::signature,
        });
        Declared {
            entry: self.functions.last_mut(),
        }
    }

    /// A synchronous export: runs on the UI thread inside the call and returns its value. Keep it
    /// short; anything slow is a [`Self::task`].
    pub fn function<Args, H: Function<Args>>(&mut self, name: &str, handler: H) -> Declared<'_> {
        self.add(name, handler, false)
    }

    /// An export that runs on a worker thread and returns a `Promise` in the app.
    pub fn task<Args, H: Function<Args>>(&mut self, name: &str, handler: H) -> Declared<'_> {
        self.add(name, handler, true)
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

    pub(crate) fn declares(&self, event: &str) -> bool {
        self.events.iter().any(|e| e.name == event)
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

fn call(module: &Module, entry: &Entry, args: Vec<Json>) -> Result<String, Failure> {
    let label = format!("native:{}.{}", module.name, entry.name);
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (entry.invoke)(args))) {
        Ok(Err(failure)) if failure.kind == 1 => Err(Failure {
            message: format!("{label}: {}", failure.message),
            ..failure
        }),
        Ok(result) => result,
        Err(payload) => Err(Failure::new(format!(
            "{label} panicked: {}",
            panic_message(payload.as_ref())
        ))),
    }
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
        result: Result<String, Failure>,
    },
    Event {
        module: String,
        event: String,
        json: String,
    },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static QUEUED: AtomicUsize = AtomicUsize::new(0);
static TASKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);

fn push(item: Item) {
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push_back(item);
        QUEUED.fetch_add(1, Ordering::AcqRel);
    }
    super::wake();
}

/// Settles one task's promise; dropped without settling (an executor that discards the job), it rejects.
struct TaskReply {
    token: u32,
    settled: bool,
}

impl TaskReply {
    fn settle(mut self, result: Result<String, Failure>) {
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

pub(crate) fn emit(module: &str, event: &str, json: String) {
    push(Item::Event {
        module: module.to_owned(),
        event: event.to_owned(),
        json,
    });
}

struct Pending {
    resolve: Value,
    reject: Value,
}

#[derive(Default)]
struct State {
    json_parse: Option<Value>,
    json_stringify: Option<Value>,
    executor: Option<Executor>,
    modules: HashMap<String, Value>,
    listeners: HashMap<(String, String), Vec<Value>>,
    pending: HashMap<u32, Pending>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn type_error(context: &Context, message: &str) -> js::Error {
    js::Error::Exception(context.error(1, message))
}

fn json_fn(pick: fn(&State) -> Option<Value>) -> js::Result<Value> {
    STATE.with(|s| pick(&s.borrow())).ok_or(js::Error::BadArg)
}

fn parse(context: &Context, json: &str) -> js::Result<Value> {
    let parse = json_fn(|s| s.json_parse)?;
    context.call(parse, Value::undefined(), &[context.string(json)])
}

fn arguments(context: &Context, args: &HostArgs) -> js::Result<Vec<Json>> {
    let count = args.argc();
    let array = context.array(count)?;
    for i in 0..count {
        context.set_index(array, i, args.arg(i))?;
    }
    let stringify = json_fn(|s| s.json_stringify)?;
    let text = context.call(stringify, Value::undefined(), &[array])?;
    let text = context.read_string(text)?;
    serde_json::from_str(&text)
        .map_err(|e| type_error(context, &format!("the arguments are not JSON: {e}")))
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
) -> js::Result<Value> {
    js::function(
        context,
        &entry.name,
        entry.arity as u32,
        move |context, args| {
            let values = arguments(context, &args)?;
            match call(module, entry, values) {
                Ok(json) => parse(context, &json),
                Err(failure) => Err(js::Error::Exception(error_value(context, &failure))),
            }
        },
    )
}

fn task_function(
    context: &Context,
    module: &'static Module,
    entry: &'static Entry,
) -> js::Result<Value> {
    js::function(
        context,
        &entry.name,
        entry.arity as u32,
        move |context, args| {
            let values = arguments(context, &args)?;
            let (promise, resolve, reject) = context.promise()?;
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
            let job: Job = Box::new(move || reply.settle(call(module, entry, values)));
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

fn event_name(context: &Context, module: &Module, value: Value) -> js::Result<String> {
    let name = (context.kind(value) == 5)
        .then(|| context.read_string(value).ok())
        .flatten();
    match name {
        Some(name) if module.declares(&name) => Ok(name),
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
        let list = s.listeners.entry((module.name.clone(), event)).or_default();
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
        let list = s.listeners.get_mut(&(module.name.clone(), event))?;
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
        let f = if entry.task {
            task_function(context, module, entry)?
        } else {
            sync_function(context, module, entry)?
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

fn deliver_event(context: &Context, module: &str, event: &str, json: &str) {
    let key = (module.to_owned(), event.to_owned());
    let listeners = STATE.with(|s| s.borrow().listeners.get(&key).cloned().unwrap_or_default());
    if listeners.is_empty() {
        return;
    }
    let data = match parse(context, json) {
        Ok(data) => data,
        Err(_) => return,
    };
    context.protect(data);
    for callback in listeners {
        // A listener may unsubscribe itself, or another, while it runs.
        context.protect(callback);
        if let Err(js::Error::Exception(e)) = context.call(callback, Value::undefined(), &[data]) {
            JsRuntime::with(|rt| {
                let err = rt.describe(e);
                eprintln!(
                    "[native:{module}] a handler for {event:?} threw: {}\n{}",
                    err.header, err.stack
                );
            });
        }
        context.unprotect(callback);
    }
    context.unprotect(data);
}

fn settle_task(context: &Context, token: u32, result: Result<String, Failure>) {
    TASKS.fetch_sub(1, Ordering::AcqRel);
    let Some(pending) = STATE.with(|s| s.borrow_mut().pending.remove(&token)) else {
        return;
    };
    let (callback, value) = match result.map(|json| parse(context, &json)) {
        Ok(Ok(value)) => (pending.resolve, value),
        Ok(Err(js::Error::Exception(e))) => (pending.reject, e),
        Ok(Err(other)) => (pending.reject, context.error(0, &other.to_string())),
        Err(failure) => (pending.reject, error_value(context, &failure)),
    };
    let _ = context.call(callback, Value::undefined(), &[value]);
    context.unprotect(pending.resolve);
    context.unprotect(pending.reject);
}

unsafe fn src_inflight(g: *mut c_void) -> bool {
    super::is_ours(g) && TASKS.load(Ordering::Acquire) != 0
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
    while n < DRAIN_CAP {
        let item = QUEUE.lock().ok().and_then(|mut q| q.pop_front());
        let Some(item) = item else { break };
        QUEUED.fetch_sub(1, Ordering::AcqRel);
        JsRuntime::with(|rt| {
            let context = rt.ctx();
            match item {
                Item::Task { token, result } => settle_task(context, token, result),
                Item::Event {
                    module,
                    event,
                    json,
                } => deliver_event(context, &module, &event, &json),
            }
        });
        n += 1;
        if start.elapsed() >= budget {
            break;
        }
    }
    n
}

/// Page reload: the old page's subscriptions and unsettled promises are dropped.
fn reset(context: &Context) {
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
    let json = context.global_get("JSON")?;
    let parse = context.get(json, &context.atom("parse"))?;
    let stringify = context.get(json, &context.atom("stringify"))?;
    context.protect(parse);
    context.protect(stringify);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.json_parse = Some(parse);
        s.json_stringify = Some(stringify);
        s.executor = executor;
    });
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

    fn invoke<Args, H: Function<Args>>(handler: H, args: Json) -> Result<Json, Failure> {
        let Json::Array(args) = args else {
            panic!("an array")
        };
        handler
            .invoke(args)
            .map(|json| serde_json::from_str(&json).unwrap())
    }

    #[test]
    fn arguments_deserialize_and_results_serialize_with_err_as_a_failure() {
        let add = |a: i32, b: Option<i32>| a + b.unwrap_or(10);
        assert_eq!(
            invoke(add, serde_json::json!([2, 3])),
            Ok(serde_json::json!(5))
        );
        assert_eq!(
            invoke(add, serde_json::json!([2])),
            Ok(serde_json::json!(12))
        );
        assert_eq!(
            invoke(add, serde_json::json!([2, 3, 99])),
            Ok(serde_json::json!(5))
        );
        let flip = |p: Point| Point { x: p.y, y: p.x };
        assert_eq!(
            invoke(flip, serde_json::json!([{ "x": 1, "y": 2 }])),
            Ok(serde_json::json!({ "x": 2, "y": 1 }))
        );
        let wrong = invoke(flip, serde_json::json!(["nope"])).unwrap_err();
        assert_eq!(wrong.kind, 1);
        let found = |id: u32| -> Result<Point, String> {
            if id == 1 {
                Ok(Point { x: 1, y: 1 })
            } else {
                Err(format!("no point {id}"))
            }
        };
        assert_eq!(
            invoke(found, serde_json::json!([1])),
            Ok(serde_json::json!({ "x": 1, "y": 1 }))
        );
        assert_eq!(
            invoke(found, serde_json::json!([2])),
            Err(Failure::new("no point 2"))
        );
        let structured = || -> Result<(), Point> { Err(Point { x: 0, y: 7 }) };
        let failure = invoke(structured, serde_json::json!([])).unwrap_err();
        assert_eq!(failure.data.as_deref(), Some(r#"{"x":0,"y":7}"#));
        // A user enum shaped like `{ "Ok": … }` is a value, not a Result.
        #[derive(serde::Serialize)]
        #[cfg_attr(feature = "types", derive(specta::Type))]
        enum Status {
            Ok(u8),
        }
        assert_eq!(
            invoke(|| Status::Ok(1), serde_json::json!([])),
            Ok(serde_json::json!({ "Ok": 1 }))
        );
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
                eval(
                    "globalThis.__settled = {};
                     __nm.slow(21).then((v) => __settled.ok = v);
                     __nm.slow(0).catch((e) => __settled.err = e.message);
                     __nm.crash().catch((e) => __settled.crash = e.message.includes('worker kaboom'));
                     ''",
                );
                emit("demo", "moved", r#"{"x":3,"y":4}"#.into());
                settle();
                assert_eq!(
                    eval("[__settled.ok, __settled.err, __settled.crash].join()"),
                    "42,zero,true"
                );
                assert_eq!(eval("JSON.stringify(__seen)"), "[7]");
                eval("__off(); ''");
                emit("demo", "moved", r#"{"x":1,"y":1}"#.into());
                settle();
                assert_eq!(eval("JSON.stringify(__seen)"), "[7]");
            })
        });
    }
}
