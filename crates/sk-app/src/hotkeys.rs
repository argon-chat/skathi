//! System-wide hotkeys for JavaScript (`__sk_appHotkeys`), over sk-hotkeys.
//! Values and callbacks stay on the VM thread; the service's worker posts owned events.

use crate::js::{self, Context, JsDispatcher, PostError, Value};
use crate::tray::Roots;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Weak,
    },
    time::Duration,
};
pub use sk_hotkeys::{Binding, Button, Chord, Fired, Hotkeys, Status};
use sk_runtime::JsRuntime;

const MAX_LISTENERS: usize = 1024;
const MAX_ID: u64 = (1 << 53) - 1;
const MAX_BINDINGS: u32 = 128;
const MAX_BUTTONS: u32 = 16;

struct Shared {
    service: Arc<Hotkeys>,
    dispatcher: JsDispatcher,
    /// Advanced by reload and shutdown.
    realm: AtomicU64,
    /// Advanced whenever the VM thread releases the holds it delivered.
    epoch: AtomicU64,
    /// The pending capture promise's id, 0 when there is none.
    capture: AtomicU64,
    started: AtomicBool,
    stopped: AtomicBool,
}
impl Shared {
    fn new(dispatcher: JsDispatcher, started: bool) -> Arc<Self> {
        let shared = Arc::new(Self {
            service: Hotkeys::new(),
            dispatcher,
            realm: AtomicU64::new(1),
            epoch: AtomicU64::new(1),
            capture: AtomicU64::new(0),
            started: AtomicBool::new(started),
            stopped: AtomicBool::new(false),
        });
        let owner = Arc::downgrade(&shared);
        shared
            .service
            .set_handler(Arc::new(move |event| forward(&owner, event)));
        shared
    }
    fn ensure_started(&self) {
        if !self.started.swap(true, Ordering::AcqRel) {
            if let Err(error) = self.service.start() {
                eprintln!("[hotkeys] could not start: {error}");
            }
        }
    }
    fn gate(self: &Arc<Self>, event: &Fired) -> impl Fn() -> bool + Send + 'static {
        let shared = self.clone();
        let realm = self.realm.load(Ordering::Acquire);
        let epoch = self.epoch.load(Ordering::Acquire);
        let generation = event.generation;
        let release = event.phase == "up";
        move || {
            shared.realm.load(Ordering::Acquire) == realm
                && shared.epoch.load(Ordering::Acquire) == epoch
                && (release || generation == shared.service.current_generation())
        }
    }
    fn post(self: &Arc<Self>, event: &Fired) -> bool {
        let current = self.gate(event);
        let id = event.id.clone();
        let down = event.phase == "down";
        self.dispatcher
            .post(move |context| {
                if current() {
                    emit(context, &id, down);
                }
            })
            .is_ok()
    }
}

