//! Application-scoped native notifications. Only owned Rust data crosses into the OS worker.

mod image;
mod options;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::Backend;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::Backend;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::Backend;

use crate::js::{self, Context, Value};
use crate::tray::Roots;
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use sk_runtime::{
    redjs::{context::ContextId, web},
    JsRuntime,
};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::{mpsc, Mutex, OnceLock};

pub(crate) fn initialize() {
    #[cfg(target_os = "macos")]
    macos::initialize();
    #[cfg(target_os = "windows")]
    windows::initialize();
}

pub(crate) fn background_launch() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::background_launch()
    }
    #[cfg(target_os = "macos")]
    {
        macos::background_launch()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

struct Capabilities {
    management: &'static str,
    activation: &'static str,
    max_actions: usize,
    silent: bool,
    inline_reply: bool,
    grouping: bool,
    image: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            management: "none",
            activation: "none",
            max_actions: 0,
            silent: false,
            inline_reply: false,
            grouping: false,
            image: false,
        }
    }
}

impl Capabilities {
    fn json(&self) -> Json {
        json!({
            "available": self.management != "none", "management": self.management,
            "activation": self.activation, "maxActions": self.max_actions, "silent": self.silent,
            "inlineReply": self.inline_reply, "image": self.image, "grouping": self.grouping
        })
    }
}

#[derive(Clone, Debug)]
struct Action {
    id: String,
    label: String,
    reply: bool,
}

struct Group {
    id: String,
    label: String,
}

struct Notice {
    id: String,
    native_id: String,
    title: String,
    body: Option<String>,
    actions: Vec<Action>,
    silent: bool,
    group: Option<Group>,
    image: Option<image::PreparedImage>,
}

fn native_id(id: &str) -> String {
    let mut hash = Sha256::new();
    let identity = sk_desktop::launch::identity();
    if let Some(identity) = identity {
        hash.update(identity.id.as_bytes());
    }
    hash.update([0]);
    if let Some(dir) = crate::embed::data_dir() {
        hash.update(b"path\0");
        #[cfg(windows)]
        {
            static PROFILE: OnceLock<String> = OnceLock::new();
            hash.update(
                PROFILE
                    .get_or_init(|| sk_desktop::launch::normalized_profile_dir(dir))
                    .as_bytes(),
            );
        }
        #[cfg(not(windows))]
        hash.update(dir.as_os_str().as_encoded_bytes());
    } else if let Some(identity) = identity {
        hash.update(b"name\0");
        hash.update(sk_desktop::launch::storage_name(&identity.name).as_bytes());
    }
    hash.update([0]);
    hash.update(id.as_bytes());
    hex::encode(&hash.finalize()[..16])
}

#[derive(Default)]
struct Submission {
    actions: Vec<String>,
    body_omitted: bool,
    group_omitted: bool,
    image_omitted: bool,
}

#[derive(Debug)]
struct Error {
    name: &'static str,
    message: String,
}

impl Error {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            name: "TypeError",
            message: message.into(),
        }
    }
    fn unsupported(message: impl Into<String>) -> Self {
        Self {
            name: "NotSupportedError",
            message: message.into(),
        }
    }
    fn denied(message: impl Into<String>) -> Self {
        Self {
            name: "NotAllowedError",
            message: message.into(),
        }
    }
    fn operation(message: impl Into<String>) -> Self {
        Self {
            name: "OperationError",
            message: message.into(),
        }
    }
    fn value(&self, cx: &Context) -> Value {
        if self.name == "TypeError" {
            return cx.error(1, &self.message);
        }
        Value(unsafe {
            sk_runtime::redjs::webapi::domexception::create(cx.raw(), self.name, &self.message)
        })
    }
}

enum Request {
    Capabilities,
    Show(Notice, Option<image::ImageInput>),
    Close(String),
}

type Output = Result<Option<Json>, Error>;
impl Request {
    fn run(self, backend: &mut Backend) -> Output {
        match self {
            Self::Capabilities => {
                let caps = match backend.capabilities() {
                    Err(error) if error.name == "NotSupportedError" => Capabilities::default(),
                    result => result?,
                };
                Ok(Some(caps.json()))
            }
            Self::Close(id) => {
                backend.close(&id)?;
                Ok(None)
            }
            Self::Show(mut notice, image) => {
                notice.image = image.map(image::prepare).transpose()?;
                let submission = backend.show(&notice)?;
                let mut omitted = Vec::new();
                if submission.body_omitted {
                    omitted.push("body");
                }
                if submission.group_omitted {
                    omitted.push("group");
                }
                if submission.image_omitted {
                    omitted.push("image");
                }
                Ok(Some(
                    json!({ "id": notice.id, "submittedActions": submission.actions, "omitted": omitted }),
                ))
            }
        }
    }
}

