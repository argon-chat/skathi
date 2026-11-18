//! WKWebView, composited under the app's Metal layer.
//!
//! One webview per `<iframe>`, in a view placed BELOW the window's gpui view: a flipped host view the
//! size of the gpui view holds one clip view per document (the visible part of the box) with the
//! webview inside it at the box. The gpui layer is made non-opaque and the app paints transparent over
//! the iframe's box, so the document shows only there. The gpui view stays on top and gets every mouse
//! event; the element forwards the ones its hit test gives the iframe (`Frame::pointer`) to the
//! webview, which hears no mouse otherwise. The keyboard is a real first-responder move, so IME and
//! the document's own key handling are WebKit's.
//!
//! Everything runs on the main thread.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::{DynBlock, RcBlock};
use objc2::rc::{Retained, Weak as ObjcWeak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    define_class, msg_send, sel, AllocAnyThread, ClassType, DefinedClass, MainThreadMarker,
    MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSColor, NSEvent, NSEventModifierFlags, NSEventType,
    NSModalResponse, NSOpenPanel, NSResponder, NSView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{
    ns_string, NSArray, NSData, NSDictionary, NSError, NSHTTPURLResponse, NSNumber, NSPoint,
    NSProcessInfo, NSRect, NSSize, NSString, NSURLRequest, NSURL,
};
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction,
    WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKOpenPanelParameters, WKPermissionDecision, WKScriptMessage,
    WKScriptMessageHandler, WKSecurityOrigin, WKUIDelegate, WKURLSchemeHandler, WKURLSchemeTask,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration, WKWindowFeatures,
};
use sk_tree::iframe::{
    Backend, Frame, FrameAttrs, FrameEvent, Placement, PointerInput, PointerKind,
};

use crate::{
    content_type, external_origin, internal_origin, internal_url, Config, APP_ORIGIN, RELAY_PATH,
};

const RELAY_HTML: &str = include_str!("relay.html");
const CURSOR_JS: &str = include_str!("cursor.js");
const NO_MENU_JS: &str = "addEventListener(\"contextmenu\", (e) => e.preventDefault(), true);";
const SCHEME: &str = "sk-app";
const NSURL_ERROR_CANCELLED: isize = -999;

type Emit = Rc<dyn Fn(FrameEvent)>;

fn log(text: impl AsRef<str>) {
    sk_tree::log::record("warn", format!("<iframe> webview: {}", text.as_ref()));
}

pub(crate) struct WebKit {
    config: Rc<Config>,
}

impl WebKit {
    pub(crate) fn new(config: Config) -> Rc<dyn Backend> {
        Rc::new(Self {
            config: Rc::new(config),
        })
    }
}

impl Backend for WebKit {
    fn supported(&self) -> bool {
        !std::env::var("SK_IFRAME").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
    }

    fn create(
        &self,
        _node: u32,
        sink: sk_tree::iframe::Sink,
        window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Option<Box<dyn Frame>> {
        let app = app_view(window)?;
        let emit: Emit = Rc::new(move |event| sink.send(event));
        open(&app, self.config.clone(), emit).map(|h| Box::new(h) as Box<dyn Frame>)
    }
}

/// A document in a webview under `view`, an `NSView` in a window, outside gpui: for tests.
pub fn frame_in_view(view: NonNull<c_void>, config: Config, emit: Emit) -> Option<Box<dyn Frame>> {
    // SAFETY: the caller passes a live NSView; retaining it keeps it so for the call.
    let app = unsafe { Retained::retain(view.as_ptr().cast::<NSView>()) }?;
    open(&app, Rc::new(config), emit).map(|h| Box::new(h) as Box<dyn Frame>)
}

fn app_view(window: &gpui::Window) -> Option<Retained<NSView>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        // SAFETY: gpui's AppKit handle is its live content NSView, used on the main thread.
        RawWindowHandle::AppKit(h) => unsafe { Retained::retain(h.ns_view.as_ptr().cast()) },
        _ => None,
    }
}