// The service's event worker, never the input hook. A full UI queue is retried, but restart and
// stop join this worker, so it gives up as soon as the hooks are down.
fn forward(owner: &Weak<Shared>, event: Fired) {
    let Some(shared) = owner.upgrade() else {
        return;
    };
    let realm = shared.realm.load(Ordering::Acquire);
    let epoch = shared.epoch.load(Ordering::Acquire);
    while !shared.stopped.load(Ordering::Acquire)
        && shared.realm.load(Ordering::Acquire) == realm
        && shared.epoch.load(Ordering::Acquire) == epoch
    {
        let running = shared.service.is_running();
        if !shared.service.is_current(&event) || (!running && event.phase == "down") {
            break;
        }
        if shared.post(&event) || !running {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn complete_capture(owner: &Weak<Shared>, id: u64, result: Result<Chord, String>) {
    let Some(shared) = owner.upgrade() else {
        return;
    };
    while !shared.stopped.load(Ordering::Acquire) && shared.capture.load(Ordering::Acquire) == id {
        let result = result.clone();
        match shared
            .dispatcher
            .post(move |context| settle(context, id, result))
        {
            Err(PostError::Full) => std::thread::sleep(Duration::from_millis(10)),
            _ => return,
        }
    }
}

struct Pending {
    id: u64,
    resolve: Value,
    reject: Value,
}
#[derive(Default)]
struct State {
    shared: Option<Arc<Shared>>,
    dispatcher: Option<JsDispatcher>,
    closed: bool,
    next_id: u64,
    listeners: BTreeMap<u64, Value>,
    held: BTreeSet<String>,
    capture: Option<Pending>,
    unsubscribe: Option<Value>,
    is_array: Option<Value>,
    object_keys: Option<Value>,
}
impl State {
    fn token(&mut self) -> Option<u64> {
        if self.closed || self.next_id >= MAX_ID {
            return None;
        }
        self.next_id += 1;
        Some(self.next_id)
    }
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

fn type_error(context: &Context, message: &str) -> js::Error {
    js::Error::Exception(context.error(1, message))
}
fn error(context: &Context, message: &str) -> js::Error {
    js::Error::Exception(context.error(0, message))
}
fn describe(context: &Context, error: js::Error) -> String {
    match error {
        js::Error::Exception(value) => context.error_info(value).0,
        other => other.to_string(),
    }
}
fn existing() -> Option<Arc<Shared>> {
    STATE.with(|state| state.borrow().shared.clone())
}
fn shared(context: &Context) -> js::Result<Arc<Shared>> {
    STATE
        .with(|state| {
            let mut state = state.borrow_mut();
            if state.closed {
                return None;
            }
            if let Some(shared) = &state.shared {
                return Some(shared.clone());
            }
            let shared = Shared::new(state.dispatcher.clone()?, false);
            state.shared = Some(shared.clone());
            Some(shared)
        })
        .ok_or_else(|| error(context, "global hotkeys have shut down"))
}

fn emit(context: &Context, id: &str, down: bool) {
    let listeners = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.listeners.is_empty() {
            return Vec::new();
        }
        if down {
            state.held.insert(id.to_owned());
        } else {
            state.held.remove(id);
        }
        state.listeners.keys().copied().collect()
    });
    if listeners.is_empty() {
        return;
    }
    let payload = context.object();
    let filled = context
        .set(payload, &context.atom("id"), context.string(id))
        .and_then(|_| {
            context.set(
                payload,
                &context.atom("phase"),
                context.string(if down { "down" } else { "up" }),
            )
        });
    if let Err(error) = filled {
        sk_tree::log::record("error", format!("hotkey event: {}", describe(context, error)));
        return;
    }
    context.protect(payload);
    for key in listeners {
        let Some(callback) = STATE.with(|state| state.borrow().listeners.get(&key).copied()) else {
            continue;
        };
        // A listener may unsubscribe itself, or another, while it runs.
        context.protect(callback);
        if let Err(error) = context.call(callback, Value::undefined(), &[payload]) {
            sk_tree::log::record(
                "error",
                format!("hotkey listener: {}", describe(context, error)),
            );
        }
        context.unprotect(callback);
    }
    context.unprotect(payload);
}

/// Every hold this page saw go down gets its up now, and older queued events are retired.
fn release_held(context: &Context, shared: &Shared) {
    shared.epoch.fetch_add(1, Ordering::AcqRel);
    let held = STATE.with(|state| std::mem::take(&mut state.borrow_mut().held));
    for id in held {
        emit(context, &id, false);
    }
}

fn remove_listener(context: &Context, id: u64) {
    if let Some(callback) = STATE.with(|state| state.borrow_mut().listeners.remove(&id)) {
        context.unprotect(callback);
    }
}
fn subscribe(context: &Context, callback: Value) -> js::Result<Value> {
    if context.kind(callback) != 9 {
        return Err(type_error(context, "onFired expects a function"));
    }
    let (id, factory) = STATE
        .with(|state| {
            let mut state = state.borrow_mut();
            if state.listeners.len() >= MAX_LISTENERS {
                return None;
            }
            let factory = state.unsubscribe?;
            let id = state.token()?;
            context.protect(callback);
            state.listeners.insert(id, callback);
            Some((id, factory))
        })
        .ok_or_else(|| {
            error(
                context,
                "hotkey listeners are closed or their limit was reached",
            )
        })?;
    context
        .call(factory, Value::undefined(), &[Value::number(id as f64)])
        .inspect_err(|_| remove_listener(context, id))
}

fn reject_pending(context: &Context, pending: Pending, reason: &str) {
    let _ = context.call(
        pending.reject,
        Value::undefined(),
        &[context.error(0, reason)],
    );
    context.unprotect(pending.resolve);
    context.unprotect(pending.reject);
}
fn begin_capture(context: &Context) -> js::Result<(u64, Value)> {
    let (promise, resolve, reject) = context.promise()?;
    let (id, previous) = STATE
        .with(|state| {
            let mut state = state.borrow_mut();
            let id = state.token()?;
            context.protect(resolve);
            context.protect(reject);
            let previous = state.capture.replace(Pending {
                id,
                resolve,
                reject,
            });
            Some((id, previous))
        })
        .ok_or_else(|| error(context, "global hotkeys have shut down"))?;
    if let Some(previous) = previous {
        reject_pending(context, previous, "hotkey capture superseded");
    }
    Ok((id, promise))
}
fn settle(context: &Context, id: u64, result: Result<Chord, String>) {
    let pending = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.capture.as_ref().is_some_and(|pending| pending.id == id) {
            state.capture.take()
        } else {
            None
        }
    });
    let Some(pending) = pending else {
        return;
    };
    if let Some(shared) = existing() {
        let _ = shared
            .capture
            .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
    }
    let (callback, value) = match result.map(|chord| chord_value(context, &chord)) {
        Ok(Ok(value)) => (pending.resolve, value),
        Ok(Err(error)) => (pending.reject, context.error(0, &describe(context, error))),
        Err(message) => (pending.reject, context.error(0, &message)),
    };
    let _ = context.call(callback, Value::undefined(), &[value]);
    context.unprotect(pending.resolve);
    context.unprotect(pending.reject);
}
fn cancel_pending(context: &Context) {
    if let Some(pending) = STATE.with(|state| state.borrow_mut().capture.take()) {
        if let Some(shared) = existing() {
            shared.capture.store(0, Ordering::Release);
        }
        reject_pending(context, pending, "hotkey capture cancelled");
    }
}

