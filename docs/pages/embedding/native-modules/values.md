---
title: Values and types
description: What every Rust type is in the app and how it crosses, borrowed strings and bytes, typed arrays, the #[skathi::data] attributes, the direct conversion and its serde fallback, and what a call costs.
section: Embedding
order: 2.1
---

A [native module](/embedding/native-modules/) function's parameters are its JavaScript arguments, each read into the type you declared; its result is handed to the app as an ordinary object, array, string or number. No JSON text is written or parsed on either side: values are read and built straight from the engine's own. They still _mean_ what JSON would, so the app sees the same thing whether it calls the module or posts the value to a server.

## The type table

Each row is a Rust type as a parameter, a result or a field of a `#[skathi::data]` type, unless it says otherwise.

| Rust                                                                | TypeScript                                                   | How it crosses                                                                                                       |
| ------------------------------------------------------------------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| `bool`                                                              | `boolean`                                                    | directly                                                                                                             |
| `u8` … `u64`, `i8` … `i64`, `usize`, `isize`, `u128`                | `number`                                                     | directly; read only from a whole number in the type's range                                                          |
| `f32`, `f64`                                                        | `number`                                                     | directly; `NaN` and the infinities are `null`                                                                        |
| `char`                                                              | `string`                                                     | directly; one character                                                                                              |
| `String`                                                            | `string`                                                     | directly                                                                                                             |
| `&str` (parameter of a function)                                    | `string`                                                     | read for the length of the call, without a `String` of yours; not on a task                                          |
| `()`                                                                | `null`                                                       | a function that returns nothing returns `null`                                                                       |
| `Option<T>`                                                         | `T \| null`                                                  | directly; `null`, `undefined` and a missing property are `None`. A trailing `Option` parameter may be left out (`?`) |
| `Box<T>`                                                            | `T`                                                          | as `T`                                                                                                               |
| `Vec<T>`, `[T]` (result)                                            | `T[]`                                                        | directly; a parameter also reads a typed array as its numbers                                                        |
| `HashMap<String, V>`, `BTreeMap<String, V>`                         | `{ [key in string]: V }`                                     | directly                                                                                                             |
| a map with other keys, `HashMap<u32, V>`                            | `{ [key in number]: V }`                                     | through serde; the keys are text in the app, as in JSON                                                              |
| tuples `(A, B)`                                                     | `[A, B]`                                                     | through serde, as arrays                                                                                             |
| `Result<T, E>` (result)                                             | `T`                                                          | `Ok` is the value; `Err` throws (a task rejects). [What JS sees](/embedding/native-modules/errors-and-tasks/)        |
| `Vec<u8>`, `skathi::Bytes`                                          | `Uint8Array` (a parameter: `ArrayBufferView \| ArrayBuffer`) | copied once; see [Bytes](#bytes)                                                                                     |
| `Option<Vec<u8>>`                                                   | `Uint8Array \| null`                                         | as above                                                                                                             |
| `&[u8]` (parameter of a function)                                   | `ArrayBufferView \| ArrayBuffer`                             | read in place, not copied; not on a task                                                                             |
| `&[i8]`, `&[i16]`, `&[u16]`, `&[i32]`, `&[u32]`, `&[f32]`, `&[f64]` | `Int8Array`, …, `Float64Array`                               | read in place from a typed array of that kind, copied from anything else that holds numbers                          |
| `&[i64]`, `&[u64]`                                                  | `BigInt64Array`, `BigUint64Array`                            | as above                                                                                                             |
| `serde_json::Value`                                                 | `Value`, a JSON type declared in the module                  | through serde, as any JSON value; a typed array arrives as JSON would see it, an object of indexes                   |
| a `#[skathi::data]` struct                                          | an object type                                               | directly, field by field                                                                                             |
| a `#[skathi::data]` enum                                            | a union, by its [representation](#enums)                     | a result directly; an argument that is not a plain variant name through serde                                        |
| any other `Serialize`/`Deserialize` type                            | its specta type, if it has one                               | through serde, over the engine's values; it means the same, a little slower                                          |

A parameter type must be `Deserialize` and a result type `Serialize`, both with [specta](https://docs.rs/specta)'s `Type` when the `types` feature is on (it is by default). `#[skathi::data]` derives all three.

## Numbers

Every Rust number is a TypeScript `number`, a 64-bit float:

- **An integer parameter takes only a whole number in its range.** `1.5` or `300` for a `u8` throws a `TypeError`; `-0` is `0`.
- **Above 2^53, integers lose precision** on the way out, as they would in any JSON. Send ids that large as strings.
- **`NaN` and the infinities are `null`.** An `f64` parameter refuses them with a `TypeError`; an `Option<f64>` reads them as `None`; a result that is `NaN` arrives as `null`.
- **An `f32` result is its shortest decimal:** `0.1f32` is `0.1`, not `0.10000000149011612`.

## What JSON leaves out

A value means what it would in JSON, so the rules for awkward JS values are JSON's:

- A property that is `undefined`, a function or a symbol is missing, and an array element that is one is `null`.
- An object's `toJSON` is called, so a `Date` reaches a `String` parameter as its ISO text.
- A `BigInt`, or an object that contains itself, is a `TypeError`.
- A `Proxy`, such as a Vue `reactive()` object or array, reads like the object it stands for, so you can pass reactive state as it is.
- Extra arguments are not read at all.

Two differences from serde's own reading, both about values that are not JSON anyway: a `#[skathi::data]` struct reads only its own fields, so an object whose _other_ properties hold a `BigInt` or a cycle is accepted; and when several fields are wrong, the first one in the struct is reported, not the first one in the object.

## Borrowed parameters

A function, not a task, may borrow two kinds of parameter for the length of the call:

```rust
#[skathi::module]
impl Text {
    pub fn word_count(&self, text: &str) -> usize {
        text.split_whitespace().count()
    }

    pub fn peak(&self, samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
    }
}
```

- **`&str`** reads the app's string without a `String` of yours.
- **`&[u8]`** reads the memory of any typed array, `DataView` or `ArrayBuffer` in place, with no copy (a view of 16-bit numbers is its bytes).
- **`&[f32]`, `&[i16]`, `&[f64]`, …** read a typed array of exactly that kind in place: `&[f32]` from a `Float32Array`. Anything else that holds numbers, a plain array included, is copied into the slice's type, so the function works either way; only the copy costs.

The borrow ends when the function returns. To keep the data, copy it (`text.to_owned()`, `bytes.to_vec()`) or take the owned type instead. A `#[task]` runs after the call has returned, so a borrowed parameter on a task does not compile; the error says which owned type to take:

```text
error: a #[task] runs on a worker after the call has returned, so it cannot borrow from it: take `String` instead
```

Any other reference, `&String`, `&mut [u8]` or `&'static str`, is a compile error too. Borrowing is only for module methods; the closure form takes owned types.

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

- **`Vec<u8>` as a parameter** copies the bytes once. A task takes `Vec<u8>` (or `skathi::Bytes`), since it runs after the call.
- **`Vec<u8>` as a result** is a `Uint8Array`, also inside `Result` or `Option`. A task's result hands its memory to the app without a copy on the UI thread.
- **A field of a `#[skathi::data]` type** that is `Vec<u8>` or `Option<Vec<u8>>` is a `Uint8Array` too. A field with its own `#[serde(with = …)]` keeps it.
- An array of numbers still works where bytes are expected, one element at a time, and a `SharedArrayBuffer` is copied rather than read in place, since another thread may write to it.

serde has no way to tell a `Vec<u8>` from any other list, so the macros do this only where they see the type: a parameter, a result, a `#[skathi::data]` field. Elsewhere, such as inside a `Vec<Vec<u8>>`, a `HashMap<String, Vec<u8>>` or a closure module, use `skathi::Bytes`, a `Vec<u8>` that crosses as bytes, or `#[serde(with = "skathi::bytes")]` on a field of your own serde type:

```rust
#[skathi::data]
pub struct Attachments {
    pub names: Vec<String>,
    pub files: Vec<skathi::Bytes>,
}
```

Other numbers come out as plain arrays: a `Vec<f32>` result is a `number[]`, not a `Float32Array`.

## #[skathi::data]

The attribute goes on every struct and enum that crosses: parameters, results and events. It derives serde's `Serialize` and `Deserialize` and, with the `types` feature, `skathi::Type`, all through `skathi`'s own paths, so your `Cargo.toml` needs no serde or specta. It also gives the type a direct conversion to and from JS values, which is faster than serde's (see [what a call costs](#what-a-call-costs)).

- **Your serde attributes stay in force**, and rename the fields in the app and in the generated types alike.
- **A derive the type already has is not added again.** A type of yours that already derives `Serialize` keeps it. Put `#[skathi::data]` above the derives so it sees them.
- **Generic types work**, as long as the parameters you use them with do.

### The attributes it honors

The direct conversion follows these serde attributes exactly as serde does:

| Attribute                                            | On             | In the app                                                     |
| ---------------------------------------------------- | -------------- | -------------------------------------------------------------- |
| `rename = "…"`                                       | field, variant | that name                                                      |
| `rename_all = "camelCase"` (and serde's other cases) | struct, enum   | every field or variant renamed                                 |
| `skip`                                               | field          | never sent, never read (the field takes its `Default`)         |
| `skip_serializing`, `skip_deserializing`             | field          | only one way                                                   |
| `default`, `default = "path"`                        | field          | may be missing in an argument                                  |
| `skip_serializing_if = "Option::is_none"`            | field          | left out of a result when the function says so; optional in TS |
| `tag = "…"`, `content = "…"`, `untagged`             | enum           | the enum's [representation](#enums)                            |

```rust
#[skathi::data]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub session_id: String,
    pub title: String,
    #[serde(rename = "artist")]
    pub author: String,
    #[serde(default)]
    pub is_playing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip)]
    pub started: Option<std::time::Instant>,
}
```

That is `{ sessionId: string; title: string; artist: string; isPlaying: boolean; album?: string | null }` in the app. `started` stays in Rust.

### Enums

An enum of unit variants is a union of strings; one with data follows serde's representation:

```rust
#[skathi::data]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Downloading,
    Ready,
}

#[skathi::data]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Shape {
    Circle { radius: f64 },
    Rect { width: f64, height: f64 },
}
```

| Representation                       | A value in the app                             |
| ------------------------------------ | ---------------------------------------------- |
| unit variants                        | `"idle"`                                       |
| externally tagged (serde's default)  | `{ "Circle": { "radius": 2 } }`                |
| `#[serde(tag = "kind")]`             | `{ "kind": "circle", "radius": 2 }`            |
| `#[serde(tag = "t", content = "c")]` | `{ "t": "circle", "c": { "radius": 2 } }`      |
| `#[serde(untagged)]`                 | `{ "radius": 2 }`: the first variant that fits |

The generated TypeScript is the matching union, so a `switch (shape.kind)` narrows it. An enum _result_ converts directly; an enum _argument_ other than a plain variant name is read through serde, with the same meaning.

### When the conversion falls back to serde

A type whose serde attributes the direct conversion does not know (`flatten`, `deny_unknown_fields`, `with` on the type, `alias`, `default` on the whole type, `from`, `try_from`, …) gets no direct conversion and keeps serde's meaning exactly, read and written over the engine's values. Nothing changes for the app; it is a little slower. The same holds for a type with only `#[derive(Serialize, Deserialize)]`, for tuples, for maps with keys other than strings and for `serde_json::Value`.

So use the attributes you need. A settings object that must refuse unknown keys keeps `deny_unknown_fields`:

```rust
#[skathi::data]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanConfig {
    #[serde(default = "default_interval")]
    pub poll_interval_ms: u64,
    #[serde(default)]
    pub ignored: Vec<String>,
}

fn default_interval() -> u64 {
    1000
}
```

A misspelled `pollIntervalMS` from the app then throws a `TypeError` that names the field, instead of being ignored.

### Without the attribute

Deriving the type information yourself works too: `#[derive(serde::Serialize, skathi::Type)]` with `#[specta(crate = skathi::specta)]` on the type, since the derive otherwise looks for a `specta` crate of your own. Such a type crosses through serde.

## What a call costs

Two ways a value crosses, and the macros pick the faster one for each type on their own:

- **A `#[skathi::data]` type converts directly.** An argument object has its fields looked up by name, all in one call into the engine, with the names made into engine strings once and reused; each field is then read by its own type's conversion, so nested data types, `Vec`, `Option`, `Box` and maps with string keys compose. A result is written into one flat buffer that the engine turns into objects and arrays in a single call, and objects of the same fields share their engine layout. Numbers, strings, `bool` and bytes convert directly too.
- **Anything else goes through serde**, still over the engine's values rather than text.

Measured with a release build on Windows (a Ryzen 9 9950X, medians of five runs), against the same calls when values crossed as JSON text. The struct is `{ id: u32, name: String, tags: Vec<String>, score: Option<f64> }` with `#[skathi::data]`; a bare [native function](/embedding/native-api/) that adds two numbers takes 0.04 µs.

| Call                                               | As JSON text | Now      |
| -------------------------------------------------- | ------------ | -------- |
| `add(a: f64, b: f64) -> f64`                       | 0.51 µs      | 0.06 µs  |
| a 32-character `String` argument                   | 0.62 µs      | 0.08 µs  |
| a 1 MiB `Uint8Array` as a `Vec<u8>` argument       | 36 ms        | 0.015 ms |
| a 1 MiB `Vec<u8>` result                           | 13.5 ms      | 0.13 ms  |
| `flip(p: Point) -> Point`                          | 0.90 µs      | 0.27 µs  |
| 1000 structs as a result                           | 0.37 ms      | 0.24 ms  |
| 1000 structs as an argument                        | 0.72 ms      | 0.31 ms  |
| an event, from `emit_to` to the handler, in bursts | 1.2 µs       | 0.6 µs   |
| `emit_to` itself, on the sending thread            | 0.81 µs      | 0.42 µs  |

What this means in practice:

- **A call per frame, or a thousand a second, is free.** Shape the API for clarity, not to save calls.
- **Big binary data is cheap in place.** `&[u8]` reads a megabyte without copying it; `Vec<u8>` copies it once, which is still far under a millisecond.
- **Many small events cost more than one batch.** Send a frame's worth of samples in one event, not one each.
- **A call that has to be faster still** is a [native API](/embedding/native-api/): JS objects of your own, built in the engine.

Next: [Errors, tasks and threads](/embedding/native-modules/errors-and-tasks/).
