//! WebView2, composited under the app's surface.
//!
//! One browser environment per process, made when the first `<iframe>` needs it; one webview per
//! `<iframe>`, in visual hosting mode (`ICoreWebView2CompositionController`): it draws into a
//! DirectComposition visual placed in the container gpui keeps under the app's surface
//! (`Window::vn_composition_underlay`). The app paints transparent over the iframe's box, so the
//! document shows only there, clipped as the box is.
//!
//! Visual hosting has no window to take the mouse, so the element forwards pointer events
//! (`Frame::pointer`) and the webview gets `SendMouseInput`. The keyboard is a real focus move
//! (`MoveFocus`).
//!
//! Everything runs on the UI thread. WebView2 calls into the browser process are asynchronous, so a
//! frame from [`WebView2::create`] is usable at once and queues what it is told until its webview
//! exists.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use vn_tree::iframe::{Backend, Frame, FrameAttrs, FrameEvent, Placement, PointerInput, PointerKind, Sink};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2CompositionControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    CursorChangedEventHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, PermissionRequestedEventHandler,
    ProcessFailedEventHandler, WebMessageReceivedEventHandler, WebResourceRequestedEventHandler,
};
use windows::core::{Interface, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::DirectComposition::{IDCompositionDevice, IDCompositionVisual};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};

use crate::{content_type, external_origin, internal_origin, internal_url, Config, APP_ORIGIN, RELAY_PATH};

const RELAY_HTML: &str = include_str!("relay.html");

/// Takes a string WebView2 allocated for an out parameter.
unsafe fn take(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    let out = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    out
}

fn log(text: impl AsRef<str>) {
    vn_tree::log::record("warn", format!("<iframe> webview: {}", text.as_ref()));
}

enum Env {
    None,
    /// Being created; these run when it is.
    Creating(Vec<Box<dyn FnOnce(Option<&ICoreWebView2Environment>)>>),
    Ready(ICoreWebView2Environment),
    Failed,
}

pub(crate) struct WebView2 {
    config: Rc<Config>,
    env: Rc<RefCell<Env>>,
    /// Is the runtime installed? Asked once; an install needs a relaunch anyway.
    installed: Cell<Option<bool>>,
}

impl WebView2 {
    pub(crate) fn new(config: Config) -> Rc<dyn Backend> {
        Rc::new(Self { config: Rc::new(config), env: Rc::new(RefCell::new(Env::None)), installed: Cell::new(None) })
    }

    /// Runs `then` with the environment, creating it first if this is the
    /// first frame of the process.
    fn with_env(&self, then: Box<dyn FnOnce(Option<&ICoreWebView2Environment>)>) {
        let mut env = self.env.borrow_mut();
        match &mut *env {
            Env::Ready(ready) => {
                let ready = ready.clone();
                drop(env);
                then(Some(&ready));
            }
            Env::Failed => {
                drop(env);
                then(None);
            }
            Env::Creating(waiting) => waiting.push(then),
            Env::None => {
                *env = Env::Creating(vec![then]);
                drop(env);
                self.create_env();
            }
        }
    }

    fn create_env(&self) {
        let slot = self.env.clone();
        let settle = move |made: Option<ICoreWebView2Environment>| {
            let waiting = match std::mem::replace(&mut *slot.borrow_mut(), match &made {
                Some(env) => Env::Ready(env.clone()),
                None => Env::Failed,
            }) {
                Env::Creating(waiting) => waiting,
                _ => Vec::new(),
            };
            for then in waiting {
                then(made.as_ref());
            }
        };
        let _ = std::fs::create_dir_all(&self.config.user_data);
        let folder = HSTRING::from(self.config.user_data.as_os_str());
        let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();
        let settle = Rc::new(RefCell::new(Some(settle)));
        let done = settle.clone();
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
            if let Err(e) = &result {
                log(format!("the browser environment could not be created: {e}"));
            }
            if let Some(settle) = done.borrow_mut().take() {
                settle(result.ok().and(env));
            }
            Ok(())
        }));
        let started =
            unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &folder, &options, &handler) };
        if let Err(e) = started {
            log(format!("the browser environment could not be started: {e}"));
            if let Some(settle) = settle.borrow_mut().take() {
                settle(None);
            }
        }
    }
}

