//! Native application tray, shared by Rust embedders and direct JavaScript methods.
//! Values and callbacks stay on the VM thread; the native pump carries only IDs.

use crate::js::{self, Context, JsDispatcher, Value};
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
pub use vn_desktop::tray::{Callback, Error, Icon, MenuItem, Options, Tray, WeakTray};
use vn_runtime::{redjs, JsRuntime};

type Result<T> = crate::js::Result<T>;
#[derive(Default)]
struct State {
    callbacks: Vec<Value>,
    tray: Option<Tray>,
    owned_revision: Option<u64>,
    epoch: Arc<AtomicU64>,
    icon_cache: Option<(String, Icon)>,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

pub(crate) struct Roots<'a> {
    context: &'a Context,
    values: Vec<Value>,
}
impl<'a> Roots<'a> {
    pub(crate) fn new(context: &'a Context) -> Self {
        Self {
            context,
            values: Vec::new(),
        }
    }
    pub(crate) fn keep(&mut self, value: Value) -> Value {
        if value.is_cell() {
            self.context.protect(value);
            self.values.push(value);
        }
        value
    }
    fn take(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.values)
    }
}
impl Drop for Roots<'_> {
    fn drop(&mut self) {
        release(self.context, std::mem::take(&mut self.values));
    }
}
fn release(context: &Context, values: Vec<Value>) {
    for value in values {
        context.unprotect(value);
    }
}
fn invalid(context: &Context, message: &str) -> crate::js::Error {
    crate::js::Error::Exception(context.error(1, message))
}
fn platform_error(context: &Context, error: Error) -> crate::js::Error {
    let value = context.error(0, &error.to_string());
    if error == Error::Unsupported {
        let _named = context.set(
            value,
            &context.atom("name"),
            context.string("NotSupportedError"),
        );
    }
    crate::js::Error::Exception(value)
}

