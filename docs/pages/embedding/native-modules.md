---
title: Native modules
description: The everyday way an embedded app and its Rust program talk. #[skathi::module] turns an impl block into typed functions, worker tasks and events; the app imports them from "native:<name>", with TypeScript types generated from your Rust types.
section: Embedding
order: 2
---

New in 0.4.0. Your program gives the app a **native module**: an `impl` block of ordinary Rust methods on one side, an `import` on the other. No names in strings, no JSON by hand, and the app's TypeScript knows every signature because it is generated from your Rust.

```rust title="src/main.rs"
use std::collections::HashMap;
use std::sync::Mutex;

#[skathi::data]
#[derive(Clone)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
}

#[skathi::data]
pub struct SearchOptions {
    pub limit: Option<u32>,
}

#[skathi::data]
pub struct SyncProgress {
    pub done: u32,
    pub total: u32,
}

#[derive(Default)]
struct Notes {
    store: Mutex<HashMap<u64, Note>>,
}

#[skathi::module(events(progress = SyncProgress))]
impl Notes {
    pub fn get(&self, id: u64) -> Result<Note, String> {
        self.store.lock().unwrap().get(&id).cloned().ok_or(format!("no note {id}"))
    }

    #[task]
    pub fn search(&self, text: String, options: SearchOptions) -> Vec<Note> {
        let limit = options.limit.unwrap_or(50) as usize;
        let store = self.store.lock().unwrap();
        store.values().filter(|n| n.body.contains(&text)).take(limit).cloned().collect()
    }
}

fn main() {
    let ui = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .module("notes", Notes::default())
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
    std::process::exit(if ui.wait().clean { 0 } else { 1 });
}
```

```ts title="app/src/notes.ts"
import { get, on, search } from "native:notes";

export async function todos() {
  const first = get(42); // a Note, now; throws "no note 42" if there is none
  const rest = await search("todo", { limit: 20 }); // a Note[], computed on a worker
  return [first, ...rest];
}

export const stopWatching = on("progress", (p) => console.log(`${p.done} of ${p.total}`));
```

Three kinds of export, one rule each:

- **A `pub fn` taking `&self`** runs inside the call, on the UI thread, and returns a value.
- **The same with `#[task]`** runs on a worker thread and returns a `Promise`.
- **`events(name = Type)`** declares something your program tells the app; `Handle::emit_to` sends it from any thread, `on` receives it.

The module object exists before any app code runs, so a top-level `import` sees it at once. `skathi` is the only dependency this needs: `#[skathi::data]` brings serde and the type information along.

## What #[skathi::module] exports

- **Every `pub fn` that takes `&self`**, under its name in camelCase: `fetch_all` is `fetchAll` in the app. The parameters keep their names, camelCased the same way, in the generated types.
- **Nothing else.** Private methods and associated functions without `self`, such as `new`, stay Rust-only helpers.
- **`&self` only.** The value is moved into an `Arc` and shared by the UI thread and the task workers, so `&mut self` or `self` on a `pub fn` is a compile error: keep changing state behind a `Mutex` or an atomic. For the same reason the type must be `Send + Sync + 'static`.
- **Plain signatures.** Owned parameter types, named with plain identifiers, at most eight; no generics, no `async fn` (a `#[task]` is the slow path). The borrowed exceptions are `&str` and slices of numbers (`&[u8]`, `&[f32]`, …), which a function reads in place; a task takes the owned type instead. Each of these is a compile error that points at the parameter or method.
- **JS names.** A method named like a JS keyword (`delete`) or like a module's own exports (`available`, `on`, `off`) does not compile: rename it.

## #[skathi::data]

The attribute goes on every struct and enum that crosses: parameters, results and events. It derives serde's `Serialize` and `Deserialize` and, with the `types` feature, `skathi::Type`, all through `skathi`'s own paths, so your `Cargo.toml` needs no serde or specta.

- Your serde attributes stay in force: `#[serde(rename_all = "camelCase")]` renames the fields in the app and in the generated types alike.
- A `Vec<u8>` or `Option<Vec<u8>>` field is a `Uint8Array` in the app, not an array of numbers. A field with its own `#[serde(with = …)]` keeps it.
- A derive the type already has is not added again: `#[derive(Serialize)]` of your own serde keeps working beside it. Put `#[skathi::data]` above the derives so it sees them.