impl Backend for WebView2 {
    fn supported(&self) -> bool {
        // The switch a test (or a field diagnosis) turns the backend off with.
        if std::env::var("VN_IFRAME").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
            return false;
        }
        if let Some(known) = self.installed.get() {
            return known;
        }
        let mut version = PWSTR::null();
        let found = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) }.is_ok();
        let version = unsafe { take(version) };
        let installed = found && !version.is_empty();
        if !installed {
            log("the WebView2 runtime is not installed; <iframe> cannot show a document on this machine");
        }
        self.installed.set(Some(installed));
        installed
    }

    fn create(&self, node: u32, sink: Sink, window: &mut gpui::Window, _cx: &mut gpui::App) -> Option<Box<dyn Frame>> {
        let hwnd = hwnd_of(window)?;
        let Some((device, underlay)) = window.vn_composition_underlay() else {
            log("this window does not present through DirectComposition, so there is nowhere to draw a document");
            return None;
        };
        // Owned pointers (gpui's `windows` crate may be another version of this one).
        let device = unsafe { IDCompositionDevice::from_raw(device as *mut _) };
        let underlay = unsafe { IDCompositionVisual::from_raw(underlay as *mut _) };
        let inner = Rc::new(RefCell::new(Inner {
            node,
            hwnd,
            config: self.config.clone(),
            sink,
            device,
            underlay,
            visual: None,
            controller: None,
            composition: None,
            webview: None,
            relay_ready: false,
            queued: Vec::new(),
            placement: None,
            shown: false,
            want_focus: false,
        }));
        let weak = Rc::downgrade(&inner);
        self.with_env(Box::new(move |env| {
            let Some(inner) = weak.upgrade() else { return };
            let Some(env) = env else {
                inner.borrow().sink.send(FrameEvent::Failed("the browser environment is not available".into()));
                return;
            };
            if let Err(e) = start(&inner, env) {
                inner.borrow().sink.send(FrameEvent::Failed(format!("the webview could not be created: {e}")));
            }
        }));
        Some(Box::new(Handle(inner)))
    }
}

fn hwnd_of(window: &gpui::Window) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

struct Inner {
    node: u32,
    hwnd: HWND,
    config: Rc<Config>,
    sink: Sink,
    device: IDCompositionDevice,
    underlay: IDCompositionVisual,
    /// The webview's root visual, a child of `underlay`.
    visual: Option<IDCompositionVisual>,
    controller: Option<ICoreWebView2Controller>,
    composition: Option<ICoreWebView2CompositionController>,
    webview: Option<ICoreWebView2>,
    /// Has the relay page said it is listening?
    relay_ready: bool,
    /// Messages for the relay page, held until it is.
    queued: Vec<String>,
    placement: Option<Placement>,
    /// Is the controller currently visible?
    shown: bool,
    want_focus: bool,
}

/// Asks the environment for a webview; the rest happens in [`attach`].
fn start(inner: &Rc<RefCell<Inner>>, env: &ICoreWebView2Environment) -> windows::core::Result<()> {
    let env3: ICoreWebView2Environment3 = env.cast()?;
    let weak = Rc::downgrade(inner);
    let env = env.clone();
    let handler = CreateCoreWebView2CompositionControllerCompletedHandler::create(Box::new(move |result, made| {
        let why = format!("no webview: {result:?}");
        let Some(composition) = result.ok().and(made) else {
            if let Some(inner) = weak.upgrade() {
                inner.borrow().sink.send(FrameEvent::Failed(why));
            }
            return Ok(());
        };
        let controller: ICoreWebView2Controller = composition.cast()?;
        let Some(inner) = weak.upgrade() else {
            // The element went away while the browser was starting.
            let _ = unsafe { controller.Close() };
            return Ok(());
        };
        if let Err(e) = attach(&inner, &env, composition, controller) {
            inner.borrow().sink.send(FrameEvent::Failed(format!("the webview could not be set up: {e}")));
        }
        Ok(())
    }));
    let hwnd = inner.borrow().hwnd;
    unsafe { env3.CreateCoreWebView2CompositionController(hwnd, &handler) }
}