#[derive(Default)]
struct Page {
    worker: Option<mpsc::SyncSender<(u64, Request)>>,
    next: u64,
    pending: HashMap<u64, (Value, Value)>,
    handler: Option<Value>,
    closed: bool,
}

thread_local! { static PAGE: RefCell<Page> = RefCell::new(Page::default()); }

#[derive(Default)]
struct Inbox {
    results: VecDeque<(u64, Output)>,
    events: VecDeque<Activation>,
    focus: bool,
    focus_token: Option<String>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox {
    results: VecDeque::new(),
    events: VecDeque::new(),
    focus: false,
    focus_token: None,
});
static PUMP: OnceLock<(usize, ContextId)> = OnceLock::new();

#[cfg(target_os = "linux")]
static WAYLAND: OnceLock<bool> = OnceLock::new();

#[cfg(target_os = "linux")]
pub(crate) fn set_display(compositor: &str) {
    let _ = WAYLAND.set(compositor == "Wayland");
}

#[cfg(target_os = "linux")]
fn on_wayland() -> bool {
    WAYLAND.get().copied().unwrap_or(false)
}

fn wake() {
    if let Some((_, id)) = PUMP.get() {
        web::wake(*id);
    }
}

enum Activation {
    Open(String),
    Button(String, String),
    Reply(String, String, String),
}

#[cfg(target_os = "linux")]
fn open_with_token(id: String, token: Option<String>) {
    let mut inbox = INBOX.lock().unwrap();
    inbox.focus = true;
    inbox.focus_token = token.filter(|t| !t.is_empty());
    inbox.events.push_back(Activation::Open(id));
    drop(inbox);
    wake();
}

fn activate(id: String, action: Option<String>) {
    enqueue(match action {
        None => Activation::Open(id),
        Some(action) => Activation::Button(id, action),
    });
}
fn reply(id: String, action: String, text: String) {
    enqueue(Activation::Reply(id, action, text));
}
fn enqueue(event: Activation) {
    let mut inbox = INBOX.lock().unwrap();
    inbox.focus |= matches!(event, Activation::Open(_));
    inbox.events.push_back(event);
    drop(inbox);
    wake();
}

fn start_worker() -> Result<mpsc::SyncSender<(u64, Request)>, Error> {
    let (tx, rx) = mpsc::sync_channel::<(u64, Request)>(256);
    std::thread::Builder::new()
        .name("sk-notifications".into())
        .spawn(move || {
            let mut backend = Backend::new();
            for (token, request) in rx {
                let result = request.run(&mut backend);
                INBOX.lock().unwrap().results.push_back((token, result));
                wake();
            }
        })
        .map_err(|e| Error::operation(e.to_string()))?;
    Ok(tx)
}

fn exception(cx: &Context, err: js::Error) -> Value {
    match err {
        js::Error::Exception(value) => value,
        other => cx.error(0, &other.to_string()),
    }
}

fn submit(cx: &Context, request: js::Result<Request>) -> js::Result<Value> {
    let (promise, resolve, reject) = cx.promise()?;
    let mut roots = Roots::new(cx);
    for value in [promise, resolve, reject] {
        roots.keep(value);
    }
    let queued = request.map_err(|e| exception(cx, e)).and_then(|request| {
        PAGE.with(|page| -> Result<(), Error> {
            let mut page = page.borrow_mut();
            if page.closed || page.pending.len() >= 256 {
                return Err(Error::operation("Notification service is closed or busy"));
            }
            if page.worker.is_none() {
                page.worker = Some(start_worker()?);
            }
            page.next += 1;
            let token = page.next;
            page.worker
                .as_ref()
                .unwrap()
                .try_send((token, request))
                .map_err(|e| Error::operation(e.to_string()))?;
            cx.protect(resolve);
            cx.protect(reject);
            page.pending.insert(token, (resolve, reject));
            Ok(())
        })
        .map_err(|e| e.value(cx))
    });
    if let Err(reason) = queued {
        cx.call(reject, Value::undefined(), &[reason])?;
    }
    Ok(promise)
}

