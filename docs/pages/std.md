---
title: Skathi modules
label: skathi:* modules
description: "The modules every Skathi host provides under the skathi: prefix, each export with its own page: ending and restarting the app, its version, the Skathi it runs on, the operating system, the network, fixed-width numbers, TOML, binary data, encodings, C libraries, native memory, strings, time, units and collections."
section: Reference
order: 6
since: "0.7"
---

# Skathi modules

`skathi:*` is Skathi's own library: modules the host provides, imported by name the way Node has
`node:` modules. There is nothing to install. Every builder (`sk dev`, `sk build`, the Vite plugin)
resolves the prefix, and the types come with `@skathi-native/runtime`.

```ts
import { app } from "skathi:app";
import { os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";
```

| Module                                     | What it gives                                                                         |
| ------------------------------------------ | ------------------------------------------------------------------------------------- |
| [`skathi:app`](#skathiapp)                 | `app.quit()`, `app.restart()`, `app.getVersion()`, and the `app*` objects             |
| [`skathi:runtime`](#skathiruntime)         | The Skathi version, the host's features, AOT and single-file                          |
| [`skathi:os`](#skathios)                   | Platform, architecture, OS release, host name, home and temp                          |
| [`skathi:network`](#skathinetwork)         | Online, internet reachability, connection type, metered and roaming                   |
| [`skathi:numeric`](#skathinumeric)         | Fixed-width integers and floats: `i8`…`u64`, `f16`, `f32`                             |
| [`skathi:toml`](#skathitoml)               | `TOML.parse()` and `TOML.stringify()`                                                 |
| [`skathi:binary`](#skathibinary)           | Byte and bit readers and writers, byte order, LEB128, CRC-32 and CRC-64               |
| [`skathi:encoding`](#skathiencoding)       | Hex, Base32, Base64, legacy text encodings and MIME types                             |
| [`skathi:ffi`](#skathiffi)                 | `dlopen()`: C functions of the libraries the config allows, `struct()`, callbacks     |
| [`skathi:memory`](#skathimemory)           | `NativeBuffer`, `Arena`, `MemoryView`: native memory without a copy, freed on demand  |
| [`skathi:gpu`](#skathigpu)                 | `wgsl` types, `struct()`, `GpuTimer`, `readTexture()`, `readBuffer()` for WebGPU      |
| [`skathi:text`](#skathitext)               | `StringBuilder`, `StringReader`, `StringWriter`, `TextDiff`, `TextSearch`, `Glob`     |
| [`skathi:time`](#skathitime)               | `TimeSpan`, `Stopwatch`, `Deadline`, `Backoff`, `RetryPolicy`, `RateLimiter`, `Timer` |
| [`skathi:units`](#skathiunits)             | `DataSize`, `Frequency`, `Angle`                                                      |
| [`skathi:collections`](#skathicollections) | `BitSet`, `BloomFilter`, `RingBuffer`, `DisjointSet`, `Trie`, `IntervalTree`          |

A name Skathi has no module for is a build error that lists the ones there are. The modules are
part of the host, so they work the same under `sk dev` and in a built app. A module the embedding
program registers is a different thing: that is [`native:<name>`](/embedding/native-modules/).

Every export has its own page below: the signatures, what throws, examples and, where the
platforms differ, what each one does.

## Conventions

- **Errors.** A wrong argument throws a `TypeError`, a value out of range a `RangeError`, and text
  that does not parse a `SyntaxError`. Each page lists its own.
- **Classes are not globals.** `BitSet`, `TimeSpan` and the rest exist only as imports, so they
  never clash with a name in your code or in a library.
- **Bytes.** Where bytes are taken, any `ArrayBuffer`, typed array or `DataView` works, and in
  `skathi:binary` and `skathi:ffi` a [`NativeBuffer`](/std/native-buffer/) too.
- **Time.** Where a time is taken, it is milliseconds or a [`TimeSpan`](/std/time-span/).
- **The `app*` objects.** `skathi:app` also exports `appTray`, `appShell`, `appWindow` and the rest.
  They have their own references: [App APIs](/app-api/) and the [Window API](/window-api/).