fn attach(
    inner: &Rc<RefCell<Inner>>,
    env: &ICoreWebView2Environment,
    composition: ICoreWebView2CompositionController,
    controller: ICoreWebView2Controller,
) -> windows::core::Result<()> {
    let webview = unsafe { controller.CoreWebView2()? };
    let config = inner.borrow().config.clone();
    unsafe {
        let settings = webview.Settings()?;
        settings.SetAreDevToolsEnabled(config.devtools)?;
        settings.SetAreDefaultContextMenusEnabled(config.devtools)?;
        settings.SetIsStatusBarEnabled(false)?;
        settings.SetIsZoomControlEnabled(false)?;
        settings.SetAreDefaultScriptDialogsEnabled(false)?;
        // A document, not a browser: no reload, find, print or zoom keys, no
        // autofill, no swipe to navigate.
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings3>() {
            settings.SetAreBrowserAcceleratorKeysEnabled(config.devtools)?;
        }
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings4>() {
            settings.SetIsPasswordAutosaveEnabled(false)?;
            settings.SetIsGeneralAutofillEnabled(false)?;
        }
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings5>() {
            settings.SetIsPinchZoomEnabled(false)?;
        }
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings6>() {
            settings.SetIsSwipeNavigationEnabled(false)?;
        }
        if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
            // Until the document paints, the box shows the element's own CSS
            // background: nothing of the webview's.
            controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 0, R: 0, G: 0, B: 0 })?;
        }
        if let Ok(controller3) = controller.cast::<ICoreWebView2Controller3>() {
            // The app says where the box is in device pixels and at what scale.
            controller3.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)?;
            controller3.SetShouldDetectMonitorScaleChanges(false)?;
        }
        if let Ok(controller4) = controller.cast::<ICoreWebView2Controller4>() {
            // Nothing is dropped into a document from outside.
            controller4.SetAllowExternalDrop(false)?;
        }
        controller.SetIsVisible(false)?;
    }

    let visual = unsafe {
        let me = inner.borrow();
        let visual = me.device.CreateVisual()?;
        composition.SetRootVisualTarget(&visual)?;
        me.underlay.AddVisual(&visual, true, None::<&IDCompositionVisual>)?;
        me.device.Commit()?;
        visual
    };

    let mut token = 0i64;
    let relay_url = format!("{APP_ORIGIN}{RELAY_PATH}");

    // While the document has the keyboard, the keys that are the window's stay
    // the window's: every Alt combination (Alt+F4, the system menu) and
    // the keys a PlayFrame host reserves. They are handed to the app's window
    // as the messages they would have been.
    unsafe {
        let weak = Rc::downgrade(inner);
        controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let (Some(args), Some(inner)) = (args, weak.upgrade()) else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND(0);
                args.KeyEventKind(&mut kind)?;
                let mut key = 0u32;
                args.VirtualKey(&mut key)?;
                let mut lparam = 0i32;
                args.KeyEventLParam(&mut lparam)?;
                // Escape leaves the app's fullscreen, whoever has the keyboard.
                let reserved = RESERVED_KEYS.contains(&key) || (key == VK_ESCAPE && vn_tree::iframe::fullscreen_active());
                let message = match kind {
                    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN => WM_SYSKEYDOWN,
                    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_UP => WM_SYSKEYUP,
                    COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN if reserved => WM_KEYDOWN,
                    COREWEBVIEW2_KEY_EVENT_KIND_KEY_UP if reserved => WM_KEYUP,
                    _ => return Ok(()),
                };
                let Ok(me) = inner.try_borrow() else { return Ok(()) };
                args.SetHandled(true)?;
                let _ = PostMessageW(Some(me.hwnd), message, WPARAM(key as usize), LPARAM(lparam as isize));
                Ok(())
            })),
            &mut token,
        )?;
    }

    unsafe {
        let sink = inner.borrow().sink.clone();
        composition.add_CursorChanged(
            &CursorChangedEventHandler::create(Box::new(move |composition, _| {
                let Some(composition) = composition else { return Ok(()) };
                let mut id = 0u32;
                composition.SystemCursorId(&mut id)?;
                sink.send(FrameEvent::Cursor(cursor_style(id)));
                Ok(())
            })),
            &mut token,
        )?;
    }

    // The app's own files and the relay page, from the package.
    unsafe {
        webview.AddWebResourceRequestedFilter(&HSTRING::from(format!("{APP_ORIGIN}/*")), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        let env = env.clone();
        let assets = config.assets.clone();
        webview.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                let uri = take(uri);
                let path = uri.strip_prefix(APP_ORIGIN).unwrap_or("/");
                let path_only = path.split(['?', '#']).next().unwrap_or(path);
                let body = if path_only == RELAY_PATH { Some(RELAY_HTML.as_bytes().to_vec()) } else { assets(path_only) };
                let response = match body {
                    Some(bytes) => {
                        let stream = SHCreateMemStream(Some(&bytes));
                        // A sandboxed document is an opaque origin: its own files are cross-origin to it.
                        let headers = format!(
                            "Content-Type: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store",
                            content_type(path_only)
                        );
                        env.CreateWebResourceResponse(stream.as_ref(), 200, &HSTRING::from("OK"), &HSTRING::from(headers))?
                    }
                    None => env.CreateWebResourceResponse(
                        None::<&windows::Win32::System::Com::IStream>,
                        404,
                        &HSTRING::from("Not Found"),
                        &HSTRING::from("Content-Type: text/plain"),
                    )?,
                };
                args.SetResponse(&response)
            })),
            &mut token,
        )?;
    }

    unsafe {
        let weak = Rc::downgrade(inner);
        let relay = relay_url.clone();
        webview.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let (Some(args), Some(inner)) = (args, weak.upgrade()) else { return Ok(()) };
                // Only the relay page itself: a frame inside it has
                // `chrome.webview` too, and must not speak as the relay.
                let mut source = PWSTR::null();
                args.Source(&mut source)?;
                if take(source) != relay {
                    return Ok(());
                }
                let mut json = PWSTR::null();
                args.WebMessageAsJson(&mut json)?;
                received(&inner, &take(json));
                Ok(())
            })),
            &mut token,
        )?;
    }

    // The iframe's navigation is the element's `load` / `error`.
    unsafe {
        let weak = Rc::downgrade(inner);
        webview.add_FrameNavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let (Some(args), Some(inner)) = (args, weak.upgrade()) else { return Ok(()) };
                let mut ok = windows::core::BOOL(0);
                args.IsSuccess(&mut ok)?;
                let event = if ok.as_bool() {
                    FrameEvent::Loaded
                } else {
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                    let _ = args.WebErrorStatus(&mut status);
                    FrameEvent::Failed(format!("the navigation failed (web error status {})", status.0))
                };
                inner.borrow().sink.send(event);
                Ok(())
            })),
            &mut token,
        )?;
    }

    // The top-level document is the relay page and stays the relay page.
    unsafe {
        let relay = relay_url.clone();
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Uri(&mut uri)?;
                if take(uri) != relay {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        // No prompt and no grant.
        webview.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let weak = Rc::downgrade(inner);
        webview.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                if let Some(inner) = weak.upgrade() {
                    inner.borrow().sink.send(FrameEvent::Failed("the browser process stopped".into()));
                }
                Ok(())
            })),
            &mut token,
        )?;
        // The relay page failing to load is the frame failing.
        let weak = Rc::downgrade(inner);
        webview.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let (Some(args), Some(inner)) = (args, weak.upgrade()) else { return Ok(()) };
                let mut ok = windows::core::BOOL(0);
                args.IsSuccess(&mut ok)?;
                if !ok.as_bool() {
                    inner.borrow().sink.send(FrameEvent::Failed("the relay page did not load".into()));
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.Navigate(&HSTRING::from(relay_url))?;
    }

    {
        let mut me = inner.borrow_mut();
        me.visual = Some(visual);
        me.controller = Some(controller);
        me.composition = Some(composition);
        me.webview = Some(webview);
    }
    let placement = inner.borrow().placement;
    inner.borrow_mut().apply_placement(placement);
    Ok(())
}

