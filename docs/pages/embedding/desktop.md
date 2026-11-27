---
title: Window, tray and hotkeys
description: Control the embedded app's window from Rust (show, hide at start, close, wait), give it a tray icon and global hotkeys from the native side, and let your exe own single instance and deep links.
section: Embedding
order: 7
---

The app's window is the app's business: it opens, closes, minimises and pops out as the [Windows guide](/guides/windows/) describes, through `appWindow`. Your Rust side does not have to touch it. When it does want a say (a tray app that starts hidden, a service that closes the UI on logout, a launcher that waits for it), the `Builder` and the `Handle` cover it.

## The window from Rust

| On `Builder`      | Effect                                                                                             |
| ----------------- | -------------------------------------------------------------------------------------------------- |
| `visible(false)`  | Boot the app without showing its window. JS runs, background work goes on, nothing appears         |
| `activate(false)` | Open the window without taking the foreground, for a console tool that keeps the keyboard          |
| `args([...])`     | The app's own arguments, seen as `appLaunch.argv`; none of them is read as a host flag             |
| `desktop_id(id)`  | Linux: the desktop entry id (`com.example.Notes`), the Wayland app id and X11 class of the windows |

| On `Handle`       | Effect                                                                                             |
| ----------------- | -------------------------------------------------------------------------------------------------- |
| `show()`          | Show and activate the window, including the first show of one started hidden. `false` once closing |
| `close()`         | Close the app as `appWindow.close()` does: the windows go at once, the teardown runs behind them   |
| `is_closed()`     | Whether the app has finished                                                                       |
| `wait()`          | Block until the app has closed; returns an `Exit`                                                  |
| `wait_timeout(d)` | `wait`, for at most `d`; `None` if it is still running                                             |

`Exit::clean` is `false` when the teardown ran past its deadline or the Skathi thread panicked, and `Exit::summary` says what closed the app and how long each teardown step took. It is the line to put in your own log.

After the app closes, Skathi's thread sleeps for the rest of the process: it cannot start again. Your program goes on; a launcher that should come back to its UI keeps the process and shows the window again instead of closing it. In the app, that is a close handler that hides instead of closing; see [Closing, hiding and quitting](/guides/closing-and-background/).

## A tray app

A program that lives in the tray starts hidden and shows its window when the user asks:

```rust
use skathi::tray::{Icon, MenuItem, Options, Tray};

let ui = skathi::Builder::new()
    .payload_file("app/dist/release/.sk/app.skpk")
    .visible(false)
    .spawn()?;

if Tray::is_available() {
    let tray = Tray::app()?;
    let (show, open, quit) = (ui.clone(), ui.clone(), ui.clone());
    tray.set(Some(Options {
        icon: Some(Icon::rgba(32, 32, include_bytes!("../assets/tray-32.rgba").to_vec())?),
        tooltip: "Notes".into(),
        menu: vec![
            MenuItem::action("Open Notes", move || {
                open.show();
            }),
            MenuItem::Separator,
            MenuItem::quit(),
        ],
        on_click: Some(std::sync::Arc::new(move || {
            show.show();
        })),
        on_quit: Some(std::sync::Arc::new(move || quit.close())),
    }))?;
}
let exit = ui.wait();
```

- The tray exists on Windows and macOS; `Tray::is_available()` says whether this platform has one.
- `Tray::app()` is **the app's** tray, the same one [`appTray`](/app-api/app-tray/) controls from JavaScript. `set` replaces everything at once, so pick one side to own it. Most apps let the JS side own the menu, since it knows the app's state, and use the Rust side only when the tray must work before the UI loads.
- Callbacks may run on another thread than yours: keep them short and talk to the app through the `Handle`, with `show`, `close` or a module event sent by `emit_to`.
- `MenuItem::quit()` runs `on_quit`, which is what quitting means for your program; `set` refuses a quit item without it.

## Global hotkeys

[`appHotkeys`](/app-api/app-hotkeys/) gives the app system-wide hotkeys. When the binding belongs to the native side (push-to-talk in a voice client, a capture key in a recorder), `skathi::hotkeys` is the same service in Rust:

```rust
use std::sync::Arc;
use skathi::hotkeys::{Binding, Button, Chord, Hotkeys};

#[skathi::data]
struct Hotkey {
    id: String,
    /// "down" or "up": a binding can be held.
    phase: String,
}

struct Voice;

// On the builder: .module("voice", Voice)
#[skathi::module(events(hotkey = Hotkey))]
impl Voice {}

let hotkeys = Hotkeys::new();
let events = ui.clone();
hotkeys.set_handler(Arc::new(move |fired| {
    let hotkey = Hotkey { id: fired.id.clone(), phase: fired.phase.clone() };
    events.emit_to("voice", "hotkey", &hotkey).ok();
}));
hotkeys.start()?;
hotkeys.set_bindings(vec![Binding {
    id: "push-to-talk".into(),
    chord: Chord { buttons: vec![Button::keyboard(0xA2 /* left Ctrl */), Button::keyboard(0x20 /* Space */)] },
    suppress: Some(true), // the focused app does not see the keys
    trigger_cooldown_ms: None,
}])?;
```

- The app hears them as `on("hotkey", (k) => …)` from `"native:voice"`, a [native module](/embedding/native-modules/) event.
- Keyboard buttons are Windows virtual-key codes on every platform; mouse buttons are device 1 (1 left, 2 right, 3 middle, 4 and 5 the side buttons).
- On macOS global hotkeys need the Accessibility permission. `status().accessibility_granted` tells whether it is there, and `request_accessibility()` shows the system prompt; call it only from a user action.
- `capture_once` records the next chord the user presses, for a "press a key" settings field.

