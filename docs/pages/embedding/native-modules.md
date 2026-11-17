---
title: Native modules
description: The everyday way an embedded app and its Rust program talk. Builder::module gives the app typed functions, worker tasks and events; the app imports them from "native:<name>", with TypeScript types generated from your Rust types.
section: Embedding
order: 2
---

New in 0.4.0. Your program gives the app a **native module**: Rust closures with ordinary typed parameters on one side, an `import` on the other. No names in strings, no JSON by hand, and the app's TypeScript knows every signature because it is generated from your Rust types.

```rust title="src/main.rs"
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use skathi::specta; // the Type derive names it

#[derive(Clone, Serialize, Deserialize, skathi::Type)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
}

#[derive(Deserialize, skathi::Type)]
pub struct SearchOptions {
    pub limit: Option<u32>,
}

#[derive(Serialize, skathi::Type)]
pub struct SyncProgress {
    pub done: u32,
    pub total: u32,
}

fn main() {
    let notes: Arc<Mutex<HashMap<u64, Note>>> = Arc::default();
    let (by_id, by_text) = (notes.clone(), notes.clone());
    let ui = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .module("notes", move |m| {
            m.function("get", move |id: u64| -> Result<Note, String> {
                by_id.lock().unwrap().get(&id).cloned().ok_or(format!("no note {id}"))
            })
            .params(["id"]);
            m.task("search", move |text: String, options: SearchOptions| {
                let limit = options.limit.unwrap_or(50) as usize;
                let notes = by_text.lock().unwrap();
                let found = notes.values().filter(|n| n.body.contains(&text));
                found.take(limit).cloned().collect::<Vec<Note>>()
            })
            .params(["text", "options"]);
            m.event::<SyncProgress>("progress");
        })
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

- **`m.function`** runs inside the call, on the UI thread, and returns a value.
- **`m.task`** runs on a worker thread and returns a `Promise`.
- **`m.event`** declares something your program tells the app; `Handle::emit_to` sends it from any thread, `on` receives it.

The module object exists before any app code runs, so a top-level `import` sees it at once. Add `serde` (with its `derive` feature) to your `Cargo.toml`; `skathi` re-exports what the `Type` derive needs.

## Functions return a value now

A function's parameters are its JavaScript arguments, each read with `serde` into the type you declared; its result is whatever `Serialize` makes of it, handed to the app as an ordinary object, array, string or number. The app never sees JSON text.

```rust
m.function("slug", |title: String, max: Option<usize>| -> Result<String, String> {
    let words: Vec<String> = title.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Err("a note needs a title".into());
    }
    Ok(words.join("-").chars().take(max.unwrap_or(40)).collect())
})
.params(["title", "max"]);
```

- **Arguments by position.** `slug("Groceries for Sunday")` returns `"groceries-for-sunday"`. A missing trailing argument is `null`, so an `Option<T>` parameter like `max` may be left out; extra arguments are ignored. Up to eight parameters; a struct parameter takes an options object.
- **A wrong argument throws.** `slug(42)` throws a `TypeError` that names the function and the argument: ``native:notes.slug: argument 1: invalid type: integer `42`, expected a string``.
- **`Err` throws.** Return `Result<T, E>` and an `Err` throws an `Error` in the app. A `String` error is its message. A struct error goes along as `error.data`, and its `message` field, if it has one, becomes the message. `E` must be `Serialize`: turn an `anyhow::Error` into text with `.map_err(|e| e.to_string())`.
- **A panic throws too.** The app gets an `Error` that says the function panicked, and keeps running.
- **Names are JS names.** An export is named like a JS function, `getNote` or `get_note`, and not a keyword such as `delete`; `spawn` refuses a module with a name it cannot export (`Error::Module`).

A function holds up the UI while it runs: nothing paints and no input is handled. A lookup in memory or a small computation is fine. Anything that touches a disk, a network, a lock another thread holds, or takes more than a millisecond or two, is a task.

## Tasks do the slow work

A task has the same shape as a function and runs on a worker thread. The app gets a promise; an `Err` or a panic rejects it.

```rust
m.task("saveAs", |note: Note, path: String| -> Result<u64, String> {
    let text = format!("# {}\n\n{}\n", note.title, note.body);
    std::fs::write(&path, &text).map_err(|e| format!("{path}: {e}"))?;
    Ok(text.len() as u64)
})
.params(["note", "path"]);
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