/// F5, F11, F12: never the document's (a PlayFrame host's `blockedKeys`).
const RESERVED_KEYS: [u32; 3] = [0x74, 0x7A, 0x7B];
const VK_ESCAPE: u32 = 0x1B;

/// A Win32 system cursor id (`IDC_*`) as the app's cursor. A cursor that is
/// not a system one (an image, `none`, `grab`) reports 0 and is the arrow.
fn cursor_style(id: u32) -> gpui::CursorStyle {
    use gpui::CursorStyle as C;
    match id {
        32513 => C::IBeam,
        32515 => C::Crosshair,
        32642 => C::ResizeUpLeftDownRight,
        32643 => C::ResizeUpRightDownLeft,
        32644 => C::ResizeLeftRight,
        32645 => C::ResizeUpDown,
        32646 => C::OpenHand,
        32648 => C::OperationNotAllowed,
        32649 => C::PointingHand,
        _ => C::Arrow,
    }
}

/// One message from the relay page, as JSON text.
fn received(inner: &Rc<RefCell<Inner>>, json: &str) {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(json) else { return };
    match message.get("t").and_then(|t| t.as_str()) {
        Some("ready") => {
            let mut me = inner.borrow_mut();
            me.relay_ready = true;
            let queued = std::mem::take(&mut me.queued);
            for text in queued {
                me.send(text);
            }
            if me.want_focus {
                me.focus(true);
            }
        }
        Some("msg") => {
            let me = inner.borrow();
            let origin = message.get("o").and_then(|o| o.as_str()).unwrap_or("null");
            let wire = message.get("w").map(|w| w.to_string()).unwrap_or_else(|| "null".into());
            me.sink.send(FrameEvent::Message { origin: external_origin(origin, &me.config.page_origin), wire });
        }
        _ => {}
    }
}