fn settle(cx: &Context, token: u64, output: Output) {
    let Some((resolve, reject)) = PAGE.with(|p| p.borrow_mut().pending.remove(&token)) else {
        return;
    };
    let (callback, value) = match output {
        Ok(Some(json)) => (
            resolve,
            Value(unsafe { sk_runtime::host::to_js(cx.raw(), &json) }),
        ),
        Ok(None) => (resolve, Value::undefined()),
        Err(error) => (reject, error.value(cx)),
    };
    let _ = cx.call(callback, Value::undefined(), &[value]);
    cx.unprotect(resolve);
    cx.unprotect(reject);
}

fn set_handler(cx: &Context, handler: Value) -> js::Result<Value> {
    if !handler.is_null() && cx.kind(handler) != 9 {
        return Err(js::Error::Exception(
            cx.error(1, "Expected an activation function or null"),
        ));
    }
    if PAGE.with(|p| p.borrow().closed) {
        return Ok(Value::undefined());
    }
    let next = (!handler.is_null()).then(|| {
        cx.protect(handler);
        handler
    });
    let old = PAGE.with(|p| std::mem::replace(&mut p.borrow_mut().handler, next));
    if let Some(old) = old {
        cx.unprotect(old);
    }
    wake();
    Ok(Value::undefined())
}

fn active(g: *mut c_void) -> bool {
    PUMP.get().is_some_and(|(owner, _)| *owner == g as usize) && PAGE.with(|p| !p.borrow().closed)
}
unsafe fn ready(g: *mut c_void) -> bool {
    if !active(g) {
        return false;
    }
    let inbox = INBOX.lock().unwrap();
    inbox.focus
        || !inbox.results.is_empty()
        || (!inbox.events.is_empty() && PAGE.with(|p| p.borrow().handler.is_some()))
}
unsafe fn drain(g: *mut c_void) -> usize {
    if !unsafe { ready(g) } {
        return 0;
    }
    let handler = PAGE.with(|p| p.borrow().handler);
    let (focus, token, result, event) = {
        let mut inbox = INBOX.lock().unwrap();
        let result = inbox.results.pop_front();
        let event = handler.and_then(|_| inbox.events.pop_front());
        (
            std::mem::take(&mut inbox.focus),
            inbox.focus_token.take(),
            result,
            event,
        )
    };
    if focus && !crate::quiet::never_focus() {
        #[cfg(target_os = "linux")]
        match token {
            Some(token) => crate::window::activate_with_token(token),
            None => crate::window::request_activate(),
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = token;
            crate::window::request_activate();
        }
    }
    JsRuntime::with(|rt| {
        let cx = rt.ctx();
        // Settling a promise or invoking a handler may re-enter the app.
        if let (Some(handler), Some(event)) = (handler, event) {
            let mut roots = Roots::new(cx);
            roots.keep(handler);
            let payload = match event {
                Activation::Button(id, action) => {
                    json!({ "kind": "button", "id": id, "actionId": action })
                }
                Activation::Open(id) => json!({ "kind": "open", "id": id }),
                Activation::Reply(id, action, text) => {
                    json!({ "kind": "reply", "id": id, "actionId": action, "text": text })
                }
            };
            let value = roots.keep(Value(unsafe { sk_runtime::host::to_js(g, &payload) }));
            if let Err(error) = cx.call(handler, Value::undefined(), &[value]) {
                sk_tree::log::record(
                    "error",
                    format!(
                        "Notification handler: {}",
                        cx.error_info(exception(cx, error)).0
                    ),
                );
            }
        }
        if let Some((token, output)) = result {
            settle(cx, token, output);
        }
    });
    1
}

fn cleanup(shutdown: bool) {
    let (handler, pending) = PAGE.with(|p| {
        let mut page = p.borrow_mut();
        // Rejection callbacks may try to resubscribe the outgoing page.
        page.closed = true;
        if shutdown {
            page.worker.take();
        }
        (page.handler.take(), std::mem::take(&mut page.pending))
    });
    JsRuntime::with(|rt| {
        rt.locked(|| {
            let cx = rt.ctx();
            if let Some(old) = handler {
                cx.unprotect(old);
            }
            if pending.is_empty() {
                return;
            }
            for (_, (resolve, reject)) in pending {
                let reason = Error {
                    name: "AbortError",
                    message: "Notification context closed".into(),
                };
                let _ = cx.call(reject, Value::undefined(), &[reason.value(cx)]);
                cx.unprotect(resolve);
                cx.unprotect(reject);
            }
            cx.drain_microtasks();
        })
    });
    if !shutdown {
        PAGE.with(|p| p.borrow_mut().closed = false);
    }
}