fn responder(view: &NSView) -> &NSResponder {
    view
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; Flipped adds no ivars and no Drop.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SkathiFrameView"]
    struct Flipped;

    impl Flipped {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // The gpui view above takes the mouse; nothing here is ever the target of a click.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

impl Flipped {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSView> {
        // SAFETY: `initWithFrame:` is NSView's designated initializer.
        let view: Retained<Flipped> =
            unsafe { msg_send![Flipped::alloc(mtm), initWithFrame: frame] };
        view.into_super()
    }
}

struct KeyStopIvars {
    forward: bool,
    emit: Emit,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; the ivars need no special drop order.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SkathiFrameKeyStop"]
    #[ivars = KeyStopIvars]
    struct KeyStop;

    // WebKit hands the focus on with `selectKeyViewFollowingView:` / `...PrecedingView:` when Tab
    // leaves the document's first or last element: the webview's key-view neighbours are these.
    impl KeyStop {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            (self.ivars().emit)(FrameEvent::FocusOut {
                forward: self.ivars().forward,
            });
            true
        }
    }
);

impl KeyStop {
    fn new(mtm: MainThreadMarker, forward: bool, emit: Emit) -> Retained<NSView> {
        let this = KeyStop::alloc(mtm).set_ivars(KeyStopIvars { forward, emit });
        // SAFETY: `initWithFrame:` is NSView's designated initializer.
        let view: Retained<KeyStop> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        view.into_super()
    }
}

struct WebIvars {
    app: RefCell<ObjcWeak<NSView>>,
    forwarding: Cell<bool>,
}

define_class!(
    // SAFETY: WKWebView may be subclassed; the overrides call through to it.
    #[unsafe(super(WKWebView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SkathiFrameWebView"]
    #[ivars = WebIvars]
    struct SkWebView;

    // WebKit's own tracking area would give the document every move over its box, under the app's
    // UI too; only the moves the app forwards count.
    impl SkWebView {
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            if self.ivars().forwarding.get() {
                // SAFETY: forwarding the event to WKWebView's own handler.
                unsafe { msg_send![super(self), mouseMoved: event] }
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            if self.ivars().forwarding.get() {
                // SAFETY: as above.
                unsafe { msg_send![super(self), mouseEntered: event] }
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            if self.ivars().forwarding.get() {
                // SAFETY: as above.
                unsafe { msg_send![super(self), mouseExited: event] }
            }
        }

        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, _event: &NSEvent) {}

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.to_app(event, true) {
                // SAFETY: as above.
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            if !self.to_app(event, false) {
                // SAFETY: as above.
                unsafe { msg_send![super(self), keyUp: event] }
            }
        }
    }
);

/// F5, F11 and F12, by virtual key code.
const RESERVED_KEYS: [u16; 3] = [0x60, 0x67, 0x6F];
const KEY_ESCAPE: u16 = 0x35;

impl SkWebView {
    fn new(mtm: MainThreadMarker, config: &WKWebViewConfiguration, app: &NSView) -> Retained<Self> {
        let this = SkWebView::alloc(mtm).set_ivars(WebIvars {
            app: RefCell::new(ObjcWeak::from_retained(&app.retain())),
            forwarding: Cell::new(false),
        });
        // SAFETY: `initWithFrame:configuration:` is WKWebView's designated initializer.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO, configuration: config] }
    }

    /// The keys a host keeps while a document has the keyboard go to the app's view instead.
    fn to_app(&self, event: &NSEvent, down: bool) -> bool {
        let code = event.keyCode();
        let reserved = RESERVED_KEYS.contains(&code)
            || (code == KEY_ESCAPE && sk_tree::iframe::fullscreen_active());
        if !reserved {
            return false;
        }
        let Some(app) = self.ivars().app.borrow().load() else {
            return false;
        };
        // SAFETY: the gpui view handles key events as the first responder would.
        unsafe {
            if down {
                let _: () = msg_send![&*app, keyDown: event];
            } else {
                let _: () = msg_send![&*app, keyUp: event];
            }
        }
        true
    }

    fn forward(&self, send: impl FnOnce(&Self)) {
        self.ivars().forwarding.set(true);
        send(self);
        self.ivars().forwarding.set(false);
    }
}

struct DelegateIvars {
    inner: Weak<RefCell<Inner>>,
    assets: Rc<dyn Fn(&str) -> Option<Vec<u8>>>,
    /// The app's URLs answered 404, until a navigation response is checked against them.
    missing: RefCell<HashSet<String>>,
    relay: String,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the ivars need no special drop order.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SkathiFrameDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            _web: &WKWebView,
            action: &WKNavigationAction,
            handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let allow = self.decide_action_for(action);
            handler.call((if allow {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _web: &WKWebView,
            response: &WKNavigationResponse,
            handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            // SAFETY: plain property reads on the response WebKit passes.
            let (main, shown) = unsafe { (response.isForMainFrame(), response.canShowMIMEType()) };
            if !main {
                self.child_response(response);
            }
            handler.call((if shown {
                WKNavigationResponsePolicy::Allow
            } else {
                WKNavigationResponsePolicy::Cancel
            },));
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_provisional(&self, _web: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            self.relay_failed(error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, _web: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            self.relay_failed(error);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _web: &WKWebView) {
            self.emit(FrameEvent::Failed("the browser process stopped".into()));
        }
    }

    unsafe impl WKUIDelegate for Delegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _web: &WKWebView,
            _config: &WKWebViewConfiguration,
            _action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            None
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(
            &self,
            _web: &WKWebView,
            _message: &NSString,
            _frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn()>,
        ) {
            handler.call(());
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(
            &self,
            _web: &WKWebView,
            _message: &NSString,
            _frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(objc2::runtime::Bool)>,
        ) {
            handler.call((objc2::runtime::Bool::NO,));
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(
            &self,
            _web: &WKWebView,
            _prompt: &NSString,
            _default: Option<&NSString>,
            _frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_capture(
            &self,
            _web: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            handler: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            handler.call((WKPermissionDecision::Deny,));
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(
            &self,
            web: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            open_files(web, parameters, handler.copy());
        }
    }

    unsafe impl WKScriptMessageHandler for Delegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn script_message(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            self.on_script_message(message);
        }
    }

    unsafe impl WKURLSchemeHandler for Delegate {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start_task(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.serve(task);
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop_task(&self, _web: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }

    impl Delegate {
        // WebKit's only report of a subframe that never loads (no network, no host): the iframe gets
        // neither `load` nor `error`. An optional delegate method, called when it is implemented.
        #[unsafe(method(_webView:didFailProvisionalLoadWithRequest:inFrame:withError:))]
        fn subframe_failed(
            &self,
            _web: &WKWebView,
            _request: &NSURLRequest,
            frame: &WKFrameInfo,
            error: &NSError,
        ) {
            // SAFETY: a plain property read.
            if unsafe { frame.isMainFrame() } || error.code() == NSURL_ERROR_CANCELLED {
                return;
            }
            let Some(inner) = self.ivars().inner.upgrade() else { return };
            let Ok(mut me) = inner.try_borrow_mut() else { return };
            if std::mem::take(&mut me.expect_child) {
                let why = format!("the document did not load: {}", error.localizedDescription());
                (me.emit)(FrameEvent::Failed(why));
            }
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, ivars: DelegateIvars) -> Retained<Self> {
        let this = Delegate::alloc(mtm).set_ivars(ivars);
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }

    fn emit(&self, event: FrameEvent) {
        let Some(inner) = self.ivars().inner.upgrade() else {
            return;
        };
        let emit = match inner.try_borrow() {
            Ok(me) => me.emit.clone(),
            Err(_) => return,
        };
        emit(event);
    }

    fn decide_action_for(&self, action: &WKNavigationAction) -> bool {
        // SAFETY: plain property reads on the action WebKit passes.
        let (target, source, request, download) = unsafe {
            (
                action.targetFrame(),
                action.sourceFrame(),
                action.request(),
                action.shouldPerformDownload(),
            )
        };
        if download {
            return false;
        }
        let url = url_text(request.URL().as_deref());
        // SAFETY: as above.
        let Some(main) = target.map(|t| unsafe { t.isMainFrame() }) else {
            return false;
        };
        if main {
            return url == self.ivars().relay;
        }
        // SAFETY: as above.
        if source.is_some_and(|s| unsafe { s.isMainFrame() }) {
            if let Some(inner) = self.ivars().inner.upgrade() {
                if let Ok(mut me) = inner.try_borrow_mut() {
                    me.expect_child = true;
                    me.child_failure = None;
                }
            }
        }
        true
    }

    /// The relay's iframe answered: an HTTP error is the navigation's `error`, reported at its `load`.
    fn child_response(&self, response: &WKNavigationResponse) {
        let Some(inner) = self.ivars().inner.upgrade() else {
            return;
        };
        let Ok(mut me) = inner.try_borrow_mut() else {
            return;
        };
        if !me.expect_child {
            return;
        }
        me.expect_child = false;
        // SAFETY: a plain property read.
        let answer = unsafe { response.response() };
        let url = url_text(answer.URL().as_deref());
        let missing = self.ivars().missing.borrow_mut().remove(&url);
        // WebKit may hand back the app's own answers without their status.
        let status = match answer.downcast_ref::<NSHTTPURLResponse>() {
            _ if missing => 404,
            Some(http) => http.statusCode(),
            None => 200,
        };
        if status >= 400 {
            me.child_failure = Some(format!("the server answered HTTP {status}"));
        }
    }

    fn relay_failed(&self, error: &NSError) {
        if error.code() == NSURL_ERROR_CANCELLED {
            return;
        }
        self.emit(FrameEvent::Failed(format!(
            "the relay page did not load: {}",
            error.localizedDescription()
        )));
    }

    fn on_script_message(&self, message: &WKScriptMessage) {
        // SAFETY: plain property reads on the message WebKit passes.
        let (name, body, frame) = unsafe { (message.name(), message.body(), message.frameInfo()) };
        let Some(text) = body.downcast_ref::<NSString>().map(NSString::to_string) else {
            return;
        };
        let Some(inner) = self.ivars().inner.upgrade() else {
            return;
        };
        if name.to_string() == "skCursor" {
            let style = cursor_style(&text);
            let Ok(mut me) = inner.try_borrow_mut() else {
                return;
            };
            if me.cursor != Some(style) {
                me.cursor = Some(style);
                (me.emit)(FrameEvent::Cursor(style));
            }
            return;
        }
        // Only the relay page itself: a frame inside it has the handler too, and must not speak as the relay.
        // SAFETY: plain property reads.
        let from_relay = unsafe { frame.isMainFrame() }
            && url_text(unsafe { frame.request() }.URL().as_deref()) == self.ivars().relay;
        if from_relay {
            received(&inner, &text);
        }
    }

    fn serve(&self, task: &ProtocolObject<dyn WKURLSchemeTask>) {
        // SAFETY: the task is live until `didFinish`, which this call makes.
        let request = unsafe { task.request() };
        let Some(url) = request.URL() else { return };
        let text = url_text(Some(&url));
        let rest = text
            .strip_prefix(APP_ORIGIN)
            .filter(|r| r.is_empty() || r.starts_with(['/', '?', '#']));
        let path = rest.map(|r| r.split(['?', '#']).next().unwrap_or(r));
        let path = match path {
            Some("") => Some("/"),
            other => other,
        };
        let body = path.and_then(|p| {
            if p == RELAY_PATH {
                Some(RELAY_HTML.as_bytes().to_vec())
            } else {
                (self.ivars().assets)(p)
            }
        });
        let (status, kind, bytes) = match body {
            Some(bytes) => (200, content_type(path.unwrap_or("/")), bytes),
            None => {
                let mut missing = self.ivars().missing.borrow_mut();
                if missing.len() >= 64 {
                    missing.clear();
                }
                missing.insert(text);
                (404, "text/plain", Vec::new())
            }
        };
        // A sandboxed document is an opaque origin: its own files are cross-origin to it.
        let headers = NSDictionary::from_slices(
            &[
                ns_string!("Content-Type"),
                ns_string!("Access-Control-Allow-Origin"),
                ns_string!("Cache-Control"),
            ],
            &[
                &*NSString::from_str(kind),
                ns_string!("*"),
                ns_string!("no-store"),
            ],
        );
        let Some(response) = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
            NSHTTPURLResponse::alloc(),
            &url,
            status,
            Some(ns_string!("HTTP/1.1")),
            Some(&headers),
        ) else {
            return;
        };
        // SAFETY: the task has not been stopped: WebKit stops a task only between run-loop turns.
        unsafe {
            task.didReceiveResponse(&response);
            task.didReceiveData(&NSData::from_vec(bytes));
            task.didFinish();
        }
    }
}

fn url_text(url: Option<&NSURL>) -> String {
    url.and_then(|u| u.absoluteString())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn open_files(
    web: &WKWebView,
    parameters: &WKOpenPanelParameters,
    handler: RcBlock<dyn Fn(*mut NSArray<NSURL>)>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        handler.call((std::ptr::null_mut(),));
        return;
    };
    let panel = NSOpenPanel::openPanel(mtm);
    // SAFETY: plain property reads.
    let (many, folders) = unsafe {
        (
            parameters.allowsMultipleSelection(),
            parameters.allowsDirectories(),
        )
    };
    panel.setAllowsMultipleSelection(many);
    panel.setCanChooseDirectories(folders);
    panel.setCanChooseFiles(true);
    let chosen = panel.clone();
    let done = RcBlock::new(move |response: NSModalResponse| {
        // NSModalResponseOK
        if response == 1 {
            let urls = chosen.URLs();
            handler.call((Retained::as_ptr(&urls).cast_mut(),));
        } else {
            handler.call((std::ptr::null_mut(),));
        }
    });
    match web.window() {
        Some(window) => panel.beginSheetModalForWindow_completionHandler(&window, &done),
        None => panel.beginWithCompletionHandler(&done),
    }
}

/// A CSS cursor keyword as the app's cursor.
fn cursor_style(css: &str) -> gpui::CursorStyle {
    use gpui::CursorStyle as C;
    match css {
        "text" => C::IBeam,
        "vertical-text" => C::IBeamCursorForVerticalLayout,
        "pointer" => C::PointingHand,
        "crosshair" => C::Crosshair,
        "grab" => C::OpenHand,
        "grabbing" => C::ClosedHand,
        "move" => C::Move,
        "all-scroll" => C::AllScroll,
        "not-allowed" | "no-drop" => C::OperationNotAllowed,
        "alias" => C::DragLink,
        "copy" => C::DragCopy,
        "context-menu" => C::ContextualMenu,
        "help" => C::Help,
        "progress" => C::Progress,
        "wait" => C::Wait,
        "cell" => C::Cell,
        "zoom-in" => C::ZoomIn,
        "zoom-out" => C::ZoomOut,
        "col-resize" => C::ResizeColumn,
        "row-resize" => C::ResizeRow,
        "ew-resize" => C::ResizeLeftRight,
        "ns-resize" => C::ResizeUpDown,
        "e-resize" => C::ResizeRight,
        "w-resize" => C::ResizeLeft,
        "n-resize" => C::ResizeUp,
        "s-resize" => C::ResizeDown,
        "nwse-resize" | "nw-resize" | "se-resize" => C::ResizeUpLeftDownRight,
        "nesw-resize" | "ne-resize" | "sw-resize" => C::ResizeUpRightDownLeft,
        _ => C::Arrow,
    }
}

/// The view all of a window's documents live in, directly under its gpui view.
fn host_of(app: &NSView, mtm: MainThreadMarker) -> Option<Retained<NSView>> {
    // SAFETY: a plain property read on a view in a window.
    let parent = unsafe { app.superview() }?;
    for view in parent.subviews().iter() {
        if view.isKindOfClass(Flipped::class()) {
            return Some(view);
        }
    }
    let host = Flipped::new(mtm, app.frame());
    host.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    parent.addSubview_positioned_relativeTo(&host, NSWindowOrderingMode::Below, Some(app));
    Some(host)
}

/// The gpui layer is opaque unless the window is transparent; the document shows through it only
/// where it is not.
fn see_through(app: &NSView) {
    // SAFETY: `layer` and `isOpaque`/`setOpaque:` are plain CALayer/NSView properties.
    unsafe {
        let layer: *mut AnyObject = msg_send![app, layer];
        if layer.is_null() {
            return;
        }
        let opaque: bool = msg_send![layer, isOpaque];
        if opaque {
            let _: () = msg_send![layer, setOpaque: false];
        }
    }
}

struct Inner {
    emit: Emit,
    config: Rc<Config>,
    app: ObjcWeak<NSView>,
    host: Retained<NSView>,
    clip: Retained<NSView>,
    web: Retained<SkWebView>,
    delegate: Retained<Delegate>,
    relay_ready: bool,
    queued: Vec<String>,
    placement: Option<Placement>,
    shown: bool,
    want_focus: bool,
    /// The relay's iframe is navigating and has not answered yet.
    expect_child: bool,
    child_failure: Option<String>,
    cursor: Option<gpui::CursorStyle>,
}

fn open(app: &NSView, config: Rc<Config>, emit: Emit) -> Option<Handle> {
    let mtm = MainThreadMarker::new()?;
    let Some(host) = host_of(app, mtm) else {
        log("the app's view is not in a window, so there is nowhere to show a document");
        return None;
    };
    let relay = format!("{APP_ORIGIN}{RELAY_PATH}");
    let devtools = config.devtools;
    let inner = Rc::new_cyclic(|weak: &Weak<RefCell<Inner>>| {
        let delegate = Delegate::new(
            mtm,
            DelegateIvars {
                inner: weak.clone(),
                assets: config.assets.clone(),
                missing: RefCell::new(HashSet::new()),
                relay: relay.clone(),
            },
        );
        let web = make_web_view(mtm, app, &delegate, devtools);
        let clip = Flipped::new(mtm, NSRect::ZERO);
        clip.setHidden(true);
        // SAFETY: plain property writes on views made here.
        unsafe { msg_send![&*clip, setClipsToBounds: true] }
        clip.addSubview(&web);
        let before = KeyStop::new(mtm, false, emit.clone());
        let after = KeyStop::new(mtm, true, emit.clone());
        clip.addSubview(&before);
        clip.addSubview(&after);
        // SAFETY: as above.
        unsafe {
            before.setNextKeyView(Some(&web));
            web.setNextKeyView(Some(&after));
        }
        host.addSubview(&clip);
        RefCell::new(Inner {
            emit: emit.clone(),
            config: config.clone(),
            app: ObjcWeak::from_retained(&app.retain()),
            host,
            clip,
            web,
            delegate,
            relay_ready: false,
            queued: Vec::new(),
            placement: None,
            shown: false,
            want_focus: false,
            expect_child: false,
            child_failure: None,
            cursor: None,
        })
    });
    let me = inner.borrow();
    let url = NSURL::URLWithString(&NSString::from_str(&relay))?;
    // SAFETY: loading a request in a webview made above, on the main thread.
    unsafe { me.web.loadRequest(&NSURLRequest::requestWithURL(&url)) };
    drop(me);
    Some(Handle(inner))
}

fn make_web_view(
    mtm: MainThreadMarker,
    app: &NSView,
    delegate: &Delegate,
    devtools: bool,
) -> Retained<SkWebView> {
    // SAFETY: WebKit configuration on the main thread, before the webview exists.
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setURLSchemeHandler_forURLScheme(
            Some(ProtocolObject::from_ref(delegate)),
            &NSString::from_str(SCHEME),
        );
        let content = config.userContentController();
        content.addScriptMessageHandler_name(
            ProtocolObject::from_ref(delegate),
            ns_string!("skRelay"),
        );
        let world = WKContentWorld::worldWithName(ns_string!("skathi"), mtm);
        content.addScriptMessageHandler_contentWorld_name(
            ProtocolObject::from_ref(delegate),
            &world,
            ns_string!("skCursor"),
        );
        let source = if devtools {
            CURSOR_JS.to_string()
        } else {
            format!("{CURSOR_JS}\n{NO_MENU_JS}")
        };
        let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(&source),
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
            &world,
        );
        content.addUserScript(&script);
        config
            .preferences()
            .setJavaScriptCanOpenWindowsAutomatically(false);

        let web = SkWebView::new(mtm, &config, app);
        web.setNavigationDelegate(Some(ProtocolObject::from_ref(delegate)));
        web.setUIDelegate(Some(ProtocolObject::from_ref(delegate)));
        web.setAllowsMagnification(false);
        web.setAllowsBackForwardNavigationGestures(false);
        web.setAllowsLinkPreview(false);
        web.unregisterDraggedTypes();
        if web.respondsToSelector(sel!(setInspectable:)) {
            web.setInspectable(devtools);
        }
        // WebKit has no public switch for a transparent page background; this key is the one
        // embedders use.
        if web.respondsToSelector(sel!(_setDrawsBackground:)) {
            let _: () = msg_send![
                &*web,
                setValue: &*NSNumber::new_bool(false),
                forKey: ns_string!("drawsBackground")
            ];
        }
        if web.respondsToSelector(sel!(setUnderPageBackgroundColor:)) {
            web.setUnderPageBackgroundColor(Some(&NSColor::clearColor()));
        }
        web
    }
}

fn received(inner: &Rc<RefCell<Inner>>, json: &str) {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Ok(mut me) = inner.try_borrow_mut() else {
        return;
    };
    match message.get("t").and_then(|t| t.as_str()) {
        Some("ready") => {
            me.relay_ready = true;
            for text in std::mem::take(&mut me.queued) {
                me.send(text);
            }
            if me.want_focus {
                me.focus(true);
            }
        }
        Some("load") => {
            me.expect_child = false;
            let event = match me.child_failure.take() {
                Some(why) => FrameEvent::Failed(why),
                None => FrameEvent::Loaded,
            };
            (me.emit)(event);
        }
        Some("msg") => {
            let origin = message.get("o").and_then(|o| o.as_str()).unwrap_or("null");
            let wire = message
                .get("w")
                .map(|w| w.to_string())
                .unwrap_or_else(|| "null".into());
            (me.emit)(FrameEvent::Message {
                origin: external_origin(origin, &me.config.page_origin),
                wire,
            });
        }
        _ => {}
    }
}

impl Inner {
    fn mtm(&self) -> MainThreadMarker {
        MainThreadMarker::from(&*self.web)
    }

    fn send(&mut self, json: String) {
        if !self.relay_ready {
            self.queued.push(json);
            return;
        }
        let script = NSString::from_str(&format!("__skRelay({json})"));
        // SAFETY: evaluating script in the relay page, on the main thread.
        unsafe { self.web.evaluateJavaScript_completionHandler(&script, None) };
    }

    fn apply_placement(&mut self, placement: Option<Placement>) {
        self.placement = placement;
        match placement.filter(|p| p.clip[2] > 0.0 && p.clip[3] > 0.0 && p.scale > 0.0) {
            None => {
                if self.shown {
                    self.clip.setHidden(true);
                    self.shown = false;
                }
            }
            Some(p) => {
                let s = f64::from(p.scale);
                let pt = |v: f32| f64::from(v.round()) / s;
                let [x, y, w, h] = p.bounds.map(pt);
                let [cx, cy, cw, ch] = p.clip.map(pt);
                self.clip
                    .setFrame(NSRect::new(NSPoint::new(cx, cy), NSSize::new(cw, ch)));
                self.web
                    .setFrame(NSRect::new(NSPoint::new(x - cx, y - cy), NSSize::new(w, h)));
                if let Some(app) = self.app.load() {
                    see_through(&app);
                }
                if !self.shown {
                    self.clip.setHidden(false);
                    self.shown = true;
                }
            }
        }
    }

    fn holds_keys(&self, window: &NSWindow) -> bool {
        window
            .firstResponder()
            .and_then(|r| r.downcast::<NSView>().ok())
            .is_some_and(|v| v.isDescendantOf(&self.clip))
    }

    fn focus(&mut self, on: bool) {
        self.want_focus = on;
        let Some(window) = self.web.window() else {
            return;
        };
        if on {
            if window.makeFirstResponder(Some(responder(&self.web))) {
                self.send("{\"t\":\"focus\"}".into());
            }
        } else if self.holds_keys(&window) {
            if let Some(app) = self.app.load() {
                window.makeFirstResponder(Some(responder(&app)));
            }
        }
    }

    fn rehome(&mut self, app: Retained<NSView>) {
        if self
            .app
            .load()
            .is_some_and(|a| Retained::as_ptr(&a) == Retained::as_ptr(&app))
        {
            return;
        }
        let Some(host) = host_of(&app, self.mtm()) else {
            return;
        };
        if let Some(window) = self.web.window() {
            if self.holds_keys(&window) {
                if let Some(old) = self.app.load() {
                    window.makeFirstResponder(Some(responder(&old)));
                }
            }
        }
        self.clip.removeFromSuperview();
        host.addSubview(&self.clip);
        self.host = host;
        self.app = ObjcWeak::from_retained(&app);
        *self.web.ivars().app.borrow_mut() = ObjcWeak::from_retained(&app);
        let placement = self.placement;
        self.shown = !self.clip.isHidden();
        self.apply_placement(placement);
    }

    fn pointer(&self, input: PointerInput) {
        let Some(p) = self.placement else { return };
        let Some(window) = self.web.window() else {
            return;
        };
        let current = NSApplication::sharedApplication(self.mtm())
            .currentEvent()
            .filter(|e| e.windowNumber() == window.windowNumber());
        let of = |kinds: &[NSEventType]| current.clone().filter(|e| kinds.contains(&e.r#type()));
        let s = f64::from(p.scale);
        let inside = NSPoint::new(
            f64::from(p.bounds[0] + input.x) / s,
            f64::from(p.bounds[1] + input.y) / s,
        );
        let mut flags = NSEventModifierFlags::empty();
        if input.shift {
            flags |= NSEventModifierFlags::Shift;
        }
        if input.control {
            flags |= NSEventModifierFlags::Control;
        }
        let synth = |kind: NSEventType, at: NSPoint| {
            let location = self.host.convertPoint_toView(at, None);
            let time = NSProcessInfo::processInfo().systemUptime();
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind, location, flags, time, window.windowNumber(), None, 0, 1, 0.0,
            )
        };
        use NSEventType as T;
        let web = &self.web;
        match input.kind {
            PointerKind::Move => {
                let event = of(&[
                    T::MouseMoved,
                    T::LeftMouseDragged,
                    T::RightMouseDragged,
                    T::OtherMouseDragged,
                ])
                .or_else(|| {
                    synth(
                        if input.buttons & 1 != 0 {
                            T::LeftMouseDragged
                        } else {
                            T::MouseMoved
                        },
                        inside,
                    )
                });
                let Some(event) = event else { return };
                let kind = event.r#type();
                web.forward(|w| unsafe {
                    // SAFETY: WKWebView's own mouse handlers, given an event in its window.
                    match kind {
                        k if k == T::LeftMouseDragged => msg_send![w, mouseDragged: &*event],
                        k if k == T::RightMouseDragged => msg_send![w, rightMouseDragged: &*event],
                        k if k == T::OtherMouseDragged => msg_send![w, otherMouseDragged: &*event],
                        _ => msg_send![w, mouseMoved: &*event],
                    }
                });
            }
            PointerKind::Down(button) | PointerKind::Up(button) => {
                let down = matches!(input.kind, PointerKind::Down(_));
                let kind = match (button, down) {
                    (0, true) => T::LeftMouseDown,
                    (0, false) => T::LeftMouseUp,
                    (2, true) => T::RightMouseDown,
                    (2, false) => T::RightMouseUp,
                    (1, true) => T::OtherMouseDown,
                    (1, false) => T::OtherMouseUp,
                    _ => return,
                };
                let Some(event) = of(&[kind]).or_else(|| synth(kind, inside)) else {
                    return;
                };
                web.forward(|w| unsafe {
                    // SAFETY: as above.
                    match (button, down) {
                        (0, true) => msg_send![w, mouseDown: &*event],
                        (0, false) => msg_send![w, mouseUp: &*event],
                        (2, true) => msg_send![w, rightMouseDown: &*event],
                        (2, false) => msg_send![w, rightMouseUp: &*event],
                        (_, true) => msg_send![w, otherMouseDown: &*event],
                        (_, false) => msg_send![w, otherMouseUp: &*event],
                    }
                });
                // The press made the gpui view first responder; a focused document keeps the keys.
                if down && self.want_focus && !self.holds_keys(&window) {
                    window.makeFirstResponder(Some(responder(web)));
                }
            }
            // Only a real wheel or trackpad event: it carries the phases and momentum WebKit scrolls by.
            PointerKind::Wheel(..) => {
                let Some(event) = of(&[T::ScrollWheel]) else {
                    return;
                };
                web.forward(|w| unsafe {
                    // SAFETY: as above.
                    msg_send![w, scrollWheel: &*event]
                });
            }
            PointerKind::Leave => {
                // Outside the box, so the document's hover ends wherever the pointer went.
                let outside = NSPoint::new(
                    f64::from(p.bounds[0]) / s - 1.0,
                    f64::from(p.bounds[1]) / s - 1.0,
                );
                let location = self.host.convertPoint_toView(outside, None);
                let time = NSProcessInfo::processInfo().systemUptime();
                // SAFETY: an enter/exit event with no tracking area and no user data.
                let event = unsafe {
                    NSEvent::enterExitEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_trackingNumber_userData(
                        T::MouseExited, location, NSEventModifierFlags::empty(), time, window.windowNumber(), None, 0, 0, std::ptr::null_mut(),
                    )
                };
                let Some(event) = event else { return };
                web.forward(|w| unsafe {
                    // SAFETY: as above.
                    msg_send![w, mouseExited: &*event]
                });
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
                message.insert(
                    "src".into(),
                    internal_url(src, &me.config.page_origin).into(),
                );
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
        me.expect_child = false;
        me.child_failure = None;
        me.delegate.ivars().missing.borrow_mut().clear();
        me.queued.retain(|m| !m.starts_with("{\"t\":\"post\""));
        me.send(serde_json::Value::Object(message).to_string());
    }

    fn post(&mut self, wire: &str, target_origin: &str) {
        let mut me = self.0.borrow_mut();
        let origin =
            serde_json::Value::String(internal_origin(target_origin, &me.config.page_origin));
        me.send(format!("{{\"t\":\"post\",\"o\":{origin},\"w\":{wire}}}"));
    }

    fn place(&mut self, placement: Option<Placement>) {
        self.0.borrow_mut().apply_placement(placement);
    }

    fn rehome(&mut self, window: &mut gpui::Window) {
        if let Some(app) = app_view(window) {
            self.0.borrow_mut().rehome(app);
        }
    }

    fn focus(&mut self, on: bool) {
        self.0.borrow_mut().focus(on);
    }

    fn pointer(&mut self, input: PointerInput) {
        self.0.borrow().pointer(input);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let me = self.0.borrow();
        if let Some(window) = me.web.window() {
            if me.holds_keys(&window) {
                if let Some(app) = me.app.load() {
                    window.makeFirstResponder(Some(responder(&app)));
                }
            }
        }
        // SAFETY: tearing down a webview made here, on the main thread.
        unsafe {
            me.web.stopLoading();
            me.web.setNavigationDelegate(None);
            me.web.setUIDelegate(None);
            me.web
                .configuration()
                .userContentController()
                .removeAllScriptMessageHandlers();
        }
        me.clip.removeFromSuperview();
    }
}