impl Inner {
    /// Posts to the relay page, or holds the message until it listens.
    fn send(&mut self, json: String) {
        match (&self.webview, self.relay_ready) {
            (Some(webview), true) => {
                if let Err(e) = unsafe { webview.PostWebMessageAsJson(&HSTRING::from(json)) } {
                    log(format!("a message to the document was not delivered: {e}"));
                }
            }
            _ => self.queued.push(json),
        }
    }

    fn apply_placement(&mut self, placement: Option<Placement>) {
        self.placement = placement;
        let (Some(controller), Some(visual)) = (&self.controller, &self.visual) else { return };
        let result: windows::core::Result<()> = (|| unsafe {
            match placement.filter(|p| p.clip[2] > 0.0 && p.clip[3] > 0.0) {
                None => {
                    if self.shown {
                        controller.SetIsVisible(false)?;
                        self.shown = false;
                    }
                }
                Some(p) => {
                    let [x, y, w, h] = p.bounds.map(f32::round);
                    if let Ok(controller3) = controller.cast::<ICoreWebView2Controller3>() {
                        controller3.SetRasterizationScale(f64::from(p.scale))?;
                    }
                    controller.SetBounds(RECT {
                        left: x as i32,
                        top: y as i32,
                        right: (x + w) as i32,
                        bottom: (y + h) as i32,
                    })?;
                    visual.SetOffsetX2(x)?;
                    visual.SetOffsetY2(y)?;
                    // In the visual's own coordinates.
                    let [cx, cy, cw, ch] = p.clip;
                    visual.SetClip2(&windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                        left: cx - x,
                        top: cy - y,
                        right: cx - x + cw,
                        bottom: cy - y + ch,
                    })?;
                    if !self.shown {
                        controller.SetIsVisible(true)?;
                        self.shown = true;
                    }
                }
            }
            self.device.Commit()
        })();
        if let Err(e) = result {
            log(format!("the document could not be placed: {e}"));
        }
    }

    /// Moves the document under another window's surface, or under this
    /// window's new compositor. The webview and its document stay.
    fn rehome(&mut self, window: &gpui::Window) {
        let (Some(hwnd), Some((device, underlay))) = (hwnd_of(window), window.vn_composition_underlay()) else {
            return;
        };
        // Owned: released here when nothing changed.
        let device = unsafe { IDCompositionDevice::from_raw(device as *mut _) };
        let underlay = unsafe { IDCompositionVisual::from_raw(underlay as *mut _) };
        if hwnd == self.hwnd && underlay.as_raw() == self.underlay.as_raw() {
            return;
        }
        let result: windows::core::Result<()> = (|| unsafe {
            if let Some(visual) = self.visual.take() {
                let _ = self.underlay.RemoveVisual(&visual);
                let _ = self.device.Commit();
            }
            let moved = hwnd != self.hwnd;
            self.hwnd = hwnd;
            self.device = device;
            self.underlay = underlay;
            let (Some(controller), Some(composition)) = (&self.controller, &self.composition) else {
                // Not made yet: `attach` uses what was just stored.
                return Ok(());
            };
            if moved {
                controller.SetParentWindow(hwnd)?;
            }
            // A visual belongs to the device that made it.
            let visual = self.device.CreateVisual()?;
            composition.SetRootVisualTarget(&visual)?;
            self.underlay.AddVisual(&visual, true, None::<&IDCompositionVisual>)?;
            self.device.Commit()?;
            self.visual = Some(visual);
            Ok(())
        })();
        if let Err(e) = result {
            log(format!("the document could not follow its element to another window: {e}"));
        }
        let placement = self.placement;
        self.apply_placement(placement);
    }

    fn focus(&mut self, on: bool) {
        self.want_focus = on;
        let Some(controller) = &self.controller else { return };
        unsafe {
            if on {
                if controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC).is_ok() {
                    self.send("{\"t\":\"focus\"}".into());
                }
            } else {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
    }
}

struct Handle(Rc<RefCell<Inner>>);