struct Reader<'a> {
    context: &'a Context,
    is_array: Value,
    object_keys: Value,
    roots: Roots<'a>,
}
impl<'a> Reader<'a> {
    fn invalid(&self, message: &str) -> js::Error {
        type_error(
            self.context,
            &format!("invalid hotkey bindings: {message}"),
        )
    }
    fn get(&mut self, object: Value, name: &str) -> js::Result<Value> {
        let value = self.context.get(object, &self.context.atom(name))?;
        Ok(self.roots.keep(value))
    }
    fn object(&self, value: Value, what: &str) -> js::Result<()> {
        if self.context.kind(value) != 8 {
            return Err(self.invalid(&format!("{what} must be an object")));
        }
        Ok(())
    }
    // Vue's reactive arrays are Proxy(Array): JSC reports them as objects, Array.isArray does not.
    fn array(&mut self, value: Value, what: &str, max: u32, too_many: &str) -> js::Result<u32> {
        let kind = self.context.kind(value);
        let array = kind == 10
            || (kind == 8
                && self
                    .context
                    .call(self.is_array, Value::undefined(), &[value])?
                    .as_bool()
                    == Some(true));
        if !array {
            return Err(self.invalid(&format!("{what} must be an array")));
        }
        let length = self.get(value, "length")?;
        let length = length
            .as_number()
            .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 0.0)
            .ok_or_else(|| self.invalid(&format!("{what} has an invalid length")))?;
        if length > max as f64 {
            return Err(type_error(self.context, too_many));
        }
        Ok(length as u32)
    }
    fn integer(&self, value: Value, name: &str, max: f64) -> js::Result<f64> {
        value
            .as_number()
            .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= max)
            .ok_or_else(|| self.invalid(&format!("{name} must be an integer from 0 to {max}")))
    }
    fn binding(&mut self, item: Value) -> js::Result<Binding> {
        self.object(item, "a binding")?;
        let names = self
            .context
            .call(self.object_keys, Value::undefined(), &[item])?;
        let names = self.roots.keep(names);
        for index in 0..self.context.array_length(names)? {
            let name = self
                .context
                .read_string(self.context.get_index(names, index)?)?;
            if !matches!(
                name.as_str(),
                "id" | "chord" | "suppress" | "triggerCooldownMs"
            ) {
                return Err(self.invalid(&format!("unknown field `{name}`")));
            }
        }
        let id = self.get(item, "id")?;
        if self.context.kind(id) != 5 {
            return Err(self.invalid("id must be a string"));
        }
        let id = self.context.read_string(id)?;
        let chord = self.get(item, "chord")?;
        self.object(chord, "chord")?;
        let list = self.get(chord, "buttons")?;
        let count = self.array(
            list,
            "chord.buttons",
            MAX_BUTTONS,
            "hotkey chord or cooldown exceeds its limit",
        )?;
        let mut buttons = Vec::with_capacity(count as usize);
        for index in 0..count {
            let button = self.context.get_index(list, index)?;
            let button = self.roots.keep(button);
            self.object(button, "a chord button")?;
            let device = self.get(button, "device")?;
            let device = self.integer(device, "device", 255.0)?;
            let code = self.get(button, "code")?;
            let code = self.integer(code, "code", u32::MAX as f64)?;
            buttons.push(Button {
                device: device as u8,
                code: code as u32,
            });
        }
        let suppress = self.get(item, "suppress")?;
        let suppress = if suppress.is_undefined() || suppress.is_null() {
            None
        } else {
            Some(
                suppress
                    .as_bool()
                    .ok_or_else(|| self.invalid("suppress must be a boolean"))?,
            )
        };
        let cooldown = self.get(item, "triggerCooldownMs")?;
        let trigger_cooldown_ms = if cooldown.is_undefined() || cooldown.is_null() {
            None
        } else {
            Some(self.integer(cooldown, "triggerCooldownMs", MAX_ID as f64)? as u64)
        };
        Ok(Binding {
            id,
            chord: Chord { buttons },
            suppress,
            trigger_cooldown_ms,
        })
    }
}
/// Reads the bindings with ordinary property access, so getters and proxies behave as in JS.
fn decode(context: &Context, value: Value) -> js::Result<Vec<Binding>> {
    let (is_array, object_keys) = STATE
        .with(|state| {
            let state = state.borrow();
            Some((state.is_array?, state.object_keys?))
        })
        .ok_or_else(|| error(context, "global hotkeys have shut down"))?;
    let mut reader = Reader {
        context,
        is_array,
        object_keys,
        roots: Roots::new(context),
    };
    let count = reader.array(
        value,
        "bindings",
        MAX_BINDINGS,
        "at most 128 hotkey bindings are allowed",
    )?;
    let mut bindings = Vec::with_capacity(count as usize);
    for index in 0..count {
        let item = context.get_index(value, index)?;
        let item = reader.roots.keep(item);
        bindings.push(reader.binding(item)?);
    }
    Ok(bindings)
}