fn generation_is_current(epoch: &AtomicU64, generation: u64) -> bool {
    if epoch.load(Ordering::Acquire) != generation {
        return false;
    }
    STATE.with(|state| {
        let state = state.borrow();
        // Rust can replace the shared application model without changing the
        // JS epoch. Retire its queued clicks, menu selections and Quit alike.
        state.owned_revision.is_none_or(|revision| {
            state
                .tray
                .as_ref()
                .is_some_and(|tray| tray.revision() == revision)
        })
    })
}
fn deliver(context: &Context, epoch: &AtomicU64, generation: u64, id: usize) {
    if !generation_is_current(epoch, generation) {
        return;
    }
    let callback = STATE.with(|state| state.borrow().callbacks.get(id).copied());
    let Some(callback) = callback else {
        return;
    };
    // A callback may replace the model, releasing every old root, while it runs.
    let mut root = Roots::new(context);
    root.keep(callback);
    if let Err(error) = context.call(callback, Value::undefined(), &[]) {
        let message = match error {
            crate::js::Error::Exception(value) => context.error_info(value).0,
            other => other.to_string(),
        };
        vn_tree::log::record("error", format!("tray callback: {message}"));
    }
}
fn callback(
    dispatcher: &JsDispatcher,
    epoch: &Arc<AtomicU64>,
    generation: u64,
    id: usize,
) -> Callback {
    let dispatcher = dispatcher.clone();
    let epoch = epoch.clone();
    Arc::new(move || {
        if epoch.load(Ordering::Acquire) != generation {
            return;
        }
        let event_epoch = epoch.clone();
        if let Err(error) =
            dispatcher.post(move |context| deliver(context, &event_epoch, generation, id))
        {
            eprintln!("[tray] event delivery: {error}");
        }
    })
}
struct Decoder<'a> {
    context: &'a Context,
    dispatcher: JsDispatcher,
    epoch: Arc<AtomicU64>,
    generation: u64,
    objects: Roots<'a>,
    callbacks: Roots<'a>,
    count: usize,
    icon_cache: Option<(String, Icon)>,
}
impl<'a> Decoder<'a> {
    fn new(
        context: &'a Context,
        dispatcher: JsDispatcher,
        epoch: Arc<AtomicU64>,
        generation: u64,
        icon_cache: Option<(String, Icon)>,
    ) -> Self {
        Self {
            context,
            dispatcher,
            epoch,
            generation,
            objects: Roots::new(context),
            callbacks: Roots::new(context),
            count: 0,
            icon_cache,
        }
    }
    fn get(&mut self, value: Value, name: &str) -> Result<Value> {
        let value = self.context.get(value, &self.context.atom(name))?;
        Ok(self.objects.keep(value))
    }
    fn object(&self, value: Value, name: &str) -> Result<()> {
        if self.context.kind(value) != 8 {
            return Err(invalid(self.context, &format!("{name} must be an object")));
        }
        Ok(())
    }
    fn string(&mut self, value: Value, key: &str) -> Result<Option<String>> {
        let value = self.get(value, key)?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        if self.context.kind(value) != 5 {
            return Err(invalid(self.context, &format!("{key} must be a string")));
        }
        self.context.read_string(value).map(Some)
    }
    fn boolean(&mut self, value: Value, key: &str) -> Result<Option<bool>> {
        let value = self.get(value, key)?;
        if value.is_undefined() {
            return Ok(None);
        }
        value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid(self.context, &format!("{key} must be a boolean")))
    }
    fn function(&mut self, value: Value, key: &str) -> Result<Option<Callback>> {
        let value = self.get(value, key)?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        if self.context.kind(value) != 9 {
            return Err(invalid(self.context, &format!("{key} must be a function")));
        }
        let id = self.callbacks.values.len();
        self.callbacks.keep(value);
        Ok(Some(callback(
            &self.dispatcher,
            &self.epoch,
            self.generation,
            id,
        )))
    }
    fn menu(&mut self, value: Value, depth: usize) -> Result<Vec<MenuItem>> {
        if value.is_undefined() || value.is_null() {
            return Ok(Vec::new());
        }
        if depth > 8 {
            return Err(invalid(self.context, "tray menu nesting exceeds 8"));
        }
        if self.context.kind(value) != 10 {
            return Err(invalid(self.context, "tray menu must be an array"));
        }
        let length = self.context.array_length(value)?;
        if length > 512 {
            return Err(invalid(self.context, "tray menu exceeds 512 items"));
        }
        let mut items = Vec::new();
        for index in 0..length {
            let item = self.context.get_index(value, index)?;
            self.objects.keep(item);
            if item.is_undefined() || item.is_null() || item.as_bool() == Some(false) {
                continue;
            }
            self.count += 1;
            if self.count > 512 {
                return Err(invalid(self.context, "tray menu exceeds 512 items"));
            }
            self.object(item, "tray menu item")?;
            let kind = self.string(item, "type")?;
            if let Some(kind) = kind.as_deref() {
                if kind != "separator" {
                    return Err(invalid(self.context, "tray menu type must be separator"));
                }
                items.push(MenuItem::Separator);
                continue;
            }
            let role = self.string(item, "role")?;
            let label = self.string(item, "label")?;
            let disabled = self.boolean(item, "disabled")?.unwrap_or(false);
            let checked = self.boolean(item, "checked")?;
            let default = self.boolean(item, "default")?.unwrap_or(false);
            let on_select = self.function(item, "onSelect")?;
            let submenu = self.get(item, "submenu")?;
            if let Some(role) = role.as_deref() {
                if role != "quit"
                    || on_select.is_some()
                    || !submenu.is_undefined()
                    || checked.is_some()
                {
                    return Err(invalid(
                        self.context,
                        "quit role cannot also have onSelect, submenu or checked",
                    ));
                }
                items.push(MenuItem::Quit { label, disabled });
            } else if !submenu.is_undefined() && !submenu.is_null() {
                if on_select.is_some() || checked.is_some() {
                    return Err(invalid(
                        self.context,
                        "submenu cannot also have onSelect or checked",
                    ));
                }
                let children = self.menu(submenu, depth + 1)?;
                items.push(MenuItem::Submenu {
                    label: label.unwrap_or_default(),
                    disabled,
                    items: children,
                });
            } else {
                items.push(MenuItem::Action {
                    label: label.unwrap_or_default(),
                    disabled,
                    checked,
                    default,
                    on_select,
                });
            }
        }
        Ok(items)
    }
    fn icon(&mut self, value: Value) -> Result<Option<Icon>> {
        if value.is_undefined() || value.is_null() {
            self.icon_cache = None;
            return Ok(None);
        }
        if self.context.kind(value) == 5 {
            let source = self.context.read_string(value)?;
            if let Some((key, icon)) = &self.icon_cache {
                if key == &source {
                    return Ok(Some(icon.clone()));
                }
            }
            if source.len() > 3 * 1024 * 1024 {
                return Err(invalid(self.context, "tray icon source exceeds 3 MiB"));
            }
            let bytes = if source.starts_with("data:") {
                // data: only: no synchronous network or filesystem work on the VM.
                let fetched = redjs::webapi::fsutil::fetch_url(self.context.raw(), &source);
                if !fetched.ok {
                    return Err(invalid(self.context, "invalid tray icon data URL"));
                }
                std::borrow::Cow::Owned(fetched.bytes)
            } else {
                let key = vn_payload::content_key(&source).ok_or_else(|| {
                    invalid(
                        self.context,
                        "tray icon must be a packaged asset, data URL or RGBA pixels",
                    )
                })?;
                std::borrow::Cow::Borrowed(crate::payload::lookup(&key).ok_or_else(|| invalid(self.context, "tray icon asset is not packaged; load it asynchronously and pass RGBA pixels in development"))?)
            };
            let bitmap =
                redjs::webapi::image::decode_thumbnail(&bytes, None, 128).ok_or_else(|| {
                    invalid(
                        self.context,
                        "tray icon cannot be decoded (2 MiB input, raster maximum 4096 per side)",
                    )
                })?;
            let mut icon = Icon::rgba(bitmap.width, bitmap.height, bitmap.rgba)
                .map_err(|error| platform_error(self.context, error))?;
            // An SVG used as a tray glyph follows the taskbar foreground colour.
            icon.template = source
                .split('?')
                .next()
                .is_some_and(|path| path.ends_with(".svg"))
                || source.starts_with("data:image/svg+xml");
            self.icon_cache = Some((source, icon.clone()));
            return Ok(Some(icon));
        }
        self.object(value, "tray icon")?;
        let width_value = self.get(value, "width")?;
        let height_value = self.get(value, "height")?;
        let dimension = |value: Value| {
            value
                .as_number()
                .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 1.0 && *n <= 512.0)
                .map(|n| n as u32)
        };
        let width = dimension(width_value).ok_or_else(|| {
            invalid(
                self.context,
                "tray icon width must be an integer from 1 to 512",
            )
        })?;
        let height = dimension(height_value).ok_or_else(|| {
            invalid(
                self.context,
                "tray icon height must be an integer from 1 to 512",
            )
        })?;
        let template = self.boolean(value, "template")?.unwrap_or(false);
        let data = self.get(value, "data")?;
        if self.context.kind(data) != 11 {
            return Err(invalid(
                self.context,
                "tray icon data must be Uint8Array or Uint8ClampedArray",
            ));
        }
        let info = self.context.typedarray_info(data)?;
        if ![2, 3].contains(&info.kind)
            || info.detached != 0
            || info.shared != 0
            || info.data.is_null()
            || info.byte_length as usize != width as usize * height as usize * 4
        {
            return Err(invalid(
                self.context,
                "tray icon data must contain exact, non-shared RGBA bytes",
            ));
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(info.data.cast::<u8>(), info.byte_length as usize)
        }
        .to_vec();
        self.icon_cache = None;
        let mut icon = Icon::rgba(width, height, bytes)
            .map_err(|error| platform_error(self.context, error))?;
        icon.template = template;
        Ok(Some(icon))
    }
    fn options(&mut self, value: Value) -> Result<Options> {
        self.objects.keep(value);
        self.object(value, "tray options")?;
        let tooltip = self.string(value, "tooltip")?.unwrap_or_default();
        let on_click = self.function(value, "onClick")?;
        let menu = self.get(value, "menu")?;
        let menu = self.menu(menu, 0)?;
        let source = self.get(value, "icon")?;
        let icon = self.icon(source)?;
        let dispatcher = self.dispatcher.clone();
        let epoch = self.epoch.clone();
        let generation = self.generation;
        let options = Options {
            icon,
            tooltip,
            menu,
            on_click,
            on_quit: Some(Arc::new(move || {
                if epoch.load(Ordering::Acquire) != generation {
                    return;
                }
                let epoch = epoch.clone();
                if let Err(error) = dispatcher.post(move |_| {
                    if generation_is_current(&epoch, generation) {
                        crate::window::request_close();
                    }
                }) {
                    eprintln!("[tray] quit delivery: {error}");
                }
            })),
        };
        options
            .validate()
            .map_err(|error| platform_error(self.context, error))?;
        Ok(options)
    }
}