impl Frame for Handle {
    fn navigate(&mut self, attrs: &FrameAttrs) {
        let mut me = self.0.borrow_mut();
        let mut message = serde_json::Map::new();
        message.insert("t".into(), "nav".into());
        match (&attrs.srcdoc, &attrs.src) {
            (Some(doc), _) => {
                message.insert("srcdoc".into(), doc.clone().into());
            }
            (None, Some(src)) => {
                message.insert("src".into(), internal_url(src, &me.config.page_origin).into());
            }
            (None, None) => return,
        }
        for (key, value) in [
            ("sandbox", &attrs.sandbox),
            ("allow", &attrs.allow),
            ("referrerpolicy", &attrs.referrer_policy),
            ("name", &attrs.name),
        ] {
            if let Some(value) = value {
                message.insert(key.into(), value.clone().into());
            }
        }
        message.insert("fullscreen".into(), attrs.allow_fullscreen.into());
        // A navigation replaces whatever was still waiting for the old document.
        me.queued.retain(|m| !m.starts_with("{\"t\":\"post\""));
        me.send(serde_json::Value::Object(message).to_string());
    }

    fn post(&mut self, wire: &str, target_origin: &str) {
        let mut me = self.0.borrow_mut();
        let origin = serde_json::Value::String(internal_origin(target_origin, &me.config.page_origin));
        // `wire` is JSON text already; it is embedded, not re-encoded.
        me.send(format!("{{\"t\":\"post\",\"o\":{origin},\"w\":{wire}}}"));
    }

    fn place(&mut self, placement: Option<Placement>) {
        self.0.borrow_mut().apply_placement(placement);
    }

    fn rehome(&mut self, window: &mut gpui::Window) {
        self.0.borrow_mut().rehome(window);
    }

    fn focus(&mut self, on: bool) {
        self.0.borrow_mut().focus(on);
    }

    fn pointer(&mut self, input: PointerInput) {
        let me = self.0.borrow();
        let Some(composition) = &me.composition else { return };
        let (kind, data) = match input.kind {
            PointerKind::Move => (COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE, 0u32),
            PointerKind::Leave => (COREWEBVIEW2_MOUSE_EVENT_KIND_LEAVE, 0),
            PointerKind::Down(0) => (COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN, 0),
            PointerKind::Up(0) => (COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP, 0),
            PointerKind::Down(1) => (COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN, 0),
            PointerKind::Up(1) => (COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP, 0),
            PointerKind::Down(2) => (COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN, 0),
            PointerKind::Up(2) => (COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP, 0),
            PointerKind::Down(_) | PointerKind::Up(_) => return,
            // Win32 wheel units: 120 per notch, positive AWAY from the user, so
            // a scroll down is negative. One notch is taken as 100 px.
            PointerKind::Wheel(dx, dy) if dy != 0.0 || dx == 0.0 => {
                (COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL, ((-dy * 1.2).round() as i32) as u32)
            }
            PointerKind::Wheel(dx, _) => (COREWEBVIEW2_MOUSE_EVENT_KIND_HORIZONTAL_WHEEL, ((dx * 1.2).round() as i32) as u32),
        };
        let mut keys = COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE.0;
        for (bit, key) in [
            (1, COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_LEFT_BUTTON),
            (2, COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_RIGHT_BUTTON),
            (4, COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_MIDDLE_BUTTON),
        ] {
            if input.buttons & bit != 0 {
                keys |= key.0;
            }
        }
        if input.shift {
            keys |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_SHIFT.0;
        }
        if input.control {
            keys |= COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_CONTROL.0;
        }
        let mut point = POINT { x: input.x.round() as i32, y: input.y.round() as i32 };
        // A leave has no position and no keys: anything else is refused.
        if input.kind == PointerKind::Leave {
            point = POINT::default();
            keys = COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE.0;
        }
        let sent = unsafe {
            composition.SendMouseInput(kind, COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS(keys), data, point)
        };
        if let Err(e) = sent {
            log(format!("a pointer event ({:?}, buttons {}) was not delivered: {e}", input.kind, input.buttons));
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let me = self.0.borrow();
        unsafe {
            if let Some(visual) = &me.visual {
                let _ = me.underlay.RemoveVisual(visual);
                let _ = me.device.Commit();
            }
            // Ends the document and, with the last one, the browser processes.
            if let Some(controller) = &me.controller {
                let _ = controller.Close();
            }
        }
        let _ = me.node;
    }
}

// The weak handle a callback keeps must not keep the element's frame alive.
#[allow(dead_code)]
fn _weak_is_not_strong(w: Weak<RefCell<Inner>>) -> bool {
    w.upgrade().is_none()
}