## Single instance and deep links

`sk.config.json` can ask for one running instance (`"singleInstance"`) and register URL schemes (`"protocols"`). In Skathi's own exe the host applies both. In yours, `desktop_launch(true)` lets the builder apply them to your exe's real command line:

```rust
let builder = skathi::Builder::new()
    .payload_file("app/dist/release/.sk/app.skpk")
    .desktop_launch(true);
match builder.spawn() {
    Ok(ui) => std::process::exit(if ui.wait().clean { 0 } else { 1 }),
    // A second launch handed its arguments (or its notes://… link) to the first and ends here.
    Err(skathi::Error::LaunchExit(code)) => std::process::exit(code),
    Err(e) => {
        eprintln!("{e}");
        std::process::exit(2)
    }
}
```

The running instance receives the second launch's arguments and links through [`appLaunch`](/app-api/app-launch/), and its window comes to the front. Without `desktop_launch`, your exe decides itself whether a second copy may run.

## The macOS menu bar

On macOS Skathi installs the standard menu bar when its window opens: Quit, Hide, Close Window, Enter Full Screen, Minimize and Zoom, with their usual shortcuts. If your exe installs menus of its own, turn this off so they are not replaced:

```rust
let builder = skathi::Builder::new().app_menu(false);
```

## A backdrop the page cannot change

A program that composes content of its own under the window, such as a video layer, depends on the window staying transparent. `Builder::backdrop` fixes the main window's backdrop: it replaces `window.backdrop` of the payload's configuration, and the page's `appWindow.backdrop = …` is not applied and prints an advisory.

```rust
let builder = skathi::Builder::new().backdrop("transparent");
```

The names are those of [`backdrop`](/window-api/config-window-backdrop/) in sk.config.json; another name makes `spawn()` return `Error::Backdrop`. Pop-out windows keep the backdrop their `<SkWindow>` asks for.

## The frame source on macOS

A macOS window gets the drawables it draws into in one of two ways: from a `CAMetalDisplayLink` update, which hands out a drawable early enough for the frame to reach the screen on time, or from the layer's `nextDrawable`, which waits for one when all are in use. `Builder::metal_frame_source` chooses, for every window:

```rust
let builder = skathi::Builder::new().metal_frame_source(skathi::MetalFrameSource::NextDrawable);
```

| Value                | What it does                                                                                                                                                                                                                                                                |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Auto` (the default) | The display link paces frames. A resize, an activation, a surface frame after a pause and a view moved to another window are drawn at once with `nextDrawable`; so is a covered window that keeps drawing, and a link that stops delivering is replaced by `CVDisplayLink`. |
| `DisplayLink`        | Only the display link. Those frames wait for its next update instead.                                                                                                                                                                                                       |
| `NextDrawable`       | `CVDisplayLink` paces draws into the layer's `nextDrawable`. Both paced and synchronous frames present with their Core Animation transaction.                                                                                                                               |

What a pinned source costs, from what each path does:

- **`DisplayLink`.** During a live resize the frame at the new size is drawn after the window has taken that size, not in the same Core Animation transaction, so an edge can show the previous frame stretched for up to a refresh. The first frame after an activation and a surface frame after a pause come up to one refresh later. A covered window that asked to keep drawing gets only the frames the link still delivers while covered, which can be none. A link that stops delivering is created again rather than replaced. Without `CAMetalDisplayLink` (macOS 13) the window uses `nextDrawable`.
- **`NextDrawable`.** A frame that finds every drawable in use waits on the UI thread for the compositor to release one, and present feedback has no target time from a link, so frame pacing under load can be worse than with the display link. macOS 13 also obtains its drawables from the layer. Presenting every frame with the transaction applies when the embedder explicitly selects `NextDrawable`.

On a MacBook Pro with an M3 Max and a 120 Hz display, a player with a native video layer and a custom fullscreen transition saw a half-second fence timeout on entry. The gap between page frames exceeded 450 ms in 5 of 12 entries before the fix and 0 of 21 with transactional presentation. The longest remaining gap was 215 ms. [Fullscreen measurements and test conditions](https://github.com/argon-chat/skathi-gpui/pull/4)

Resident `CAMetalLayer Display Drawable` buffers in `footprint -v`, on an Apple Silicon Mac mini with macOS 26.4, a 1600 × 1000 window at scale 1, with the screen locked:

| Value          | Repainting every frame | Idle | After a resize        |
| -------------- | ---------------------- | ---- | --------------------- |
| `Auto`         | 3                      | 3    | 3 (2 of the old size) |
| `DisplayLink`  | 4                      | 4    | 4 (2 of the old size) |
| `NextDrawable` | 3                      | 3    | 3 (2 of the old size) |

A locked screen counts as covering the window, so `Auto` drew through `nextDrawable` there and measured as `NextDrawable`; a visible window under `Auto` switches between the two paths and kept four or five on another machine (see [GPU frames, Windows and macOS](/embedding/native-frames/#gpu-frames-windows-and-macos)). Under `DisplayLink` the link delivered only a few frames a second while locked. Measure your own window before you pin a source. It does nothing on Windows and Linux.

Next: [Dev loop and devtools](/embedding/dev-loop/).