## Events tell the app

Declare each event with its payload type, then send it from any thread with the `Handle`:

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

- Delivery happens in the app's next frame, in order. Nothing is queued for later: an event sent while nothing listens is dropped.
- `on(event, handler)` returns a function that removes the handler; `off(event, handler)` does the same. A reload drops every handler.
- `emit_to` refuses an event the module did not declare (`EmitError::NotDeclared`), data `serde_json` cannot write (`EmitError::NotJson`) and anything after the app closed (`EmitError::Closed`).
- `on` with an event the module did not declare throws a `TypeError` that lists the ones it did.

Events are for news. When the app needs an answer, it calls a function or a task.

> **Tip:** An event per item of a fast stream (log lines, sensor samples) costs a round of JSON per item. Batch them per frame, or use a [native namespace](/embedding/native-api/) with the dispatcher.

## Types for the app

Every parameter, result and event type derives `skathi::Type` beside serde's derives. From those, your executable describes its modules in TypeScript, and the CLI writes them into the app:

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
- Serde's attributes count: `#[serde(rename_all = "camelCase")]` renames the fields in the declarations as on the wire, and an enum of unit variants becomes a union of strings.
- Every Rust number is a TypeScript `number`. A `u64` above 2^53 loses precision on the way, as it would in any JSON: send such ids as strings.
- `.params(["id"])` names the parameters; without it they are `arg1`, `arg2`, …
- A `Result` return declares its `Ok` type: `get` returns `Note`, and an `Err` is a throw.
- The `types` feature (on by default) is what pulls in [specta](https://docs.rs/specta). Off, the derive is gone, `sk types` says the exe was built without it, and the modules work the same.

The `Type` derive names the `specta` crate. `use skathi::specta;` in the module that derives saves a dependency; adding `specta` to your `Cargo.toml` at skathi's version works too.

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

|                   | Runs on             | May block?       | The app gets                      |
| ----------------- | ------------------- | ---------------- | --------------------------------- |
| `m.function`      | the Skathi thread   | no: it is the UI | a value, or a throw               |
| `m.task`          | a worker thread     | yes              | a promise                         |
| `Handle::emit_to` | any thread          | it never blocks  | delivered in the app's next frame |
| an `on` handler   | the app's JS thread | no: it is the UI | —                                 |

The low-level way in is still there: [Native APIs](/embedding/native-api/) build JS objects directly in the engine, for hot calls and streams.

## Deprecated: appHost.call

Before 0.4.0 the two sides talked through `appHost`: `Builder::function(name, handler)` with JSON text in and out, called as `appHost.call(name, ...args)`, and `Handle::emit(event, json)` heard by `appHost.on(event, …)`. Both still work. The Rust compiler marks them deprecated, and the app prints a `[skathi compat]` advisory once for each. They will be removed in a later release.

| Before                                        | Now                                                             |
| --------------------------------------------- | --------------------------------------------------------------- |
| `.function("notes.get", \|call\| …)`          | `.module("notes", \|m\| { m.function("get", \|id: u64\| …); })` |
| `serde_json::from_str(call.args())`           | typed parameters                                                |
| `call.into_reply()` and a thread              | `m.task`                                                        |
| `appHost.call<Note>("notes.get", 42)`         | `get(42)` from `"native:notes"`                                 |
| `ui.emit("sync.progress", &json.to_string())` | `ui.emit_to("notes", "progress", &progress)`                    |
| `appHost.on("sync.progress", …)`              | `on("progress", …)` from `"native:notes"`                       |

`appHost.available` stays: it tells whether a program embeds the app at all.

Next: [Native APIs](/embedding/native-api/), for when a module's JSON is not enough.
