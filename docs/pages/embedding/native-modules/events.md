---
title: Events
description: Send news from Rust to the app with Handle::emit_to, from your own threads or from inside a native module; how delivery works, why an event nobody hears is dropped, and the state-plus-event pattern that covers it.
section: Embedding
order: 2.3
since: "0.4"
---

Functions and tasks answer the app. Events are the other direction: your program tells the app something happened, a download finished, a device appeared, a setting changed elsewhere. A [native module](/embedding/native-modules/) declares the events it sends, with a payload type each; `Handle::emit_to` sends one from any thread, and the app subscribes with `on`.

## Declare, send, listen

Declare each event with its payload type in the attribute, `events(progress = SyncProgress)`, then send it with the `Handle` that `spawn` returned:

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

- **The names are strings in Rust**, the module's name and the event's: `emit_to("notes", "progress", …)`. A typo is a runtime error, `EmitError::NotDeclared`, so keep the names in one place (constants, or an enum with a method that returns the pair).
- **An event name is used as written**, not camelCased like a function: `events(window_changed = …)` is `"window_changed"` in the app. Write `events(windowChanged = …)` for the app's spelling, or a string for one that is not a Rust identifier: `events("sync-done" = Summary)`.
- **The payload can be any type a result can be**: a `#[skathi::data]` struct, an `Option` (the app gets `null`), a number, a `Vec`.
- **`on(event, handler)` returns a function that removes the handler**; `off(event, handler)` does the same. Removing twice is harmless, and the same handler added twice is called once.
- **`on` with an event the module did not declare throws a `TypeError`** that lists the ones it did, so a typo in the app shows at once. TypeScript catches it earlier: the generated `on` takes only the declared names, and types each handler's data.

## From inside a module

A module often sends its own events: the updater that finished a download, the voice engine whose device list changed. But the module is registered on the `Builder` before there is a `Handle`, so it cannot take one in its constructor. Give it a slot and fill the slot in `setup`, which runs before any app code with a `HostContext` whose `handle()` is the running app's:

```rust
use std::sync::{Arc, OnceLock};

#[skathi::data]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub version: String,
    pub received: u64,
    pub total: u64,
}

#[derive(Default)]
pub struct Updater {
    ui: Arc<OnceLock<skathi::Handle>>,
}

#[skathi::module(events(progress = Download, ready = String))]
impl Updater {
    #[task]
    pub fn download(&self, version: String) -> Result<(), String> {
        let total = 4 << 20;
        for step in 1..=4u64 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let received = step << 20;
            self.emit("progress", &Download { version: version.clone(), received, total });
        }
        self.emit("ready", &version);
        Ok(())
    }

    fn emit<T: skathi::serde::Serialize>(&self, event: &str, data: &T) {
        if let Some(ui) = self.ui.get() {
            let _ = ui.emit_to("updater", event, data);
        }
    }
}

fn main() {
    let updater = Updater::default();
    let slot = updater.ui.clone();
    let builder = skathi::Builder::new()
        .module("updater", updater)
        .setup(move |host| {
            let _ = slot.set(host.handle());
            Ok(())
        });
    // … payload, spawn and wait as usual
}
```

- **The slot is shared before the module moves.** `Builder::module` takes the module by value, so clone the `Arc` around the slot first. A program whose modules share one state (see [several modules, one state](/embedding/native-modules/#several-modules-one-state)) keeps the handle in that state instead.
- **`setup` is the one place every platform gives you a handle.** On macOS `run_on_main_thread` returns none.
- **Before `setup` there is no handle**, so code that may run that early checks the slot, as `emit` does, and drops the event: nothing in the app could have heard it.
- **Threads of yours** take a clone of the handle the same way; `Handle` is cheap to clone and every method works from any thread.
- `emit` is a private method, so it stays Rust-only; only `pub fn`s are exported.

## How delivery works

- **At once, in order.** `emit_to` serializes the data on your thread and queues it; it never blocks, and the app's thread only builds the JS values. The app's thread picks the queue up between frames without waiting for the next one, and delivers at most a frame's budget of items at a time; the rest follow right after.
- **An event nobody listens to is dropped.** Nothing is kept for a handler that subscribes later. The same goes for events sent while the page reloads: they belonged to the old page.
- **A handler that throws** does not stop the others. Its error and stack go to your program's stderr as `[native:notes] a handler for "progress" threw: …`.
- **Events emitted before the app has loaded** wait for it, then follow the rule above: delivered if a handler is subscribed by then.
- **`emit_into` gives the data away.** It is `emit_to` taking the data by value, so a [`NativeBuffer`](/embedding/native-modules/values/) in it, a video frame or a block of samples, reaches the app in its own memory instead of as a copy. It refuses the same things.
- **`emit_to` refuses** an event the module did not declare (`EmitError::NotDeclared`), data that cannot be serialized, such as a map whose keys are not strings or numbers (`EmitError::NotJson`), and anything after the app closed (`EmitError::Closed`). `Closed` is the normal way a thread of yours learns to stop.

## State plus an event

Because events are dropped when nobody listens, an event alone cannot tell a page what is true _now_: a component mounted later, or a page after a reload, missed everything before it subscribed. Pair each event with a function that returns the current state:

```rust
#[skathi::data]
#[derive(Clone)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub title: String,
    pub since_ms: u64,
}

pub struct PresenceModule {
    current: std::sync::Mutex<Option<Presence>>,
}

#[skathi::module(events(changed = Option<Presence>))]
impl PresenceModule {
    pub fn current(&self) -> Option<Presence> {
        self.current.lock().unwrap().clone()
    }
}
```

```ts
import { current, on, type Presence } from "native:presence";

export function watchPresence(apply: (presence: Presence | null) => void): () => void {
  const off = on("changed", apply);
  apply(current());
  return off;
}
```

Subscribe first, then read: a change that lands between the two is delivered after `apply(current())`, so the last call always has the newest state. The writer updates the state and then emits, under the same rule.

For something the app must handle exactly once, such as a link the OS asked the app to open, keep a queue in Rust instead: send the item's id in the event, and let the app claim it with a function that removes it from the queue (`take(id)`, returning `null` when another path already took it). A `pending()` function drains whatever arrived before the app subscribed.

## Many events

Each event is a delivery: about 0.6 µs from `emit_to` to the handler in a burst. That is nothing for state changes, and adds up for a stream of log lines or sensor samples. Batch them, a `Vec` per frame or per 50 ms, or use a [native API](/embedding/native-api/) with the dispatcher for a real stream.

Events are for news. When the app needs an answer, it calls a function or a task.

Next: [Types, tests and CI](/embedding/native-modules/types-and-testing/).