fn chord_value(context: &Context, chord: &Chord) -> js::Result<Value> {
    let mut roots = Roots::new(context);
    let buttons = roots.keep(context.array(chord.buttons.len() as u32)?);
    for (index, button) in chord.buttons.iter().enumerate() {
        let item = roots.keep(context.object());
        context.set(
            item,
            &context.atom("device"),
            Value::number(button.device as f64),
        )?;
        context.set(item, &context.atom("code"), Value::number(button.code as f64))?;
        context.set_index(buttons, index as u32, item)?;
    }
    let chord = context.object();
    context.set(chord, &context.atom("buttons"), buttons)?;
    Ok(chord)
}
fn status_value(context: &Context, status: &Status) -> js::Result<Value> {
    let object = context.object();
    let mut roots = Roots::new(context);
    roots.keep(object);
    context.set(object, &context.atom("running"), Value::boolean(status.running))?;
    context.set(
        object,
        &context.atom("platform"),
        context.string(&status.platform),
    )?;
    context.set(
        object,
        &context.atom("accessibilityGranted"),
        Value::boolean(status.accessibility_granted),
    )?;
    let error = status
        .error
        .as_deref()
        .map_or(Value::null(), |error| context.string(error));
    context.set(object, &context.atom("error"), error)?;
    context.set(
        object,
        &context.atom("restarts"),
        Value::number(status.restarts as f64),
    )?;
    Ok(object)
}

