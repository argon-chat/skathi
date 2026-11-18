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

This page is the tour. The rest of the section goes deeper:

- [Values and types](/embedding/native-modules/values/): every Rust type and what it is in the app, borrowed strings and bytes, typed arrays, `#[skathi::data]`'s attributes, and what a call costs.
- [Errors, tasks and threads](/embedding/native-modules/errors-and-tasks/): what JS sees for an `Err`, a panic or a wrong argument, and how tasks run.
- [Events](/embedding/native-modules/events/): sending them from your threads and from the module itself.
- [Types, tests and CI](/embedding/native-modules/types-and-testing/): the generated `.d.ts`, testing a module, and keeping both honest in CI.
- [Migrating from appHost.call](/embedding/native-modules/migrating/): the old JSON functions, side by side with modules.

## Your first module, step by step

Say the app shows the files your program worked on recently and how much room a folder takes. Four steps get it there.

**1. Describe the values.** Every struct or enum that crosses gets `#[skathi::data]`. Rename its fields the way the app spells them:

```rust
#[skathi::data]
#[serde(rename_all = "camelCase")]
pub struct FolderSize {
    pub files: u32,
    pub total_bytes: u64,
}
```

**2. Write the methods.** A struct holds whatever state the module needs; an `impl` block with `#[skathi::module]` exports its `pub fn`s. Reading a list from memory is quick, so it is a plain function; walking a folder touches the disk, which can stall on a network drive, so it is a task:

```rust
pub struct Files {
    recent: std::sync::Mutex<Vec<String>>,
}

#[skathi::module]
impl Files {
    pub fn recent(&self) -> Vec<String> {
        self.recent.lock().unwrap().clone()
    }

    #[task]
    pub fn folder_size(&self, path: String) -> Result<FolderSize, String> {
        let mut size = FolderSize { files: 0, total_bytes: 0 };
        for entry in std::fs::read_dir(&path).map_err(|e| format!("{path}: {e}"))? {
            let meta = entry.and_then(|e| e.metadata()).map_err(|e| format!("{path}: {e}"))?;
            if meta.is_file() {
                size.files += 1;
                size.total_bytes += meta.len();
            }
        }
        Ok(size)
    }
}
```

**3. Register it.** One line on the builder names the module:

```rust
let builder = skathi::Builder::new().module(
    "files",
    Files {
        recent: std::sync::Mutex::new(Vec::new()),
    },
);
```

**4. Import it.** Build the exe, run `bunx sk types --exe <your exe>` in the app (or start it with [`sk dev --exe`](/embedding/dev-loop/)), and the app has `native:files`, typed:

```ts
import { folderSize, recent } from "native:files";

const files: string[] = recent();
const { totalBytes } = await folderSize("C:\\Users\\me\\Documents");
```

`folder_size` is `folderSize` in the app, and `total_bytes` is `totalBytes` because of `rename_all`. Its `Result` became a promise that rejects with the `Err` text.

## What #[skathi::module] exports

- **Every `pub fn` that takes `&self`**, under its name in camelCase: `fetch_all` is `fetchAll` in the app. The parameters keep their names, camelCased the same way, in the generated types.
- **Nothing else.** Private methods and associated functions without `self`, such as `new`, stay Rust-only helpers.
- **`&self` only.** The value is moved into an `Arc` and shared by the UI thread and the task workers, so `&mut self` or `self` on a `pub fn` is a compile error: keep changing state behind a `Mutex` or an atomic. For the same reason the type must be `Send + Sync + 'static`.
- **Plain signatures.** Owned parameter types, named with plain identifiers, at most eight; no generics, no `async fn` (a `#[task]` is the slow path). The borrowed exceptions are `&str` and slices of numbers (`&[u8]`, `&[f32]`, …), which a function reads in place; a task takes the owned type instead. Each of these is a compile error that points at the parameter or method.
- **JS names.** A method named like a JS keyword (`delete`) or like a module's own exports (`available`, `on`, `off`) does not compile: rename it.
- **Event names as written.** `events(window_changed = …)` is `"window_changed"` in the app, not camelCased like a function. Write the name the app should use: `events(windowChanged = …)`.

## Where things run

|                    | Runs on             | May block?       | The app gets                      |
| ------------------ | ------------------- | ---------------- | --------------------------------- |
| a `pub fn`         | the Skathi thread   | no: it is the UI | a value, or a throw               |
| a `#[task]` method | a worker thread     | yes              | a promise                         |
| `Handle::emit_to`  | any thread          | it never blocks  | delivered as soon as it is queued |
| an `on` handler    | the app's JS thread | no: it is the UI | —                                 |

A function holds up the UI while it runs: nothing paints and no input is handled. A lookup in memory or a small computation is fine. Anything that touches a disk, a network, a lock another thread holds, or takes more than a millisecond or two, is a task. [Errors, tasks and threads](/embedding/native-modules/errors-and-tasks/) has the details.

## Several modules, one state

A program usually has more than one concern: a voice engine, an updater, the tray. Give each its own module, so the app imports `native:voice` and `native:updater` rather than one bag of everything, and let them share the state behind an `Arc`:

```rust
use std::sync::Arc;

#[derive(Default)]
pub struct Services {
    pub volume: std::sync::atomic::AtomicU32,
    pub update_ready: std::sync::atomic::AtomicBool,
}

pub struct Voice(Arc<Services>);
pub struct Updater(Arc<Services>);

#[skathi::module]
impl Voice {
    pub fn volume(&self) -> u32 {
        self.0.volume.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[skathi::module]
impl Updater {
    pub fn ready(&self) -> bool {
        self.0.update_ready.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub fn register(builder: skathi::Builder, services: &Arc<Services>) -> skathi::Builder {
    builder
        .module("voice", Voice(services.clone()))
        .module("updater", Updater(services.clone()))
}
```

A function that registers every module, like `register`, is also what you call to [answer `sk types` early](/embedding/native-modules/types-and-testing/#answer-sk-types-before-anything-else).

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
- Each closure must be `Send + Sync`: wrap what is not (a `RefCell`, an `Rc`) or keep it on another thread and send to it.
- A closure sees only serde types, so `Vec<u8>` is an array of numbers there and `&str` is not accepted: take and return `skathi::Bytes` for a `Uint8Array`, and `String` for text.

## One app, with or without a program

The project still runs under `sk dev` and as its own exe, where nothing embeds it. The import does not break: every native module is there, `available` is `false`, each function throws (and a task's `await` rejects) with an `Error` that says the app is not embedded, and `on` never fires. Check first where the app should work both ways:

```ts
import { available, get, type Note } from "native:notes";

export function noteOrDraft(id: number): Note {
  return available ? get(id) : { id, title: "Draft", body: "" };
}
```

A module the program did not register behaves the same, with an error that names the module, so a typo in an import shows up at the first call.

This stand-in is Skathi's: its builders resolve `native:*`. A build of the same code by another tool, such as a Vite web build or vitest, needs one of its own; [Types, tests and CI](/embedding/native-modules/types-and-testing/#the-same-code-in-a-web-build) shows a ten-line plugin.

The low-level way in is still there: [Native APIs](/embedding/native-api/) build JS objects directly in the engine, for hot calls and streams.

Next: [Values and types](/embedding/native-modules/values/).
