//! Direct JavaScriptCore bindings for embedding applications.
//!
//! Build ordinary JS objects and native methods in `Builder::setup`. The API is
//! redjs's native value API, not a JSON transport. `Context` is thread-confined.
//! A `Value` is only GC-safe on the VM stack or while protected: never move one
//! into worker threads, even though its raw encoded representation is Copy/Send.
//! Worker tasks carry owned Rust data; create JS values inside `JsDispatcher::post`.
//!
//! Native closures consume a finite process-lifetime trampoline slot. Register
//! methods once at setup; do not create a new native function for each event.

pub use vn_runtime::redjs::api::{Atom, Context, Error, HostArgs, Result, TaKind, Value};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use vn_runtime::JsRuntime;

/// Creates a direct native JS function with argument decoding and a panic boundary.
/// An error throws a JavaScript exception; snapshots can return ordinary values
/// synchronously. Use `Context::promise` only for genuinely asynchronous work.
pub fn function<F>(context: &Context, name: &str, arity: u32, mut handler: F) -> Result<Value>
where
    F: FnMut(&Context, HostArgs) -> Result<Value> + 'static,
{
    context.function(name, arity, move |_g, frame| {
        JsRuntime::with(|runtime| {
            let context = runtime.ctx();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handler(context, unsafe { HostArgs::new(frame) })
            }));
            match result {
                Ok(Ok(value)) => value.0,
                Ok(Err(error)) => {
                    let exception = match error {
                        Error::Exception(value) => value,
                        other => context.error(0, &other.to_string()),
                    };
                    context.throw(exception);
                    Value::undefined().0
                }
                Err(_) => {
                    context.throw(context.error(0, "native plugin method panicked"));
                    Value::undefined().0
                }
            }
        })
    })
}

type Task = Box<dyn FnOnce(&Context) + Send + 'static>;
const QUEUE_CAPACITY: usize = 1024;
const DRAIN_CAP: usize = 16;

struct DispatchState {
    global: usize,
    context_id: u64,
    closed: AtomicBool,
    queue: Mutex<VecDeque<Task>>,
}
static DISPATCH: OnceLock<Arc<DispatchState>> = OnceLock::new();

/// A bounded, wakeable queue onto the one UI/VM thread. Clone and send this
/// handle to workers; post owned Rust results, never raw JSC values or callbacks.
#[derive(Clone)]
pub struct JsDispatcher {
    state: Arc<DispatchState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostError {
    Closed,
    Full,
}
impl std::fmt::Display for PostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self { Self::Closed => "the JavaScript UI has closed", Self::Full => "the JavaScript task queue is full" })
    }
}
impl std::error::Error for PostError {}

impl JsDispatcher {
    pub(crate) fn install(context: &Context) -> Self {
        let state = DISPATCH.get_or_init(|| Arc::new(DispatchState {
            global: context.raw() as usize,
            context_id: context.id().0,
            closed: AtomicBool::new(false),
            queue: Mutex::new(VecDeque::new()),
        })).clone();
        let _ = vn_runtime::redjs::web::waker(context.id());
        vn_runtime::redjs::web::register_pump_source(vn_runtime::redjs::web::PumpSource {
            inflight: dispatch_inflight, ready: dispatch_ready, drain: dispatch_drain,
        });
        Self { state }
    }

    pub fn post(&self, task: impl FnOnce(&Context) + Send + 'static) -> std::result::Result<(), PostError> {
        let mut queue = self.state.queue.lock().map_err(|_| PostError::Closed)?;
        if self.state.closed.load(Ordering::Acquire) { return Err(PostError::Closed); }
        if queue.len() >= QUEUE_CAPACITY { return Err(PostError::Full); }
        queue.push_back(Box::new(task));
        drop(queue);
        vn_runtime::redjs::web::wake(vn_runtime::redjs::context::ContextId(self.state.context_id));
        vn_runtime::frames::wake();
        Ok(())
    }
}

unsafe fn dispatch_inflight(_global: *mut c_void) -> bool { false }
unsafe fn dispatch_ready(global: *mut c_void) -> bool {
    DISPATCH.get().is_some_and(|s| s.global == global as usize
        && !s.closed.load(Ordering::Acquire) && s.queue.lock().is_ok_and(|q| !q.is_empty()))
}
unsafe fn dispatch_drain(global: *mut c_void) -> usize {
    if !unsafe { dispatch_ready(global) } { return 0; }
    let state = DISPATCH.get().expect("dispatch_ready checked the state");
    let started = Instant::now();
    let budget = Duration::from_micros(vn_runtime::redjs::web::pump_budget_us() as u64);
    let mut delivered = 0;
    while delivered < DRAIN_CAP {
        let Some(task) = state.queue.lock().ok().and_then(|mut queue| queue.pop_front()) else { break; };
        JsRuntime::with(|runtime| {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(runtime.ctx()))).is_err() {
                eprintln!("[embed] a posted JavaScript task panicked");
            }
        });
        delivered += 1;
        if started.elapsed() >= budget { break; }
    }
    delivered
}

type LifecycleHook = std::rc::Rc<dyn Fn(&Context)>;
thread_local! {
    static RELOAD_HOOKS: std::cell::RefCell<Vec<LifecycleHook>> = const { std::cell::RefCell::new(Vec::new()) };
    static SHUTDOWN_HOOKS: std::cell::RefCell<Vec<LifecycleHook>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn on_reload(hook: impl Fn(&Context) + 'static) {
    RELOAD_HOOKS.with(|hooks| hooks.borrow_mut().push(std::rc::Rc::new(hook)));
}
pub(crate) fn on_shutdown(hook: impl Fn(&Context) + 'static) {
    SHUTDOWN_HOOKS.with(|hooks| hooks.borrow_mut().push(std::rc::Rc::new(hook)));
}
fn run_hooks(hooks: Vec<LifecycleHook>, context: &Context) {
    for hook in hooks {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook(context))).is_err() {
            eprintln!("[embed] a plugin lifecycle hook panicked");
        }
    }
}
fn discard_queued_tasks(state: &DispatchState) {
    let discarded = {
        let Ok(mut queue) = state.queue.lock() else { return; };
        std::mem::take(&mut *queue)
    };
    // User-owned captures may post follow-up work from Drop. Never run their
    // destructors with the dispatcher mutex held, including during shutdown.
    drop(discarded);
}