fn set_bindings(context: &Context, value: Value) -> js::Result<Value> {
    let bindings = decode(context, value)?;
    let shared = shared(context)?;
    shared.ensure_started();
    if !shared
        .service
        .would_change(&bindings)
        .map_err(|message| type_error(context, &message))?
    {
        return Ok(Value::boolean(false));
    }
    release_held(context, &shared);
    shared
        .service
        .set_bindings(bindings)
        .map(Value::boolean)
        .map_err(|message| type_error(context, &message))
}
fn capture_once(context: &Context) -> js::Result<Value> {
    let shared = shared(context)?;
    shared.ensure_started();
    release_held(context, &shared);
    let (id, promise) = begin_capture(context)?;
    shared.capture.store(id, Ordering::Release);
    let owner = Arc::downgrade(&shared);
    if let Err(message) = shared
        .service
        .capture_once(move |result| complete_capture(&owner, id, result))
    {
        // The previous promise is already superseded; a refused successor must not leave the
        // previous native capture running.
        shared.service.cancel_capture();
        settle(context, id, Err(message));
    }
    Ok(promise)
}
fn restart(context: &Context) -> js::Result<Value> {
    let shared = shared(context)?;
    shared.started.store(true, Ordering::Release);
    release_held(context, &shared);
    shared
        .service
        .restart()
        .map(Value::boolean)
        .map_err(|message| error(context, &message))
}
fn cancel_capture(context: &Context) {
    if let Some(shared) = existing() {
        shared.service.cancel_capture();
    }
    cancel_pending(context);
}

fn clear(context: &Context, shutdown: bool) {
    let (listeners, capture, roots) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.held.clear();
        let listeners = std::mem::take(&mut state.listeners);
        let capture = state.capture.take();
        let roots = if shutdown {
            state.closed = true;
            [
                state.unsubscribe.take(),
                state.is_array.take(),
                state.object_keys.take(),
            ]
        } else {
            [None; 3]
        };
        (listeners, capture, roots)
    });
    if let Some(shared) = existing() {
        shared.capture.store(0, Ordering::Release);
    }
    for callback in listeners.into_values() {
        context.unprotect(callback);
    }
    if let Some(pending) = capture {
        reject_pending(context, pending, "hotkey capture cancelled");
    }
    for root in roots.into_iter().flatten() {
        context.unprotect(root);
    }
}
fn reload(context: &Context) {
    if let Some(shared) = existing() {
        shared.realm.fetch_add(1, Ordering::AcqRel);
        shared.service.cancel_capture();
    }
    clear(context, false);
}
fn shutdown(context: &Context) {
    let shared = existing();
    if let Some(shared) = &shared {
        shared.realm.fetch_add(1, Ordering::AcqRel);
        shared.stopped.store(true, Ordering::Release);
        shared.service.cancel_capture();
    }
    clear(context, true);
    if let Some(shared) = shared {
        shared.service.stop();
    }
}