pub(crate) fn install(rt: &JsRuntime) -> Result<(), String> {
    let cx = rt.ctx();
    let _ = PUMP.set((cx.raw() as usize, cx.id()));
    web::register_pump_source(web::PumpSource {
        inflight: ready,
        ready,
        drain,
    });
    let install_methods = || -> js::Result<()> {
        let mut roots = Roots::new(cx);
        let namespace = roots.keep(cx.object());
        let mut define = |name: &str, method| cx.set(namespace, &cx.atom(name), roots.keep(method));
        define(
            "capabilities",
            js::function(cx, "capabilities", 0, |cx, _| {
                submit(cx, Ok(Request::Capabilities))
            })?,
        )?;
        define(
            "show",
            js::function(cx, "show", 1, |cx, a| {
                submit(cx, options::show(cx, a.arg(0)))
            })?,
        )?;
        define(
            "close",
            js::function(cx, "close", 1, |cx, a| {
                submit(cx, options::close(cx, a.arg(0)))
            })?,
        )?;
        define(
            "setActivationHandler",
            js::function(cx, "setActivationHandler", 1, |cx, a| {
                set_handler(cx, a.arg(0))
            })?,
        )?;
        cx.global_set("__sk_appNotifications", namespace)
    };
    install_methods().map_err(|e| e.to_string())?;
    js::on_reload(|_| cleanup(false));
    js::on_shutdown(|_| cleanup(true));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_api_rejects_bad_input_and_preserves_activations_across_reload() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                install(rt).unwrap();
                let cx = rt.ctx();
                cx.eval(r#"(() => {
                globalThis.notificationEvents = [];
                globalThis.notificationFailures = [];
                const n = __sk_appNotifications;
                const bad = [
                    n.show({title:"  "}),
                    n.show({title:"A",actions:[{kind:"reply",id:"r",label:"R"},{kind:"button",id:"r",label:"B"}]}),
                    n.show({title:"A",image:{png:"x"}}),
                    n.close(""),
                ];
                for (const p of bad) p.catch(e => notificationFailures.push(e.name));
                n.setActivationHandler(event => notificationEvents.push(event.id));
            })()"#).unwrap();
                cx.drain_microtasks();
                assert_eq!(
                    cx.read_string(cx.eval("notificationFailures.join(',')").unwrap())
                        .unwrap(),
                    "TypeError,TypeError,TypeError,TypeError"
                );
                assert!(cx
                    .eval("__sk_appNotifications.setActivationHandler({})")
                    .is_err());
                activate("old".into(), Some("read".into()));
                let (worker, requests) = mpsc::sync_channel(1);
                let previous = PAGE.with(|p| p.borrow_mut().worker.replace(worker));
                cx.eval("__sk_appNotifications.capabilities().catch(e => { notificationFailures.push(e.name); __sk_appNotifications.setActivationHandler(() => notificationEvents.push('stale')); })").unwrap();
                assert!(matches!(
                    requests.try_recv(),
                    Ok((_, Request::Capabilities))
                ));
                PAGE.with(|p| p.borrow_mut().worker = previous);
                cleanup(false);
                cx.drain_microtasks();
                assert!(PAGE.with(|p| p.borrow().handler.is_none()));
                assert_eq!(
                    cx.read_string(cx.eval("notificationFailures.at(-1)").unwrap())
                        .unwrap(),
                    "AbortError"
                );
                reply("new".into(), "reply".into(), "hello".into());
                cx.eval("__sk_appNotifications.setActivationHandler(e => notificationEvents.push([e.kind,e.id,e.actionId,e.text].join('/')))").unwrap();
                unsafe {
                    drain(cx.raw());
                    drain(cx.raw());
                }
                assert_eq!(
                    cx.read_string(cx.eval("notificationEvents.join(',')").unwrap())
                        .unwrap(),
                    "button/old/read/,reply/new/reply/hello"
                );
                cleanup(false);
            })
        });
    }

    #[test]
    fn logical_ids_are_stable_and_fit_native_limits() {
        let key = native_id("conversation/123");
        assert_eq!(key.len(), 32);
        assert_eq!(key, native_id("conversation/123"));
        assert_ne!(key, native_id("conversation/124"));
    }
}
