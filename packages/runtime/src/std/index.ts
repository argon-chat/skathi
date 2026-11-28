// The `skathi:<name>` modules by name, for the dev loader; a production bundle imports their files.

import * as app from "./app";
import * as binary from "./binary";
import * as collections from "./collections";
import * as encoding from "./encoding";
import * as ffi from "./ffi";
import * as gamepad from "./gamepad";
import * as gpu from "./gpu";
import * as image from "./image";
import * as memory from "./memory";
import * as network from "./network";
import * as numeric from "./numeric";
import * as os from "./os";
import * as process from "./process";
import * as runtime from "./runtime";
import * as text from "./text";
import * as time from "./time";
import * as toml from "./toml";
import * as units from "./units";

export const stdModules: Readonly<Record<string, Record<string, unknown>>> = {
  /** `app.quit()`, `app.restart()`, `app.getVersion()`, and the `app*` objects */
  app,
  /** Byte and bit readers and writers, byte order, LEB128, CRC-32 and CRC-64 */
  binary,
  /** `BitSet`, `BloomFilter`, `RingBuffer`, `DisjointSet`, `Trie`, `IntervalTree` */
  collections,
  /** Hex, Base32, Base64, legacy text encodings and MIME types */
  encoding,
  /** `dlopen()` for C functions of the libraries the config allows, `struct()` and callbacks */
  ffi,
  /** `gamepads` for controller snapshots, button and axis events, and rumble */
  gamepad,
  /** `wgsl` types, `struct()`, `GpuTimer`, `readTexture()`, `readBuffer()` for WebGPU */
  gpu,
  /** `ImageDecoder`, `ImageEncoder`, `Image` for metadata, frame decoding and encoding off-thread */
  image,
  /** `NativeBuffer`, `Arena`, `MemoryView` for native memory without a copy, freed on demand */
  memory,
  /** Online, internet reachability, connection type, metered and roaming */
  network,
  /** Fixed-width integers and floats from `i8` to `u64`, plus `f16` and `f32` */
  numeric,
  /** Platform, architecture, OS release, host name, home and temp */
  os,
  /** `Process.spawn()` and `Process.exec()` for the programs the config allows, without a shell */
  process,
  /** The Skathi version, the host's features, AOT and single-file */
  runtime,
  /** `StringBuilder`, `StringReader`, `StringWriter`, `TextDiff`, `TextSearch`, `FuzzyMatcher`, `Glob` */
  text,
  /** `TimeSpan`, `Stopwatch`, `Deadline`, `Backoff`, `RetryPolicy`, `RateLimiter`, `TaskQueue`, `Timer` */
  time,
  /** `TOML.parse()`, `TOML.stringify()`, and `import config from "./config.toml"` */
  toml,
  /** `DataSize`, `Frequency`, `Angle` */
  units,
};