Deriving the type information yourself works too: `#[derive(serde::Serialize, skathi::Type)]` with `#[specta(crate = skathi::specta)]` on the type, since the derive otherwise looks for a `specta` crate of your own.

## Functions return a value now

A function's parameters are its JavaScript arguments, each read with `serde` into the type you declared; its result is whatever `Serialize` makes of it, handed to the app as an ordinary object, array, string or number. Both are read and built straight from the engine's own values: no JSON text is written or parsed, and a call with a few numbers costs about what a [native function](/embedding/native-api/) does.

```rust
#[skathi::module]
impl Tools {
    pub fn slug(&self, title: String, max: Option<usize>) -> Result<String, String> {
        let words: Vec<String> = title.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Err("a note needs a title".into());
        }
        Ok(words.join("-").chars().take(max.unwrap_or(40)).collect())
    }
}
```

- **Arguments by position.** `slug("Groceries for Sunday")` returns `"groceries-for-sunday"`. A missing trailing argument is `null`, so an `Option<T>` parameter like `max` may be left out; extra arguments are ignored. A struct parameter takes an options object.
- **A wrong argument throws.** `slug(42)` throws a `TypeError` that names the function and the argument: ``native:tools.slug: argument 1: invalid type: integer `42`, expected a string``.
- **`Err` throws.** Return `Result<T, E>` and an `Err` throws an `Error` in the app. A `String` error is its message. A struct error goes along as `error.data`, and its `message` field, if it has one, becomes the message. `E` must be `Serialize`: turn an `anyhow::Error` into text with `.map_err(|e| e.to_string())`.
- **A panic throws too.** The app gets an `Error` that says the function panicked, and keeps running.

A value means what it would in JSON, so the app sees the same thing whether it calls the module or posts the value to a server:

- **Numbers.** `NaN` and the infinities arrive as `null` (an `f64` parameter refuses them; an `Option<f64>` is `None`), and a result's `NaN` is `null`. `-0` is `0` to an integer. An integer type takes only a whole number in its range: `1.5` or `300` for a `u8` is a `TypeError`. An `f32` result is its shortest decimal: `0.1f32` is `0.1`, not `0.10000000149011612`.
- **What JSON leaves out.** A property that is `undefined`, a function or a symbol is missing, and an array element that is one is `null`. An object's `toJSON` is called, so a `Date` reaches a `String` parameter as its ISO text. A `BigInt` or an object that contains itself is a `TypeError`.
- **Extra arguments** are not read at all.

A function holds up the UI while it runs: nothing paints and no input is handled. A lookup in memory or a small computation is fine. Anything that touches a disk, a network, a lock another thread holds, or takes more than a millisecond or two, is a task.

## Tasks do the slow work

A task is a method marked `#[task]`. It runs on a worker thread, and the app gets a promise; an `Err` or a panic rejects it.

```rust
// In `impl Notes`, beside `get` and `search`:
#[task]
pub fn save_as(&self, note: Note, path: String) -> Result<u64, String> {
    let text = format!("# {}\n\n{}\n", note.title, note.body);
    std::fs::write(&path, &text).map_err(|e| format!("{path}: {e}"))?;
    Ok(text.len() as u64)
}
```

```ts
import { saveAs, type Note } from "native:notes";

export async function exportNote(note: Note, path: string): Promise<string> {
  try {
    return `saved ${await saveAs(note, path)} bytes`;
  } catch (e) {
    return (e as Error).message; // "D:/notes/42.md: Access is denied. (os error 5)"
  }
}
```

Tasks run on four worker threads Skathi owns. A program with its own pool hands them over instead:

```rust
let builder = skathi::Builder::new().task_executor(|job| {
    std::thread::spawn(job);
});
```

`job` is a `Box<dyn FnOnce() + Send>`: give it to `rayon::spawn`, to a tokio runtime's `spawn_blocking`, or to a thread of yours. A job your executor drops without running rejects its promise, so the app never waits forever.

A page reload drops the promises still waiting; their tasks finish and their results are discarded.

The arguments are read inside the call, on the UI thread, before the task is queued: a wrong argument rejects the promise at once, and the worker gets owned Rust values. The result is serialized on the worker and turned into JS values when the promise settles.

## Bytes

Binary data crosses as typed arrays, not as arrays of numbers:

```rust
#[skathi::module]
impl Images {
    pub fn checksum(&self, data: &[u8]) -> u32 {
        data.iter().map(|&b| u32::from(b)).sum()
    }

    #[task]
    pub fn thumbnail(&self, png: Vec<u8>, width: u32) -> Result<Vec<u8>, String> {
        shrink(&png, width).map_err(|e| e.to_string())
    }
}
```

