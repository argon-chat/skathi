---
title: Migrating from appHost.call
label: Migrating from appHost
description: Move an embedder from Builder::function, appHost.call and Handle::emit to native modules, one function at a time, with the before and after side by side.
section: Embedding
order: 2.5
---

Before 0.4.0 an embedded app and its program talked through `appHost`: `Builder::function(name, handler)` with JSON text in and out, called as `appHost.call(name, ...args)`, and `Handle::emit(event, json)` heard by `appHost.on(event, …)`. Both still work. The Rust compiler marks them deprecated, and the app prints a `[skathi compat]` advisory once for each. They will be removed in a later release.

[Native modules](/embedding/native-modules/) replace them with typed methods, worker tasks and declared events, several times faster for small calls and thousands of times faster for bytes, and with TypeScript generated from your Rust. The two work side by side, so you can move one function at a time.

## At a glance

| Before                                        | Now                                                   |
| --------------------------------------------- | ----------------------------------------------------- |
| `.function("notes.get", \|call\| …)`          | `pub fn get(&self, id: u64)` in a `#[skathi::module]` |
| `serde_json::from_str(call.args())`           | typed parameters                                      |
| `call.resolve(json)`, `call.reject(message)`  | return a value, or `Err(message)`                     |
| `call.into_reply()` and a thread              | `#[task]`                                             |
| `appHost.call<Note>("notes.get", 42)`         | `get(42)` from `"native:notes"`, typed                |
| always a `Promise`                            | a value for a function, a `Promise` for a task        |
| `ui.emit("sync.progress", &json.to_string())` | `ui.emit_to("notes", "progress", &progress)`          |
| `appHost.on("sync.progress", …)`              | `on("progress", …)` from `"native:notes"`             |
| hand-written TypeScript, if any               | `src/native-modules.d.ts` from `sk types --exe`       |

`appHost.available` stays: it tells whether a program embeds the app at all. Each module has its own `available` too.

## A function

Before: one handler per name, the arguments as a JSON array, the reply as JSON text.

```rust
#[allow(deprecated)]
fn old(builder: skathi::Builder, store: Arc<Mutex<HashMap<u64, Note>>>) -> skathi::Builder {
    builder.function("notes.get", move |call| {
        let args: Vec<serde_json::Value> = serde_json::from_str(call.args()).unwrap_or_default();
        let Some(id) = args.first().and_then(|v| v.as_u64()) else {
            return call.reject("notes.get needs an id");
        };
        match store.lock().unwrap().get(&id) {
            Some(note) => call.resolve(serde_json::to_string(note).unwrap()),
            None => call.reject(format!("no note {id}")),
        }
    })
}
```

```ts
const note = await appHost.call<Note>("notes.get", 42);
```

After: a method. The argument is read into a `u64` (a wrong one is a `TypeError` that names it), and the `Result` is the reply.

```rust
pub struct Notes {
    store: Arc<Mutex<HashMap<u64, Note>>>,
}

#[skathi::module]
impl Notes {
    pub fn get(&self, id: u64) -> Result<Note, String> {
        self.store.lock().unwrap().get(&id).cloned().ok_or(format!("no note {id}"))
    }
}

// On the builder: .module("notes", Notes { store })
```

```ts
import { get } from "native:notes";

const note = get(42); // no await: it is a function now
```

The `await` goes away. A function returns its value in the call, so code that did `await appHost.call(…)` still works if you leave the `await` (awaiting a value gives the value), but a `.then` chain on the result does not: return values are not promises.

## Slow work

Before: the handler took the reply and moved it to a thread of its own, so the UI thread was not held.

```rust
#[allow(deprecated)]
fn old_export(builder: skathi::Builder) -> skathi::Builder {
    builder.function("notes.export", |call| {
        let args: Vec<String> = serde_json::from_str(call.args()).unwrap_or_default();
        let reply = call.into_reply();
        std::thread::spawn(move || match args.first() {
            Some(path) => match std::fs::write(path, "…") {
                Ok(()) => reply.resolve("true"),
                Err(e) => reply.reject(format!("{path}: {e}")),
            },
            None => reply.reject("notes.export needs a path"),
        });
    })
}
```

After: `#[task]`, on Skathi's workers or [your executor](/embedding/native-modules/errors-and-tasks/#how-a-task-runs).

```rust
#[skathi::module]
impl Exporter {
    #[task]
    pub fn export_to(&self, path: String) -> Result<bool, String> {
        std::fs::write(&path, "…").map_err(|e| format!("{path}: {e}"))?;
        Ok(true)
    }
}
```

The app keeps its `await`: a task returns a promise.

## Events

Before: any name, JSON text you made yourself, heard by `appHost.on`.

```rust
#[allow(deprecated)]
fn old_progress(ui: &skathi::Handle, done: u32, total: u32) {
    let _ = ui.emit("sync.progress", &format!(r#"{{"done":{done},"total":{total}}}"#));
}
```

After: the event is declared on a module with its type, sent with `emit_to`, and the app's `on` knows the payload's type.

```rust
#[skathi::module(events(progress = SyncProgress))]
impl SyncEvents {}

fn new_progress(ui: &skathi::Handle, done: u32, total: u32) {
    let _ = ui.emit_to("sync", "progress", &SyncProgress { done, total });
}
```

```ts
import { on } from "native:sync";

on("progress", ({ done, total }) => console.log(`${done} of ${total}`));
```

A module with only events is fine: `impl SyncEvents {}` exports nothing but `available`, `on` and `off`. To send from inside a module, see [events from inside a module](/embedding/native-modules/events/#from-inside-a-module).

## Globals you installed yourself

Some programs skipped `appHost` and installed a global in `setup` with `HostContext::expose` and `js::function`: `globalThis.myApp.settings.get()`. That still works and is the right tool for a [native API](/embedding/native-api/), a hot path or a stream. For the ordinary calls, a module is less code and is typed:

- **Each namespace becomes a module**: `myApp.settings.get()` is `get()` from `"native:settings"`. Group by concern; the app imports what it uses.
- **`onChanged(callback)` methods become events.** A callback registry, protecting and unprotecting JS values, clearing it on reload: all of that goes. Declare `changed`, send it with `emit_to`, and the app uses `on("changed", …)`.
- **A subscription that replayed the current state** (`onChanged` calling the new callback at once) becomes [state plus an event](/embedding/native-modules/events/#state-plus-an-event): a `current()` function and the event, which the app reads in that order.
- **Hand-made promises** (a pending map, a worker that settles them in a frame hook) become tasks.
- **Argument checks** (`kind(value) != 5`, length limits, `Array.isArray` on Vue proxies) become types: a `String` parameter refuses a number by itself, a `#[skathi::data]` struct reads a Vue `reactive()` object as it is, and `deny_unknown_fields` refuses a misspelled key. Keep the checks that are about meaning, such as a range or a URL scheme, and return `Err` from them.
- **The global's `.d.ts`** is generated now; delete the hand-written interface and import the types from the module.

## In the app

- **Replace `appHost.call("x.y", …)`** with the import, one module at a time. TypeScript lists every caller of a function whose signature changed.
- **`appHost.on` handlers** become `on` from the module; the returned function still unsubscribes.
- **Code that checked `appHost.available`** can check the module's `available`: it also tells a module the program did not register.
- **Tests that stubbed a global** (`vi.stubGlobal("myApp", …)`) mock the module instead: `vi.mock("native:settings", …)`, with the [stand-in plugin](/embedding/native-modules/types-and-testing/#the-same-code-in-a-web-build) in the vitest config.

Next: [Native APIs](/embedding/native-api/), for JS objects of your own design.
