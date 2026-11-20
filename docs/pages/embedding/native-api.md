---
title: Native APIs in the setup hook
label: Native APIs
description: Build JS objects of your own in the engine. Builder::setup and HostContext, native namespaces backed by Rust functions, posting results from worker threads into the page, frame, reload and shutdown hooks, cookies and OS-sealed secrets.
section: Embedding
order: 3
---

[Native modules](/embedding/native-modules/) carry serde values between Rust and the app, read and built straight from the engine's values, with bytes passed in place. That is the right default, and fast enough for a call per frame or a thousand a second. Its limit is its shape: values in, a value out, and every event a separate delivery. When Rust holds on to an app's callback, has a stream to deliver, or you want a JS object that behaves like a browser API, you build it directly in the engine.

That happens in `Builder::setup`. It runs **once**, on the Skathi thread, after storage and the native main window are ready, before application code and the first reveal, and gets a `HostContext`:

```rust
let builder = skathi::Builder::new()
    .payload_file("app/dist/release/.sk/app.skpk")
    .setup(|host| {
        // host.js(), host.expose(…), host.dispatcher(), host.on_frame(…), host.read_secret(…) …
        Ok(())
    });
```

An `Err` or a panic in `setup` stops startup; `spawn` then never shows a window. Keep it short: the app waits for it.

## What HostContext offers

| Call                                             | What it is for                                                                                              |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `js()`                                           | Direct JavaScriptCore: build objects, strings, numbers and native functions                                 |
| `expose(name, value)`                            | Install a value as a global before app code runs. Refuses names starting with `__sk` and globals that exist |
| `dispatcher()`                                   | A bounded queue from worker threads onto the JS thread                                                      |
| `on_frame(hook)`                                 | Runs once per frame on the JS thread, before the frame's JS work: deliver what workers queued               |
| `on_reload(hook)`                                | Runs after the old page is torn down and before the new one: drop callbacks the old page gave you           |
| `on_shutdown(hook)`                              | Runs once during an orderly shutdown: release what you protected                                            |
| `handle()`                                       | The running app's `Handle`, for things that outlive `setup` (on macOS the only way to one)                  |
| `surface(name)`                                  | A [native frame](/embedding/native-frames/) surface                                                         |
| `set_cookie(cookie)`                             | Put a cookie, HttpOnly included, in the jar before the first request                                        |
| `read_secret(name)`, `write_secret(name, value)` | Strings sealed by the OS (DPAPI, the Keychain, the Secret Service), never written in plain text             |
| `storage_dir()`, `random_bytes(buf)`             | Where the profile's storage lives; cryptographically random bytes                                           |

## Unsafe native window handles (since 0.6)

> [!WARNING]
> Use a native handle only when the [window API](/window-api/) cannot express the operation. Changing the frame, style, delegate, level or lifecycle behind Skathi’s back can invalidate its state and crash the process.

`HostContext::unsafe_window_handle()` and `Handle::unsafe_window_handle()` return `Option<raw_window_handle::RawWindowHandle>` for the main window on the Skathi UI thread. Use `unsafe_window_handle_for(id: u64)` on either object for the window ID from a pop-out’s `open` event. A closed or unknown window and a call on a worker thread return `None`. Obtain a fresh handle for each native operation. Closing or recreating the window invalidates the previous handle.

On macOS the AppKit variant carries the content `NSView`. Obtain its `window` on the main thread for the owning `NSWindow`. Windows supplies a Win32 `HWND`, X11 an XCB window ID and Wayland a `wl_surface`. Rust embedders can use `host.dispatcher()` to post work to the UI thread.

## A native namespace