```ts
import { checksum, thumbnail } from "native:images";

const file = new Uint8Array(await (await fetch("photo.png")).arrayBuffer());
checksum(file); // reads the app's memory where it is
const small: Uint8Array = await thumbnail(file, 256);
```

- **`&[u8]`** in a function reads the bytes in place, without a copy, from any typed array, `DataView` or `ArrayBuffer` (a view of 16-bit numbers is its bytes). The slice lives for the call; take `Vec<u8>` to keep the bytes.
- **`&[f32]`, `&[i16]`, `&[f64]`, …** read a typed array of exactly that kind in place: `&[f32]` from a `Float32Array`. Anything else that holds numbers, an array included, is copied into the slice's type, so the function works either way; only the copy costs.
- **`Vec<u8>`** as a parameter copies the bytes once. A task takes `Vec<u8>` (or `skathi::Bytes`), since it runs after the call: `&[u8]` on a `#[task]` does not compile.
- **`Vec<u8>` as a result** is a `Uint8Array`, also inside `Result` or `Option`. A task's result hands its memory to the app without a copy on the UI thread.
- An array of numbers still works where bytes are expected, one element at a time, and a `SharedArrayBuffer` is copied rather than read in place, since another thread may write to it.

serde has no way to tell a `Vec<u8>` from any other list, so the attribute does this only where it sees the type: a parameter, a result, a `#[skathi::data]` field. Elsewhere, such as inside a `Vec<Vec<u8>>` or a closure module, use `skathi::Bytes`, a `Vec<u8>` that serializes as bytes, or `#[serde(with = "skathi::bytes")]` on a field of your own type.

## Events tell the app

Declare each event with its payload type in the attribute, `events(progress = SyncProgress)`, then send it from any thread with the `Handle`:

```rust
let events = ui.clone();
std::thread::spawn(move || {
    for done in 1..=3 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let progress = SyncProgress { done, total: 3 };
        if events.emit_to("notes", "progress", &progress).is_err() {
            break; // the app has closed
        }
    }
});
```

```vue
<script setup lang="ts">
import { onScopeDispose, ref } from "vue";
import { on, type SyncProgress } from "native:notes";

const progress = ref<SyncProgress>({ done: 0, total: 0 });
onScopeDispose(on("progress", (p) => (progress.value = p)));
</script>

<template>
  <progress :value="progress.done" :max="progress.total" />
</template>
```

- An event name that is not a Rust identifier is a string: `events("sync-done" = Summary)`.
- Delivery happens in the app's next frame, in order. Nothing is queued for later: an event sent while nothing listens is dropped.
- `on(event, handler)` returns a function that removes the handler; `off(event, handler)` does the same. A reload drops every handler.
- `emit_to` refuses an event the module did not declare (`EmitError::NotDeclared`), data that cannot be serialized, such as a map whose keys are not strings or numbers (`EmitError::NotJson`), and anything after the app closed (`EmitError::Closed`).
- The data is serialized on the thread that sends it; the app's thread only builds the JS values.
- `on` with an event the module did not declare throws a `TypeError` that lists the ones it did.

Events are for news. When the app needs an answer, it calls a function or a task.

> **Tip:** An event per item of a fast stream (log lines, sensor samples) costs a delivery per item. Batch them per frame, or use a [native namespace](/embedding/native-api/) with the dispatcher.

## Types for the app

From the module's signatures and its `#[skathi::data]` types, your executable describes its modules in TypeScript, and the CLI writes them into the app:

```sh
cargo build
cd app && bunx sk types --exe ../target/debug/notes.exe
```

That writes `app/src/native-modules.d.ts`:

```ts title="app/src/native-modules.d.ts"
// The native modules of the program that embeds this app. Generated by `sk types`; do not edit.

declare module "native:notes" {
  export type Note = {
    id: number;
    title: string;
    body: string;
  };
  export type SearchOptions = {
    limit: number | null;
  };
  export type SyncProgress = {
    done: number;
    total: number;
  };
  export const available: boolean;
  export function get(id: number): Note;
  export function search(text: string, options: SearchOptions): Promise<Note[]>;
  export function on(event: "progress", handler: (data: SyncProgress) => void): () => void;
  export function off(event: "progress", handler: (data: SyncProgress) => void): void;
}
```

