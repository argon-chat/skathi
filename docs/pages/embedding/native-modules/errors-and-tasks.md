---
title: Errors, tasks and threads
description: What the app sees when a native module function returns Err, panics or gets the wrong argument; how #[task] runs slow work on a worker, on Skathi's threads or your own executor; and what a reload does to work in flight.
section: Embedding
order: 2.2
---

A [native module](/embedding/native-modules/) function either returns a value or fails, and a failure in Rust is always an `Error` the app can catch: never a crash, never a hang. This page lists each way a call can fail and what JavaScript gets for it, then how tasks keep slow work off the UI thread.

## What JS sees

```rust
#[skathi::data]
pub struct Quota {
    pub message: String,
    pub limit: u32,
}

#[skathi::module]
impl Tools {
    pub fn slug(&self, title: String, max: Option<usize>) -> Result<String, String> {
        let words: Vec<String> = title.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Err("a note needs a title".into());
        }
        Ok(words.join("-").chars().take(max.unwrap_or(40)).collect())
    }

    pub fn reserve(&self, count: u32) -> Result<u32, Quota> {
        if count > 10 {
            return Err(Quota { message: format!("{count} is over the limit"), limit: 10 });
        }
        Ok(count)
    }

    pub fn divide(&self, a: i32, b: i32) -> i32 {
        a / b
    }
}
```

| What happens in Rust                                       | A function                                                                       | A `#[task]`                                   |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------- | --------------------------------------------- |
| `Ok(v)`, or a plain return value                           | returns `v`                                                                      | resolves with `v`                             |
| `Err("text")` with a `String` error                        | throws `Error`, `message` is the text                                            | rejects with the same `Error`                 |
| `Err(e)` with a struct error                               | throws `Error`, `e` in `error.data`; its `message` field, if any, is the message | rejects with the same `Error`                 |
| a panic                                                    | throws `Error`: `native:tools.divide panicked: attempt to divide by zero`        | rejects with the same `Error`                 |
| a wrong argument                                           | throws `TypeError`, naming the function and the argument                         | rejects with the `TypeError`                  |
| a getter or `toJSON` in an argument throws                 | that exception, as it was thrown                                                 | thrown from the call, before any promise      |
| the result cannot be serialized (a map with tuple keys, …) | throws `Error`: `the result cannot be serialized: …`                             | rejects with that `Error`                     |
| the app is not embedded, or the module is not registered   | throws `Error`, saying which; check `available`                                  | throws the same, before any promise           |
| your executor drops the job without running it             | —                                                                                | rejects: `the task was dropped before it ran` |

In the app:

```ts
import { reserve, slug } from "native:tools";

slug("Groceries for Sunday"); // "groceries-for-sunday"
slug("Groceries for Sunday", 9); // "groceries"

try {
  slug(42 as unknown as string);
} catch (e) {
  // TypeError: native:tools.slug: argument 1: invalid type: integer `42`, expected a string
}

try {
  reserve(50);
} catch (e) {
  const error = e as Error & { data?: { limit: number } };
  error.message; // "50 is over the limit"
  error.data?.limit; // 10
}
```

- **`E` must be `Serialize`.** A `String` is the usual choice. Turn an `anyhow::Error` or an `io::Error` into text with `.map_err(|e| e.to_string())`, and add context while you are there: `format!("{path}: {e}")` reads better in a log than `Access is denied`.
- **A struct error carries data.** Use it when the app has to act on the failure, not just show it: a limit, a retry delay, a field name. Give it a `message` field so the `Error` reads well in a console.
- **A panic does not take the app down.** It is caught at the call, the app gets an `Error`, and both sides keep running. Your panic hook still runs first (the default one prints to stderr). A panic is still a bug: return an `Err` for failures you expect.
- **The generated types declare the `Ok` type.** `slug` returns `string` in TypeScript; an `Err` is a throw, not a return value.

## Tasks do the slow work

A function holds up the UI while it runs: nothing paints and no input is handled. Mark a method `#[task]` and it runs on a worker thread; the app gets a promise, which an `Err` or a panic rejects.

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

When to make something a task:

- **It touches a disk, a network or another process**, even if it is usually fast: the slow case is the one the user notices.
- **It waits for a lock another thread holds** for longer than a moment.
- **It computes for more than a millisecond or two.** A frame is 16 ms, or 7 at 144 Hz.

What stays a function: reading state the module already holds (a snapshot behind a `Mutex`), a quick computation, anything the app wants as a plain value in a `computed`.

### How a task runs

1. **The arguments are read in the call**, on the UI thread, before the task is queued. A wrong argument rejects the promise at once, and the worker gets owned Rust values: that is why a task cannot borrow `&str` or `&[u8]`.
2. **The method runs on a worker.** It has `&self` like any method, shared with the UI thread and the other workers, so state it changes is behind a `Mutex` or an atomic.
3. **The result is serialized on the worker**, so a large result costs the UI thread nothing but building the JS values. A `Vec<u8>` result hands its memory over without a copy.
4. **The promise settles in a later frame**, in the order results arrive.

Tasks run on four worker threads Skathi owns. A program with its own pool hands them over instead:

```rust
let builder = skathi::Builder::new().task_executor(|job| {
    std::thread::spawn(job);
});
```

`job` is a `Box<dyn FnOnce() + Send>`: give it to `rayon::spawn`, to a tokio runtime's `spawn_blocking`, or to a thread of yours. A job your executor drops without running rejects its promise, so the app never waits forever.

Four workers are plenty for work that computes. Tasks that mostly wait, on a network or a slow device, can use them all up; give those an executor with more threads, or keep a thread of your own and report back with an [event](/embedding/native-modules/events/).

### Long work and progress

A task returns once. For work that reports as it goes, a sync of a thousand files or a download, start it with a task (or a function that hands it to a thread of yours) and send progress as [events](/embedding/native-modules/events/):

```rust
#[skathi::module(events(progress = SyncProgress))]
impl Syncer {
    #[task]
    pub fn sync_all(&self, total: u32) -> u32 {
        for done in 1..=total {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if let Some(ui) = self.ui.get() {
                let _ = ui.emit_to("sync", "progress", &SyncProgress { done, total });
            }
        }
        total
    }
}
```

The app awaits the task for the result and listens to `progress` for the bar. [Events](/embedding/native-modules/events/#from-inside-a-module) shows where `self.ui` comes from.

## Reloads and shutdown

- **A reload drops the promises still waiting.** Their tasks run to the end on the worker, and the results are discarded: the page that asked is gone. A task that must not run for a page that no longer exists, such as one that opens a URL, checks its own state first.
- **A reload also drops every `on` handler** and the events still queued for them. The new page subscribes again as it starts.
- **At shutdown** a task still running finishes on its worker, and its result goes nowhere. Stop your own threads after `Handle::wait` returns.

Next: [Events](/embedding/native-modules/events/).