A global object whose methods are Rust functions. They run synchronously, take and return real JS values, and cost no more than any other native call (a native module's function costs about the same; what you gain here is control over the values):

```rust
use skathi::js;

.setup(|host| {
    let cx = host.js();
    let clock = cx.object();
    let now = js::function(cx, "now", 0, |_, _| Ok(js::Value::number(monotonic_ms())))
        .map_err(|e| e.to_string())?;
    cx.set(clock, &cx.atom("now"), now).map_err(|e| e.to_string())?;
    host.expose("nativeClock", clock).map_err(|e| e.to_string())?;
    Ok(())
})
```

```ts title="src/native.d.ts"
declare global {
  const nativeClock: { now(): number };
}
export {};
```

The app calls `nativeClock.now()` like any built-in. Inside a native function:

- `args.arg(i)` is the `i`th argument, `undefined` past the end; `args.argc()` counts them; `args.this()` is the receiver.
- `cx.to_number(v)`, `cx.to_rust_string(v)` read arguments; `js::Value::number`, `cx.string(…)`, `cx.object()`, `cx.array(n)` make results.
- An `Err` throws a JavaScript exception; a panic is caught and thrown as `native plugin method panicked`, so a bug in Rust never takes the process down from JS.

Register native functions once, in `setup`. Each one takes a slot for the life of the process; making a new one per event runs the slots out.

> **Warning:** A `js::Value` is only safe on the JS thread's stack, or while protected with `cx.protect`. Never move one into a worker thread, even though it is `Copy`. Workers carry Rust data, and the dispatcher turns it into JS on the right thread.

## From worker threads into the page

The dispatcher is a bounded queue onto the JS thread. A worker posts a closure that owns plain Rust data; the closure runs on the JS thread with the `Context`, where it can build values and call into the page.

This example gives the app `sync.onProgress(callback)` and calls the callback from a sync thread. It shows the whole life of a JS callback held by Rust: protect it while you hold it, drop it on reload, release it on shutdown.

```rust
use std::cell::Cell;
use skathi::js::{self, Value};

thread_local! {
    // The app's callback. Only touched on the JS thread.
    static LISTENER: Cell<Option<Value>> = const { Cell::new(None) };
}

fn forget_listener(cx: &js::Context) {
    if let Some(old) = LISTENER.with(|slot| slot.take()) {
        cx.unprotect(old);
    }
}

let builder = skathi::Builder::new().setup(move |host| {
    let cx = host.js();
    let sync = cx.object();
    let on_progress = js::function(cx, "onProgress", 1, |cx, args| {
        let callback = args.arg(0);
        forget_listener(cx);
        cx.protect(callback);
        LISTENER.with(|slot| slot.set(Some(callback)));
        Ok(Value::undefined())
    })
    .map_err(|e| e.to_string())?;
    cx.set(sync, &cx.atom("onProgress"), on_progress).map_err(|e| e.to_string())?;
    host.expose("sync", sync).map_err(|e| e.to_string())?;

    let dispatcher = host.dispatcher();
    std::thread::spawn(move || {
        for progress in engine.progress() {
            let (done, total) = (progress.done as f64, progress.total as f64);
            let posted = dispatcher.post(move |cx| {
                if let Some(callback) = LISTENER.with(Cell::get) {
                    let _ = cx.call(callback, Value::undefined(), &[Value::number(done), Value::number(total)]);
                }
            });
            if posted.is_err() {
                break; // Closed: the app has gone; Full: it is not keeping up
            }
        }
    });

    host.on_reload(forget_listener);
    host.on_shutdown(forget_listener);
    Ok(())
});
```

```ts
sync.onProgress((done: number, total: number) => {
  progress.value = total ? done / total : 0;
});
```

`post` never blocks. It fails with `PostError::Closed` once the app has closed and `PostError::Full` when 1024 tasks are waiting, which means the producer outruns the UI: coalesce, or drop the stale ones.

### Batching per frame

A stream that produces faster than the screen refreshes (log lines, ticks, samples) is cheaper delivered once per frame. Workers push into a queue of your own; `on_frame` drains it on the JS thread right before the frame's JS work, so a handler never sees half an update:

```rust
let queue = Arc::new(Mutex::new(Vec::<LogLine>::new()));
let pending = queue.clone();
host.on_frame(move || {
    let lines = std::mem::take(&mut *pending.lock().unwrap());
    if !lines.is_empty() {
        deliver(&lines); // one call into the page for the whole batch
    }
});
```

`on_frame` is on the frame's critical path: keep it bounded, and leave the expensive work on the worker.

## Cookies and secrets

Some things must exist before the app's first line runs: a session cookie for the API the app talks to, a token read from the OS keychain.

```rust
use skathi::{HostCookie, SameSite};

.setup(|host| {
    if let Some(token) = host.read_secret("api-session")? {
        host.set_cookie(HostCookie {
            url: "https://api.example.com".into(),
            name: "session".into(),
            value: token,
            domain: None,
            path: Some("/".into()),
            secure: true,
            http_only: true,
            same_site: SameSite::Lax,
            expires: None, // a session cookie
        })?;
    }
    Ok(())
})
```

- An HttpOnly cookie set here goes out with the app's `fetch` requests, and the app's JavaScript can never read it.
- `read_secret` returns `Ok(None)` only when the secret does not exist; a corrupt or undecryptable one is an error, so a broken keychain is not mistaken for a logged-out user.
- Secrets are sealed by the OS provider and stored in the profile. There is no plain-text fallback: without a provider, `write_secret` fails.

Cookies and secrets need the `storages` feature, which is on by default.

## The raw engine

`HostContext::runtime()` hands out the engine's own runtime, for an extension built against the same `sk-runtime` crate that registers host functions of its own. It exists for that one case; everything above is the supported surface, and `expose` refuses the `__sk` names on purpose.

Next: [Native frames](/embedding/native-frames/), for when what you have to show is pixels.