fn install_roots(context: &Context) -> js::Result<()> {
    let remove = js::function(context, "removeListener", 1, |context, args| {
        if let Some(id) = args.arg(0).as_number().filter(|id| {
            id.is_finite() && *id > 0.0 && id.fract() == 0.0 && *id <= MAX_ID as f64
        }) {
            remove_listener(context, id as u64);
        }
        Ok(Value::undefined())
    })?;
    let factory = context.eval("(function(remove){return function(id){let active=true;return function unsubscribe(){if(active){active=false;remove(id);}};};})")?;
    let unsubscribe = context.call(factory, Value::undefined(), &[remove])?;
    let is_array = context.eval("Array.isArray")?;
    let object_keys = context.eval("Object.keys")?;
    for value in [unsubscribe, is_array, object_keys] {
        context.protect(value);
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.unsubscribe = Some(unsubscribe);
        state.is_array = Some(is_array);
        state.object_keys = Some(object_keys);
    });
    Ok(())
}
fn method(
    context: &Context,
    roots: &mut Roots,
    object: Value,
    name: &str,
    arity: u32,
    handler: impl FnMut(&Context, js::HostArgs) -> js::Result<Value> + 'static,
) -> js::Result<()> {
    let function = roots.keep(js::function(context, name, arity, handler)?);
    context.set(object, &context.atom(name), function)
}
fn namespace(context: &Context) -> js::Result<Value> {
    let mut roots = Roots::new(context);
    let object = roots.keep(context.object());
    let r = &mut roots;
    method(context, r, object, "setBindings", 1, |context, args| {
        set_bindings(context, args.arg(0))
    })?;
    method(context, r, object, "status", 0, |context, _| {
        status_value(context, &shared(context)?.service.status())
    })?;
    method(context, r, object, "requestAccessibility", 0, |context, _| {
        Ok(Value::boolean(
            shared(context)?.service.request_accessibility(),
        ))
    })?;
    method(context, r, object, "restart", 0, |context, _| restart(context))?;
    method(context, r, object, "captureOnce", 0, |context, _| {
        capture_once(context)
    })?;
    method(context, r, object, "cancelCapture", 0, |context, _| {
        cancel_capture(context);
        Ok(Value::undefined())
    })?;
    method(context, r, object, "onFired", 1, |context, args| {
        subscribe(context, args.arg(0))
    })?;
    context.set(
        object,
        &context.atom("available"),
        Value::boolean(cfg!(any(windows, target_os = "macos"))),
    )?;
    Ok(object)
}