fn set(context: &Context, dispatcher: &JsDispatcher, value: Value) -> Result<()> {
    let (epoch, previous, cache) = STATE.with(|state| {
        let state = state.borrow();
        (
            state.epoch.clone(),
            state.epoch.load(Ordering::Acquire),
            state.icon_cache.clone(),
        )
    });
    let generation = previous.wrapping_add(1);
    let remove = value.is_null();
    let mut decoder = Decoder::new(
        context,
        dispatcher.clone(),
        epoch.clone(),
        generation,
        cache,
    );
    let options = if remove {
        None
    } else {
        Some(decoder.options(value)?)
    };
    if epoch.load(Ordering::Acquire) != previous {
        return Err(invalid(
            context,
            "tray was replaced while reading its options",
        ));
    }
    let tray = if remove {
        Tray::existing_app()
    } else {
        Some(Tray::app().map_err(|error| platform_error(context, error))?)
    };
    let revision = tray
        .as_ref()
        .map(|tray| tray.set(options))
        .transpose()
        .map_err(|error| platform_error(context, error))?;
    epoch.store(generation, Ordering::Release);
    let old_callbacks = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.tray = tray;
        state.owned_revision = revision;
        state.icon_cache = if remove {
            None
        } else {
            decoder.icon_cache.take()
        };
        std::mem::replace(&mut state.callbacks, decoder.callbacks.take())
    });
    release(context, old_callbacks);
    Ok(())
}
fn cleanup(context: &Context, shutdown: bool) {
    let (tray, revision, callbacks) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.epoch.fetch_add(1, Ordering::AcqRel);
        state.icon_cache = None;
        (
            state.tray.take(),
            state.owned_revision.take(),
            std::mem::take(&mut state.callbacks),
        )
    });
    if let (Some(tray), Some(revision)) = (&tray, revision) {
        if tray.revision() == revision {
            if let Err(error) = tray.set(None) {
                eprintln!("[tray] removing JavaScript tray: {error}");
            }
        }
    }
    release(context, callbacks);
    if shutdown {
        if let Some(tray) = Tray::existing_app() {
            tray.stop();
        }
    }
    drop(tray);
}

