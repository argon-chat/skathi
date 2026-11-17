---
title: Functions and events
description: The everyday way an embedded app and its Rust host talk. appHost.call and Builder::function, Handle::emit and appHost.on, answering from worker threads, errors, and an app that runs with or without a host.
section: Embedding
order: 2
---

Two directions, one rule each:

- **The app asks, Rust answers.** `appHost.call(name, ...args)` in the app runs the handler you registered with `Builder::function(name, …)` and returns a promise of its answer.
- **Rust tells, the app listens.** `Handle::emit(event, json)` in Rust reaches every `appHost.on(event, …)` handler in the app.

Everything crosses as JSON: arguments arrive as one JSON array, results and event data are one JSON value each. Functions, `undefined`, class instances and `Map`s do not survive the trip; plain objects, arrays, strings, numbers, booleans and `null` do.

## Functions the app calls

```rust
let builder = skathi::Builder::new().function("notes.get", |call| {
    let (id,): (u64,) = match serde_json::from_str(call.args()) {
        Ok(args) => args,
        Err(e) => return call.reject(format!("notes.get takes a note id: {e}")),
    };
    match NOTES.get(id) {
        Some(note) => call.resolve(serde_json::to_string(&note).unwrap()),
        None => call.reject(format!("no note {id}")),
    }
});
```

```ts
import { appHost } from "@skathi-native/runtime";

const note = await appHost.call<Note>("notes.get", 42);
```

- `call.args()` is the JSON array of arguments, `[42]` here. `serde_json` reads it straight into a tuple or a struct.
- `call.resolve(json)` settles the promise with one JSON value. Text that is not JSON rejects instead.
- `call.reject(message)` rejects it with an `AppHostError` whose `message` is yours and whose `function` names the call.
- `appHost.functions` lists the names you registered, so the app can tell what this host offers. Calling a name that is not there rejects at once with a `TypeError` that lists the ones that are.

Names are free-form. A dotted prefix per area (`notes.get`, `sync.start`) keeps a growing list readable.

### Keep the UI thread free

A handler runs **on the Skathi thread**, inside the JS call: while it runs, nothing paints and no input is handled. A lookup in memory is fine. Anything that touches a disk, a network, a lock another thread holds, or takes more than a millisecond or two, moves off:

```rust
.function("notes.search", move |call| {
    let args: (String, SearchOptions) = match serde_json::from_str(call.args()) {
        Ok(args) => args,
        Err(e) => return call.reject(format!("bad arguments: {e}")),
    };
    let reply = call.into_reply();
    let index = index.clone();
    pool.spawn(move || match index.search(&args.0, &args.1) {
        Ok(notes) => reply.resolve(serde_json::to_string(&notes).unwrap()),
        Err(e) => reply.reject(e.to_string()),
    });
})
```

`call.into_reply()` turns the call into a `Reply` that can go to any thread and settle the promise from there. A `Reply` dropped without an answer rejects, so a worker that panics or gives up never leaves the app waiting on a promise forever. While a call is unanswered, the app keeps running normally; `await` only suspends the code that called.

> **Tip:** A `tokio` or `async-std` runtime works the same way: move the `Reply` into the task, answer when it finishes.

### Errors the app sees

```ts
import { appHost, type AppHostError } from "@skathi-native/runtime";

try {
  await appHost.call("notes.get", 7);
} catch (e) {
  if (e instanceof Error && e.name === "AppHostError") {
    console.warn(`${(e as AppHostError).function} failed: ${e.message}`); // "notes.get failed: no note 7"
  }
}
```

| The call rejects with | When                                                                                      |
| --------------------- | ----------------------------------------------------------------------------------------- |
| `AppHostError`        | your handler rejected, its `Reply` was dropped, or it answered with text that is not JSON |
| `AppHostError`        | the page reloaded while the call was waiting                                              |
| `AppHostError`        | the app is not embedded (`appHost.available` is `false`)                                  |
| `TypeError`           | no function has that name; the message lists the ones that exist                          |
| `TypeError`           | the arguments are not JSON-serialisable                                                   |

### A typed client

`appHost.call` is untyped by nature. One module that names every function once keeps the rest of the app typed and the names in one place:

```ts title="src/host.ts"
import { appHost } from "@skathi-native/runtime";

export interface Note {
  id: number;
  title: string;
  body: string;
}

export const host = {
  getNote: (id: number) => appHost.call<Note>("notes.get", id),
  search: (text: string, limit = 50) => appHost.call<Note[]>("notes.search", text, { limit }),
  save: (note: Note) => appHost.call<void>("notes.save", note),
};
```

## Events you send

```rust
let ui = builder.spawn()?;
let events = ui.clone();
std::thread::spawn(move || {
    for progress in sync.progress() {
        let data = serde_json::json!({ "done": progress.done, "total": progress.total });
        if events.emit("sync.progress", &data.to_string()).is_err() {
            break; // the app has closed
        }
    }
});
```

```vue
<script setup lang="ts">
import { onScopeDispose, ref } from "vue";
import { appHost } from "@skathi-native/runtime";

const progress = ref({ done: 0, total: 0 });
const stop = appHost.on("sync.progress", (data) => {
  progress.value = data as { done: number; total: number };
});
onScopeDispose(stop);
</script>
```

- The `Handle` is cheap to clone and every method works from any thread.
- Delivery happens in the app's next frame, in order. Events emitted before the app is ready wait for it.
- `emit` refuses data that is not one JSON value (`EmitError::NotJson`; a bare string needs its quotes) and anything after the app closed (`EmitError::Closed`).
- `appHost.on` returns a function that removes the handler; `appHost.off(event, handler)` does the same. A reload drops every handler.

Events are for news. When the app needs an answer to a question, it calls a function instead.

> **Tip:** An event per item of a fast stream (log lines, sensor samples) costs a JSON round per item. Batch them per frame, or use a [native namespace](/embedding/native-api/) with the dispatcher.

## One app, with or without a host

The project still runs under `sk dev` and as its own exe, where nothing embeds it: `appHost.available` is `false`, `functions` is empty, `call` rejects and `on` never fires. Code that should work both ways checks first:

```ts
export async function userName(): Promise<string> {
  if (appHost.available && appHost.functions.includes("user.name")) return appHost.call<string>("user.name");
  return "Guest";
}
```

A typed client like the one above is a good place for that check, with a fake for each function while you work on the UI alone.

## Where calls run

|                             | Runs on             | May block?                       | Settles                           |
| --------------------------- | ------------------- | -------------------------------- | --------------------------------- |
| `Builder::function` handler | the Skathi thread   | no: hand off with `into_reply()` | `Call::resolve`, `Call::reject`   |
| `Reply`                     | any thread          | yes                              | `resolve`, `reject`, or on drop   |
| `Handle::emit`              | any thread          | it never blocks                  | delivered in the app's next frame |
| `appHost.on` handler        | the app's JS thread | no: it is the UI thread          | —                                 |

Next: [Native APIs](/embedding/native-api/), for when JSON in both directions is not enough.