pub(crate) fn reload(context: &Context) {
    // A queued old-page event must not enter a new subscription after reload.
    if let Some(state) = DISPATCH.get() {
        discard_queued_tasks(state);
    }
    run_hooks(RELOAD_HOOKS.with(|hooks| hooks.borrow().clone()), context);
}
pub(crate) fn shutdown(context: &Context) {
    if let Some(state) = DISPATCH.get() {
        state.closed.store(true, Ordering::Release);
        discard_queued_tasks(state);
    }
    run_hooks(SHUTDOWN_HOOKS.with(|hooks| std::mem::take(&mut *hooks.borrow_mut())), context);
    RELOAD_HOOKS.with(|hooks| hooks.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_namespace_returns_sync_values_catches_panics_and_dispatches_owned_worker_data() {
        vn_runtime::testing::on_js_thread(|| JsRuntime::with(|runtime| {
            let context = runtime.ctx();
            let namespace = context.object();
            let list = function(context, "list", 0, |context, _| {
                let array = context.array(1)?;
                context.set_index(array, 0, context.string("native snapshot"))?;
                Ok(array)
            }).unwrap();
            context.set(namespace, &context.atom("list"), list).unwrap();
            let panic = function(context, "fail", 0, |_, _| panic!("test native panic")).unwrap();
            context.set(namespace, &context.atom("fail"), panic).unwrap();
            context.global_set("__vnDirectApiTest", namespace).unwrap();
            assert_eq!(context.eval("__vnDirectApiTest.list()[0]").and_then(|v| context.read_string(v)).unwrap(), "native snapshot");
            assert_eq!(context.eval("__vnDirectApiTest.list() instanceof Promise").unwrap().as_bool(), Some(false));
            assert_eq!(context.eval("try { __vnDirectApiTest.fail(); false } catch (e) { /native plugin/.test(e.message) }").unwrap().as_bool(), Some(true));
            let dispatcher = JsDispatcher::install(context);
            std::thread::spawn(move || dispatcher.post(|context| {
                context.global_set("__vnPostedTest", context.string("owned Rust data")).unwrap();
            }).unwrap()).join().unwrap();
            assert_eq!(unsafe { dispatch_drain(context.raw()) }, 1);
            assert_eq!(context.global_get("__vnPostedTest").and_then(|v| context.read_string(v)).unwrap(), "owned Rust data");
        }));
    }

    #[test]
    fn discarded_task_captures_can_reenter_the_dispatcher_on_reload_and_shutdown() {
        struct RepostOnDrop { dispatcher: JsDispatcher, dropped: Arc<std::sync::atomic::AtomicUsize>, accepted: Arc<AtomicBool> }
        impl Drop for RepostOnDrop {
            fn drop(&mut self) {
                self.accepted.store(self.dispatcher.post(|_| {}).is_ok(), Ordering::Release);
                self.dropped.fetch_add(1, Ordering::AcqRel);
            }
        }
        fn discard_with_deadline(state: Arc<DispatchState>) {
            let (done, result) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || { discard_queued_tasks(&state); let _ = done.send(()); });
            result.recv_timeout(Duration::from_secs(2)).expect("discarding a task must not deadlock its destructor");
        }
        let state = Arc::new(DispatchState { global: 0, context_id: 0, closed: AtomicBool::new(false), queue: Mutex::new(VecDeque::new()) });
        let dispatcher = JsDispatcher { state: state.clone() };
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepted = Arc::new(AtomicBool::new(false));
        let guard = RepostOnDrop { dispatcher: dispatcher.clone(), dropped: dropped.clone(), accepted: accepted.clone() };
        dispatcher.post(move |_| drop(guard)).unwrap();
        discard_with_deadline(state.clone());
        assert_eq!(dropped.load(Ordering::Acquire), 1);
        assert!(accepted.load(Ordering::Acquire));
        assert_eq!(state.queue.lock().unwrap().len(), 1, "reload cleanup can enqueue follow-up work");
        discard_with_deadline(state.clone());
        let guard = RepostOnDrop { dispatcher: dispatcher.clone(), dropped: dropped.clone(), accepted: accepted.clone() };
        dispatcher.post(move |_| drop(guard)).unwrap();
        state.closed.store(true, Ordering::Release);
        discard_with_deadline(state.clone());
        assert_eq!(dropped.load(Ordering::Acquire), 2);
        assert!(!accepted.load(Ordering::Acquire));
        assert!(state.queue.lock().unwrap().is_empty());
    }

    #[test]
    fn worker_queue_is_bounded_and_closed_handles_reject() {
        let state = Arc::new(DispatchState { global: 0, context_id: 0, closed: AtomicBool::new(false), queue: Mutex::new(VecDeque::new()) });
        let dispatcher = JsDispatcher { state: state.clone() };
        for _ in 0..QUEUE_CAPACITY { dispatcher.post(|_| {}).unwrap(); }
        assert_eq!(dispatcher.post(|_| {}), Err(PostError::Full));
        state.closed.store(true, Ordering::Release);
        assert_eq!(dispatcher.post(|_| {}), Err(PostError::Closed));
    }
}