pub(crate) fn install(runtime: &JsRuntime) -> std::result::Result<(), String> {
    let context = runtime.ctx();
    let dispatcher = JsDispatcher::install(context);
    let mut roots = Roots::new(context);
    let namespace = roots.keep(context.object());
    let method = js::function(context, "set", 1, move |context, args| {
        set(context, &dispatcher, args.arg(0))?;
        Ok(Value::undefined())
    })
    .map_err(|error| error.to_string())?;
    roots.keep(method);
    context
        .set(namespace, &context.atom("set"), method)
        .map_err(|error| error.to_string())?;
    context
        .set(
            namespace,
            &context.atom("available"),
            Value::boolean(Tray::is_available()),
        )
        .map_err(|error| error.to_string())?;
    context
        .global_set("__vn_appTray", namespace)
        .map_err(|error| error.to_string())?;
    js::on_reload(|context| cleanup(context, false));
    js::on_shutdown(|context| cleanup(context, true));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_callbacks_survive_gc_and_stale_events_are_fenced_without_native_windows() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let dispatcher = JsDispatcher::install(context);
                let epoch = Arc::new(AtomicU64::new(12));
                let mut decoder = Decoder::new(context, dispatcher, epoch.clone(), 12, None);
                let value = context.eval("globalThis.__trayCount = 0; ({tooltip:'Test',onClick:()=>__trayCount++,menu:[false,{label:'Muted',checked:true,onSelect:()=>__trayCount+=2},{label:'More',submenu:[{role:'quit'}]}]})").unwrap();
                let options = decoder.options(value).unwrap();
                assert_eq!(options.menu.len(), 2);
                assert!(matches!(
                    &options.menu[0],
                    MenuItem::Action {
                        checked: Some(true),
                        ..
                    }
                ));
                STATE.with(|state| state.borrow_mut().callbacks = decoder.callbacks.take());
                drop(decoder);
                context.gc();
                deliver(context, &epoch, 12, 0);
                deliver(context, &epoch, 12, 1);
                assert_eq!(
                    context.global_get("__trayCount").unwrap().as_number(),
                    Some(3.0)
                );
                epoch.store(13, Ordering::Release);
                deliver(context, &epoch, 12, 0);
                assert_eq!(
                    context.global_get("__trayCount").unwrap().as_number(),
                    Some(3.0)
                );
                cleanup(context, false);
                STATE.with(|state| assert!(state.borrow().callbacks.is_empty()));
            })
        });
    }
    #[test]
    fn callback_can_remove_its_own_model_without_a_root_or_registry_borrow() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let dispatcher = JsDispatcher::install(context);
                cleanup(context, false);
                let clear_dispatcher = dispatcher.clone();
                let clear = js::function(context, "__trayClear", 0, move |context, _| {
                    set(context, &clear_dispatcher, Value::null())?;
                    Ok(Value::undefined())
                })
                .unwrap();
                context.global_set("__trayClear", clear).unwrap();
                let epoch = STATE.with(|state| state.borrow().epoch.clone());
                let generation = epoch.load(Ordering::Acquire).wrapping_add(1);
                let mut decoder =
                    Decoder::new(context, dispatcher, epoch.clone(), generation, None);
                decoder.options(context.eval("globalThis.__traySelfRemoved = 0; ({onClick:()=>{ __trayClear(); __traySelfRemoved++; }})").unwrap()).unwrap();
                STATE.with(|state| state.borrow_mut().callbacks = decoder.callbacks.take());
                epoch.store(generation, Ordering::Release);
                drop(decoder);
                context.gc();
                deliver(context, &epoch, generation, 0);
                assert_eq!(
                    context.global_get("__traySelfRemoved").unwrap().as_number(),
                    Some(1.0)
                );
                STATE.with(|state| assert!(state.borrow().callbacks.is_empty()));
                deliver(context, &epoch, generation, 0);
                assert_eq!(
                    context.global_get("__traySelfRemoved").unwrap().as_number(),
                    Some(1.0)
                );
                context
                    .global_set("__trayClear", Value::undefined())
                    .unwrap();
                cleanup(context, false);
            })
        });
    }
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn queued_js_callback_is_retired_when_rust_replaces_or_stops_model() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                cleanup(context, false);
                let dispatcher = JsDispatcher::install(context);
                for stop in [false, true] {
                    let epoch = STATE.with(|state| state.borrow().epoch.clone());
                    let generation = epoch.load(Ordering::Acquire).wrapping_add(1);
                    let mut decoder =
                        Decoder::new(context, dispatcher.clone(), epoch.clone(), generation, None);
                    let options = decoder
                        .options(
                            context
                                .eval("globalThis.__trayQueued = 0; ({onClick:()=>__trayQueued++})")
                                .unwrap(),
                        )
                        .unwrap();
                    // A controller without an icon needs no Explorer (Windows)
                    // and no main queue (macOS creates its item there, lazily).
                    let tray = Tray::new().unwrap();
                    let revision = tray.set(None).unwrap();
                    STATE.with(|state| {
                        let mut state = state.borrow_mut();
                        state.callbacks = decoder.callbacks.take();
                        state.tray = Some(tray.clone());
                        state.owned_revision = Some(revision);
                    });
                    epoch.store(generation, Ordering::Release);
                    drop(decoder);
                    let native_callback = options.on_click.unwrap();
                    native_callback();
                    runtime.pump(vn_runtime::runtime::PUMP_TIMERS);
                    assert_eq!(
                        context.global_get("__trayQueued").unwrap().as_number(),
                        Some(1.0)
                    );
                    native_callback(); // queued to VM, but not drained yet
                    let rust_revision = if stop {
                        tray.stop();
                        tray.revision()
                    } else {
                        tray.set(None).unwrap()
                    };
                    assert!(
                        !generation_is_current(&epoch, generation),
                        "queued semantic Quit must use this same fence"
                    );
                    runtime.pump(vn_runtime::runtime::PUMP_TIMERS);
                    assert_eq!(
                        context.global_get("__trayQueued").unwrap().as_number(),
                        Some(1.0)
                    );
                    cleanup(context, false);
                    assert_eq!(
                        tray.revision(),
                        rust_revision,
                        "reload must preserve a newer Rust model"
                    );
                    tray.stop();
                    let stopped_revision = tray.revision();
                    tray.stop();
                    assert_eq!(tray.revision(), stopped_revision, "stop is idempotent");
                }
            })
        });
    }
    #[test]
    fn decoder_rejects_invalid_models_and_reads_only_exact_rgba_views() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let dispatcher = JsDispatcher::install(context);
                for source in [
                    "({menu:{}})",
                    "({menu:[{label:'x',onSelect:1}]})",
                    "({menu:[{role:'quit',onSelect:()=>{}}]})",
                    "({menu:[{label:''}]})",
                    "({tooltip:'x'.repeat(128)})",
                    "({icon:{width:1,height:1,data:new Uint8Array(3)}})",
                ] {
                    let mut decoder = Decoder::new(
                        context,
                        dispatcher.clone(),
                        Arc::new(AtomicU64::new(0)),
                        1,
                        None,
                    );
                    assert!(
                        decoder.options(context.eval(source).unwrap()).is_err(),
                        "{source}"
                    );
                }
                let mut decoder =
                    Decoder::new(context, dispatcher, Arc::new(AtomicU64::new(0)), 1, None);
                let options = decoder.options(context.eval("({icon:{width:1,height:1,data:new Uint8Array([9,255,0,0,128,9]).subarray(1,5)}})").unwrap()).unwrap();
                assert_eq!(options.icon.unwrap().rgba.as_ref(), &[255, 0, 0, 128]);
            })
        });
    }
}