pub(crate) fn install(runtime: &JsRuntime) -> std::result::Result<(), String> {
    let context = runtime.ctx();
    let dispatcher = JsDispatcher::install(context);
    STATE.with(|state| state.borrow_mut().dispatcher = Some(dispatcher));
    let setup = || -> js::Result<()> {
        install_roots(context)?;
        let object = namespace(context)?;
        context.global_set("__sk_appHotkeys", object)
    };
    setup().map_err(|error| describe(context, error))?;
    js::on_reload(reload);
    js::on_shutdown(shutdown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(context: &Context) -> Arc<Shared> {
        clear(context, false);
        let dispatcher = JsDispatcher::install(context);
        // `started`: a test never installs the real hooks.
        let shared = Shared::new(dispatcher.clone(), true);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.closed = false;
            state.dispatcher = Some(dispatcher);
            state.shared = Some(shared.clone());
        });
        if STATE.with(|state| state.borrow().unsubscribe.is_none()) {
            install_roots(context).unwrap();
        }
        context
            .global_set("__hotkeyProbe", namespace(context).unwrap())
            .unwrap();
        shared
    }
    fn text(context: &Context, source: &str) -> String {
        context
            .eval(source)
            .and_then(|value| context.read_string(value))
            .unwrap()
    }
    fn rejection(context: &Context, promise: Value) -> String {
        let (status, reason) = context.promise_info(promise).unwrap();
        assert_eq!(status, 2, "the promise must be rejected");
        context.error_info(reason).0
    }
    fn held(id: &str) -> bool {
        STATE.with(|state| state.borrow().held.contains(id))
    }
    fn f(code: u32) -> Binding {
        Binding {
            id: "ptt".into(),
            chord: Chord {
                buttons: vec![Button::keyboard(code)],
            },
            suppress: None,
            trigger_cooldown_ms: None,
        }
    }

    #[test]
    fn hotkey_bindings_read_reactive_proxies_and_reject_bad_shapes() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let shared = fresh(context);
                context.eval("globalThis.__savedStringify=JSON.stringify;JSON.stringify=()=>{throw new Error('must not serialize')}").unwrap();
                let accepted = context.eval("__hotkeyProbe.setBindings(new Proxy([{id:'proxy-f24',chord:new Proxy({buttons:new Proxy([{device:0,code:135}],{})},{}),suppress:true}],{}))");
                context
                    .eval("JSON.stringify=globalThis.__savedStringify")
                    .unwrap();
                assert_eq!(
                    accepted.unwrap().as_bool(),
                    Some(true),
                    "the client's nested reactive proxy shape"
                );
                let expected = Binding {
                    id: "proxy-f24".into(),
                    chord: Chord {
                        buttons: vec![Button::keyboard(135)],
                    },
                    suppress: Some(true),
                    trigger_cooldown_ms: None,
                };
                assert!(!shared.service.would_change(&[expected]).unwrap());
                assert_eq!(
                    context
                        .eval("__hotkeyProbe.setBindings([])")
                        .unwrap()
                        .as_bool(),
                    Some(true)
                );
                for source in [
                    "({})",
                    "[{id:'x',chord:{buttons:[]},extra:1}]",
                    "[{id:'x'}]",
                    "[{id:1,chord:{buttons:[]}}]",
                    "[{id:'x',chord:{}}]",
                    "[{id:'x',chord:{buttons:[{device:0,code:1.5}]}}]",
                    "[{id:'x',chord:{buttons:[{device:-1,code:1}]}}]",
                    "[{id:'x',chord:{buttons:[{device:2,code:1}]}}]",
                    "[{id:'x',chord:{buttons:[{device:0,code:65}]},suppress:1}]",
                    "[{id:'x',chord:{buttons:[{device:0,code:65}]},triggerCooldownMs:60001}]",
                    "[{id:'x',chord:{buttons:new Array(17).fill({device:0,code:65})}}]",
                    "new Array(129).fill(0)",
                    "[{id:'',chord:{buttons:[{device:0,code:65}]}}]",
                    "[{id:'x',chord:{buttons:[{device:0,code:65}]}},{id:'x',chord:{buttons:[{device:0,code:66}]}}]",
                ] {
                    let thrown = context.eval(&format!("(()=>{{try{{__hotkeyProbe.setBindings({source});return ''}}catch(e){{return e.name}}}})()")).and_then(|v| context.read_string(v)).unwrap();
                    assert_eq!(thrown, "TypeError", "{source}");
                }
                assert_eq!(
                    text(context, "(()=>{try{__hotkeyProbe.setBindings([{get id(){throw new Error('getter failed')},chord:{buttons:[]}}])}catch(e){return e.message}})()"),
                    "getter failed",
                    "a getter's exception propagates unchanged"
                );
                assert_eq!(
                    context
                        .eval("__hotkeyProbe.setBindings([])")
                        .unwrap()
                        .as_bool(),
                    Some(false),
                    "a refused call leaves the bindings as they were"
                );
                let status = text(
                    context,
                    "JSON.stringify(Object.keys(__hotkeyProbe.status()))",
                );
                assert_eq!(
                    status,
                    r#"["running","platform","accessibilityGranted","error","restarts"]"#
                );
            })
        });
    }

    #[test]
    fn hotkey_holds_are_released_once_and_stale_deliveries_are_gated() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let shared = fresh(context);
                context.eval("globalThis.__hotkeyEvents=[];globalThis.__hotkeyOff=__hotkeyProbe.onFired(event=>__hotkeyEvents.push(event.id+':'+event.phase+':'+Object.keys(event).length))").unwrap();
                emit(context, "push-to-talk", true);
                assert!(held("push-to-talk"));
                assert_eq!(
                    context
                        .eval("__hotkeyProbe.setBindings([])")
                        .unwrap()
                        .as_bool(),
                    Some(false)
                );
                assert!(
                    held("push-to-talk"),
                    "unchanged bindings must keep the delivered hold"
                );
                let queued_down = shared.gate(&Fired {
                    id: "push-to-talk".into(),
                    phase: "down".into(),
                    generation: shared.service.current_generation(),
                });
                release_held(context, &shared);
                assert!(!queued_down(), "a queued down is retired by the release");
                assert!(!held("push-to-talk"));
                assert_eq!(
                    text(context, "__hotkeyEvents.join(',')"),
                    "push-to-talk:down:2,push-to-talk:up:2"
                );
                release_held(context, &shared);
                assert_eq!(
                    context.eval("__hotkeyEvents.length").unwrap().as_number(),
                    Some(2.0),
                    "a repeated release must not repeat the up"
                );
                emit(context, "push-to-talk", true);
                emit(context, "push-to-talk", false);
                assert!(!held("push-to-talk"));

                let event = Fired {
                    id: "ptt".into(),
                    phase: "down".into(),
                    generation: shared.service.current_generation(),
                };
                let queued = shared.gate(&event);
                assert!(queued());
                shared.service.set_bindings(vec![f(0x70)]).unwrap();
                assert!(
                    !queued(),
                    "a posted down must not adopt a later backend generation"
                );
                assert!(
                    !shared.gate(&event)(),
                    "a down already taken by the worker keeps its source generation"
                );
                let release = shared.gate(&Fired {
                    phase: "up".into(),
                    ..event
                });
                assert!(release(), "an old up can balance a delivered hold");
                shared.epoch.fetch_add(1, Ordering::AcqRel);
                assert!(!release(), "the VM thread already released that hold");

                let current = shared.gate(&Fired {
                    id: "ptt".into(),
                    phase: "up".into(),
                    generation: 0,
                });
                assert!(current());
                assert!(shared.post(&Fired {
                    id: "ptt".into(),
                    phase: "up".into(),
                    generation: 0,
                }));
                reload(context);
                assert!(!current(), "reload retires queued deliveries");
                runtime.pump(sk_runtime::runtime::PUMP_TIMERS);
                assert_eq!(
                    context.eval("__hotkeyEvents.length").unwrap().as_number(),
                    Some(4.0)
                );
                assert!(STATE.with(|state| state.borrow().listeners.is_empty()));
                context.eval("__hotkeyOff();__hotkeyOff()").unwrap();

                context.eval("__hotkeyProbe.onFired(event=>__hotkeyEvents.push(event.phase))").unwrap();
                assert!(shared.post(&Fired {
                    id: "ptt".into(),
                    phase: "up".into(),
                    generation: 0,
                }));
                runtime.pump(sk_runtime::runtime::PUMP_TIMERS);
                assert_eq!(text(context, "String(__hotkeyEvents[4])"), "up");
                clear(context, false);
            })
        });
    }

    #[test]
    fn hotkey_capture_promises_settle_once_and_reload_cancels_them() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let shared = fresh(context);
                let refused = context.eval("__hotkeyProbe.captureOnce()").unwrap();
                assert!(
                    rejection(context, refused).contains("hotkeys are not running"),
                    "a host whose hooks are down rejects at once"
                );

                let (first, first_promise) = begin_capture(context).unwrap();
                shared.capture.store(first, Ordering::Release);
                let (second, second_promise) = begin_capture(context).unwrap();
                shared.capture.store(second, Ordering::Release);
                assert!(rejection(context, first_promise).contains("hotkey capture superseded"));
                let owner = Arc::downgrade(&shared);
                let chord = Chord {
                    buttons: vec![Button::keyboard(65)],
                };
                complete_capture(&owner, first, Ok(chord.clone()));
                runtime.pump(sk_runtime::runtime::PUMP_TIMERS);
                assert_eq!(context.promise_info(second_promise).unwrap().0, 0);
                complete_capture(&owner, second, Ok(chord));
                runtime.pump(sk_runtime::runtime::PUMP_TIMERS);
                let (status, captured) = context.promise_info(second_promise).unwrap();
                assert_eq!(status, 1);
                context.global_set("__captured", captured).unwrap();
                assert_eq!(
                    text(context, "JSON.stringify(__captured)"),
                    r#"{"buttons":[{"device":0,"code":65}]}"#
                );
                assert_eq!(shared.capture.load(Ordering::Acquire), 0);

                let (third, cancelled) = begin_capture(context).unwrap();
                shared.capture.store(third, Ordering::Release);
                context.eval("__hotkeyProbe.cancelCapture()").unwrap();
                assert!(
                    rejection(context, cancelled).contains("hotkey capture cancelled"),
                    "cancelCapture rejects synchronously on the VM thread"
                );
                complete_capture(&owner, third, Err("hotkey capture cancelled".into()));

                let (fourth, pending) = begin_capture(context).unwrap();
                shared.capture.store(fourth, Ordering::Release);
                reload(context);
                assert!(rejection(context, pending).contains("hotkey capture cancelled"));
                assert_eq!(shared.capture.load(Ordering::Acquire), 0);

                clear(context, true);
                assert!(STATE.with(|state| {
                    let state = state.borrow();
                    state.closed
                        && state.listeners.is_empty()
                        && state.capture.is_none()
                        && state.unsubscribe.is_none()
                        && state.is_array.is_none()
                        && state.object_keys.is_none()
                }));
                assert!(begin_capture(context).is_err());
            })
        });
    }
}
