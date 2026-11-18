//! WKWebView end to end, headless: a window that is never ordered front, a plain view standing in
//! for gpui's, and the main thread's run loop. libtest runs tests off the main thread, so this
//! file is its own harness.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent,
        NSEventModifierFlags, NSEventType, NSResponder, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSDate, NSObject, NSPoint, NSRect, NSRunLoop, NSSize, NSString};
    use sk_tree::iframe::{Frame, FrameAttrs, FrameEvent, Placement, PointerInput, PointerKind};

    thread_local! {
        static APP_KEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    define_class!(
        // SAFETY: NSView has no subclassing requirements; no ivars.
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "SkathiTestAppView"]
        struct AppView;

        // Stands in for gpui's view: records the keys that reach it.
        impl AppView {
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool {
                true
            }

            #[unsafe(method(keyDown:))]
            fn key_down(&self, event: &NSEvent) {
                APP_KEYS.with(|k| k.borrow_mut().push(format!("down {}", event.keyCode())));
            }
        }
    );

    fn app_keys() -> Vec<String> {
        APP_KEYS.with(|k| k.borrow().clone())
    }

    const PAGE: &str = r#"<!doctype html><html><body style="margin:0">
<a id="link" href="/elsewhere.html" style="display:block;width:100px;height:40px">link</a>
<input id="field">
<script>
document.getElementById("field").addEventListener("input", (e) => say({ type: "typed", value: e.target.value }));
const say = (m) => parent.postMessage(m, "*");
addEventListener("message", (e) => {
  const d = e.data;
  if (d.type === "ping") say({ type: "echo", n: d.n, origin: e.origin, sourceIsParent: e.source === parent });
  if (d.type === "escape") {
    let topNav = "allowed";
    try { top.location.href = "https://example.com/"; } catch (err) { topNav = err.name; }
    const opened = window.open("https://example.com/");
    const t0 = Date.now();
    alert("hi");
    const confirmed = confirm("sure?");
    const alertMs = Date.now() - t0;
    setTimeout(() => say({ type: "escaped", topNav, opened: opened === null, confirmed, alertMs }), 300);
  }
  if (d.type === "field") document.getElementById("field").focus();
  if (d.type === "hover") {
    document.getElementById("link").dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 5, clientY: 5 }));
  }
});
addEventListener("mousedown", (e) => say({ type: "down", x: e.clientX, y: e.clientY, button: e.button }));
say({ type: "hello", secure: isSecureContext, origin: location.origin, w: innerWidth, h: innerHeight });
</script></body></html>"#;

    type Log = Rc<RefCell<Vec<FrameEvent>>>;

    struct Harness {
        _window: Retained<NSWindow>,
        view: Retained<NSView>,
        log: Log,
        failures: Vec<String>,
    }

    impl Harness {
        fn frame(&self) -> Box<dyn Frame> {
            let log = self.log.clone();
            let config = sk_webview::Config {
                assets: Rc::new(|path| match path {
                    "/page.html" => Some(PAGE.as_bytes().to_vec()),
                    _ => None,
                }),
                user_data: std::env::temp_dir().join("sk-webview-test"),
                page_origin: "app://redjs".into(),
                devtools: false,
            };
            let view = NonNull::from(&*self.view).cast();
            sk_webview::frame_in_view(view, config, Rc::new(move |e| log.borrow_mut().push(e)))
                .expect("a frame")
        }

        fn wait(&self, what: &str, done: impl Fn(&[FrameEvent]) -> bool) -> bool {
            let end = Instant::now() + Duration::from_secs(15);
            while Instant::now() < end {
                if done(&self.log.borrow()) {
                    return true;
                }
                let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
                NSRunLoop::currentRunLoop().runUntilDate(&until);
            }
            eprintln!(
                "timed out waiting for {what}; events: {:?}",
                self.log.borrow()
            );
            false
        }

        fn wait_for(&self, time: Duration) {
            let until = NSDate::dateWithTimeIntervalSinceNow(time.as_secs_f64());
            NSRunLoop::currentRunLoop().runUntilDate(&until);
        }

        fn check(&mut self, name: &str, ok: bool) {
            println!("{} {name}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.failures.push(name.into());
            }
        }

        fn messages(&self, kind: &str) -> Vec<(String, serde_json::Value)> {
            self.log
                .borrow()
                .iter()
                .filter_map(|e| match e {
                    FrameEvent::Message { origin, wire } => {
                        let v: serde_json::Value = serde_json::from_str(wire).ok()?;
                        Some((origin.clone(), decode(&v)))
                    }
                    _ => None,
                })
                .filter(|(_, v)| v["type"] == kind)
                .collect()
        }
    }

    /// The relay's wire for plain objects: `["o", [[key, value]...]]`.
    fn decode(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Array(a) if a.first().and_then(|t| t.as_str()) == Some("o") => {
                let mut out = serde_json::Map::new();
                for entry in a[1].as_array().into_iter().flatten() {
                    if let (Some(k), Some(v)) = (entry[0].as_str(), entry.get(1)) {
                        out.insert(k.into(), decode(v));
                    }
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(a) if a.first().and_then(|t| t.as_str()) == Some("u") => {
                serde_json::Value::Null
            }
            other => other.clone(),
        }
    }

    fn wire(json: &str) -> String {
        // `{"type":"ping","n":7}` in the relay's codec.
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let entries: Vec<serde_json::Value> = v
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| serde_json::json!([k, v]))
            .collect();
        serde_json::json!(["o", entries]).to_string()
    }

    /// A local web server: 404 for `/gone`, a page for anything else.
    fn serve_http() -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let status = if head.starts_with("GET /gone") {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let body = "<p>page</p>";
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    fn src(path: &str) -> FrameAttrs {
        FrameAttrs {
            src: Some(path.into()),
            ..Default::default()
        }
    }

    fn placement() -> Placement {
        Placement {
            bounds: [40.0, 30.0, 640.0, 480.0],
            clip: [40.0, 30.0, 640.0, 400.0],
            radii: [0.0; 4],
            scale: 2.0,
            opacity: 1.0,
        }
    }

    fn loaded(log: &[FrameEvent]) -> bool {
        log.iter().any(|e| matches!(e, FrameEvent::Loaded))
    }

    fn failed(log: &[FrameEvent]) -> bool {
        log.iter().any(|e| matches!(e, FrameEvent::Failed(_)))
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("the harness runs on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        let rect = NSRect::new(NSPoint::new(-20000.0, -20000.0), NSSize::new(800.0, 600.0));
        // SAFETY: a borderless window that is never shown.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window outlives the harness.
        unsafe { window.setReleasedWhenClosed(false) };
        let content = window.contentView().unwrap();
        // SAFETY: `initWithFrame:` is NSView's designated initializer.
        let view: Retained<AppView> =
            unsafe { msg_send![AppView::alloc(mtm), initWithFrame: content.bounds()] };
        let view = view.into_super();
        content.addSubview(&view);
        let mut h = Harness {
            _window: window,
            view,
            log: Rc::default(),
            failures: Vec::new(),
        };

        let mut frame = h.frame();
        frame.navigate(&src("/page.html"));
        frame.place(Some(placement()));
        let ok = h.wait("load and hello", |l| {
            loaded(l) && !h.messages("hello").is_empty()
        });
        h.check(
            "an app file loads and says hello",
            ok && !failed(&h.log.borrow()),
        );
        if let Some((origin, hello)) = h.messages("hello").first().cloned() {
            h.check(
                "the document's origin is the app's, as the app sees it",
                origin == "app://redjs",
            );
            h.check("the document is a secure context", hello["secure"] == true);
            h.check(
                "the document is sized to the box in points",
                hello["w"] == 320 && hello["h"] == 240,
            );
        }

        for n in 0..5 {
            frame.post(&wire(&format!("{{\"type\":\"ping\",\"n\":{n}}}")), "/");
        }
        let ok = h.wait("echoes", |_| h.messages("echo").len() >= 5);
        let ns: Vec<i64> = h
            .messages("echo")
            .iter()
            .map(|(_, v)| v["n"].as_i64().unwrap_or(-1))
            .collect();
        h.check(
            "messages reach the document in order and come back",
            ok && ns == [0, 1, 2, 3, 4],
        );
        h.check(
            "the document sees the app's messages from its parent",
            h.messages("echo")
                .iter()
                .all(|(_, v)| v["sourceIsParent"] == true),
        );

        frame.post(&wire("{\"type\":\"escape\"}"), "/");
        let ok = h.wait("escape", |_| !h.messages("escaped").is_empty());
        if let Some((_, e)) = h.messages("escaped").first() {
            h.check("window.open gives nothing", e["opened"] == true);
            h.check(
                "confirm() answers no without a dialog",
                e["confirmed"] == false,
            );
            h.check(
                "alert() returns at once",
                e["alertMs"].as_i64().unwrap_or(9999) < 200,
            );
        } else {
            h.check("the escape attempts report back", ok);
        }
        frame.post(&wire("{\"type\":\"ping\",\"n\":99}"), "/");
        let ok = h.wait("echo after escape", |_| {
            h.messages("echo").iter().any(|(_, v)| v["n"] == 99)
        });
        h.check(
            "a top-level navigation is blocked: the relay is still there",
            ok,
        );
        h.check(
            "the document loaded once",
            h.log
                .borrow()
                .iter()
                .filter(|e| matches!(e, FrameEvent::Loaded))
                .count()
                == 1
                && h.messages("hello").len() == 1,
        );

        frame.post(&wire("{\"type\":\"hover\"}"), "/");
        let ok = h.wait("cursor", |l| {
            l.iter()
                .any(|e| matches!(e, FrameEvent::Cursor(gpui::CursorStyle::PointingHand)))
        });
        h.check(
            "the cursor follows the document: a link is the pointing hand",
            ok,
        );

        frame.pointer(PointerInput {
            kind: PointerKind::Down(0),
            x: 200.0,
            y: 100.0,
            buttons: 1,
            shift: false,
            control: false,
        });
        frame.pointer(PointerInput {
            kind: PointerKind::Up(0),
            x: 200.0,
            y: 100.0,
            buttons: 0,
            shift: false,
            control: false,
        });
        let ok = h.wait("down", |_| !h.messages("down").is_empty());
        let at = h
            .messages("down")
            .first()
            .map(|(_, v)| (v["x"].clone(), v["y"].clone()));
        h.check(
            "a forwarded press lands at the document's coordinates",
            ok && at == Some((serde_json::json!(100), serde_json::json!(50))),
        );

        frame.focus(true);
        let first = h._window.firstResponder();
        let focused = first.is_some_and(|r| {
            // SAFETY: a class name read.
            let name: Retained<objc2_foundation::NSString> = unsafe { msg_send![&*r, className] };
            name.to_string() == "SkathiFrameWebView"
        });
        h.check(
            "focus moves the window's first responder to the webview",
            focused,
        );

        frame.post(&wire("{\"type\":\"field\"}"), "/");
        h.wait_for(Duration::from_millis(500));
        let window = h._window.clone();
        let key = |chars: &str, code: u16, down: bool| {
            NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                if down { NSEventType::KeyDown } else { NSEventType::KeyUp },
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(),
                0.0,
                window.windowNumber(),
                None,
                &NSString::from_str(chars),
                &NSString::from_str(chars),
                false,
                code,
            )
            .unwrap()
        };
        for down in [true, false] {
            window.sendEvent(&key("a", 0x00, down));
        }
        let ok = h.wait("typed", |_| {
            h.messages("typed").iter().any(|(_, v)| v["value"] == "a")
        });
        h.check("keys reach the focused document", ok);
        for down in [true, false] {
            window.sendEvent(&key("\t", 0x30, down));
        }
        let ok = h.wait("tab out", |l| {
            l.iter()
                .any(|e| matches!(e, FrameEvent::FocusOut { forward: true }))
        });
        h.check(
            "Tab past the document's last element hands the focus on",
            ok,
        );
        frame.focus(true);
        window.sendEvent(&key("\u{F708}", 0x60, true));
        h.check(
            "F5 goes to the app while the document has the keys",
            app_keys() == ["down 96"],
        );
        frame.focus(false);
        let back = h._window.firstResponder().is_some_and(|r| {
            let r: &objc2::runtime::AnyObject = &r;
            let v: &objc2::runtime::AnyObject = &h.view;
            std::ptr::eq(r, v)
        });
        h.check("focus(false) gives the keys back to the app's view", back);

        let content = h._window.contentView().unwrap();
        let subviews = content.subviews();
        let host = subviews.objectAtIndex(0);
        h.check(
            "the documents' host is directly under the app's view, its size",
            subviews.count() == 2
                && std::ptr::eq(&*subviews.objectAtIndex(1), &*h.view)
                && host.frame() == h.view.frame(),
        );
        let clip = host.subviews().objectAtIndex(0);
        let web = clip.subviews().objectAtIndex(0);
        let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        h.check(
            "the clip is the visible part of the box, the webview the box, in points",
            clip.frame() == rect(20.0, 15.0, 320.0, 200.0)
                && web.frame() == rect(0.0, 0.0, 320.0, 240.0),
        );

        // The window is never on screen, so the document's own visibilityState stays "hidden".
        frame.place(None);
        h.check(
            "a box that is not painted hides its document",
            clip.isHidden(),
        );
        frame.place(Some(placement()));
        h.check("painted again, the document shows", !clip.isHidden());
        drop(frame);
        h.check(
            "a dropped document leaves the host",
            host.subviews().count() == 0,
        );

        h.log.borrow_mut().clear();
        let mut frame = h.frame();
        frame.navigate(&src("/missing.html"));
        frame.place(Some(placement()));
        let ok = h.wait("missing", |l| failed(l) || loaded(l));
        h.check(
            "a missing app file is an error",
            ok && failed(&h.log.borrow()) && !loaded(&h.log.borrow()),
        );
        drop(frame);

        h.log.borrow_mut().clear();
        let mut frame = h.frame();
        frame.navigate(&src("https://nonexistent.invalid/"));
        let ok = h.wait("dns failure", |l| failed(l) || loaded(l));
        h.check(
            "a host that does not resolve is an error",
            ok && failed(&h.log.borrow()),
        );
        drop(frame);

        let port = serve_http();
        for (path, error) in [("/gone", true), ("/here", false)] {
            h.log.borrow_mut().clear();
            let mut frame = h.frame();
            frame.navigate(&src(&format!("http://127.0.0.1:{port}{path}")));
            let ok = h.wait("http answer", |l| failed(l) || loaded(l));
            h.check(
                &format!(
                    "a web page answered {path} is {}",
                    if error { "an error" } else { "loaded" }
                ),
                ok && failed(&h.log.borrow()) == error,
            );
            drop(frame);
        }

        h.log.borrow_mut().clear();
        let mut frame = h.frame();
        frame.navigate(&FrameAttrs {
            srcdoc: Some(
                "<script>parent.postMessage({type:'hello',origin:location.origin},'*')</script>"
                    .into(),
            ),
            sandbox: Some("allow-scripts".into()),
            ..Default::default()
        });
        let ok = h.wait("srcdoc", |l| loaded(l))
            && h.wait("srcdoc hello", |_| !h.messages("hello").is_empty());
        let origin = h.messages("hello").first().map(|(o, _)| o.clone());
        h.check(
            "a sandboxed srcdoc loads and speaks with the opaque origin",
            ok && origin.as_deref() == Some("null"),
        );
        drop(frame);

        for _ in 0..10 {
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
        }
        if h.failures.is_empty() {
            println!("all passed");
        } else {
            eprintln!("{} failed: {:?}", h.failures.len(), h.failures);
            std::process::exit(1);
        }
    }
}