- [`sk dev --exe`](/embedding/dev-loop/) does the same every time it starts, and writes the file only when it changed. Commit it: the app then type-checks and builds without your Rust toolchain.
- Without it, TypeScript stops at the import with `Cannot find module 'native:notes'`. That is the reminder to run `sk types`.
- An enum of unit variants becomes a union of strings, and serde's renames apply.
- Every Rust number is a TypeScript `number`. A `u64` above 2^53 loses precision on the way, as it would in any JSON: send such ids as strings.
- Bytes are a `Uint8Array` in results and fields; a `&[u8]` or `Vec<u8>` parameter takes `ArrayBufferView | ArrayBuffer`, and `&[f32]` a `Float32Array`.
- A `Result` return declares its `Ok` type: `get` returns `Note`, and an `Err` is a throw.
- The `types` feature (on by default) is what pulls in [specta](https://docs.rs/specta). Off, `#[skathi::data]` derives serde only, `sk types` says the exe was built without it, and the modules work the same.

## Closures, for a few functions

A module without a type of its own, such as a couple of functions over values `main` already has, can be a closure that registers each export by hand:

```rust
let version = env!("CARGO_PKG_VERSION");
let builder = skathi::Builder::new().module("about", move |m: &mut skathi::Module| {
    m.function("version", move || version.to_owned());
    m.task("checksum", |path: String| -> Result<u32, String> {
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        Ok(bytes.iter().fold(0u32, |sum, b| sum.wrapping_mul(31).wrapping_add(u32::from(*b))))
    })
    .params(["path"]);
    m.event::<String>("updated");
});
```

- The closure's parameter needs its type, `&mut skathi::Module`: Rust cannot infer it from `module`, which also takes a `#[skathi::module]` type.
- `m.function`, `m.task` and `m.event` are what the attribute generates. The export name is the string, used as written.
- `.params([...])` names the parameters in the generated types; without it they are `arg1`, `arg2`, …
- A closure sees only serde types, so `Vec<u8>` is an array of numbers there: take and return `skathi::Bytes` for a `Uint8Array`.

## One app, with or without a program

The project still runs under `sk dev` and as its own exe, where nothing embeds it. The import does not break: every native module is there, `available` is `false`, each function throws (and a task's `await` rejects) with an `Error` that says the app is not embedded, and `on` never fires. Check first where the app should work both ways:

```ts
import { available, get, type Note } from "native:notes";

export function noteOrDraft(id: number): Note {
  return available ? get(id) : { id, title: "Draft", body: "" };
}
```

A module the program did not register behaves the same, with an error that names the module, so a typo in an import shows up at the first call.

## Where things run

|                    | Runs on             | May block?       | The app gets                      |
| ------------------ | ------------------- | ---------------- | --------------------------------- |
| a `pub fn`         | the Skathi thread   | no: it is the UI | a value, or a throw               |
| a `#[task]` method | a worker thread     | yes              | a promise                         |
| `Handle::emit_to`  | any thread          | it never blocks  | delivered in the app's next frame |
| an `on` handler    | the app's JS thread | no: it is the UI | —                                 |

The low-level way in is still there: [Native APIs](/embedding/native-api/) build JS objects directly in the engine, for hot calls and streams.

## Deprecated: appHost.call

Before 0.4.0 the two sides talked through `appHost`: `Builder::function(name, handler)` with JSON text in and out, called as `appHost.call(name, ...args)`, and `Handle::emit(event, json)` heard by `appHost.on(event, …)`. Both still work. The Rust compiler marks them deprecated, and the app prints a `[skathi compat]` advisory once for each. They will be removed in a later release.

| Before                                        | Now                                                   |
| --------------------------------------------- | ----------------------------------------------------- |
| `.function("notes.get", \|call\| …)`          | `pub fn get(&self, id: u64)` in a `#[skathi::module]` |
| `serde_json::from_str(call.args())`           | typed parameters                                      |
| `call.into_reply()` and a thread              | `#[task]`                                             |
| `appHost.call<Note>("notes.get", 42)`         | `get(42)` from `"native:notes"`                       |
| `ui.emit("sync.progress", &json.to_string())` | `ui.emit_to("notes", "progress", &progress)`          |
| `appHost.on("sync.progress", …)`              | `on("progress", …)` from `"native:notes"`             |

`appHost.available` stays: it tells whether a program embeds the app at all.

Next: [Native APIs](/embedding/native-api/), for JS objects of your own design.
