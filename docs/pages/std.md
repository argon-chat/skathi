---
title: Skathi modules
label: skathi:* modules
description: "The modules every Skathi host provides under the skathi: prefix — ending and restarting the app, its version, the Skathi it runs on, the operating system, the network, fixed-width numbers, TOML, binary data and encodings."
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

| Module                               | What it gives                                                             |
| ------------------------------------ | ------------------------------------------------------------------------- |
| [`skathi:app`](#skathiapp)           | `app.quit()`, `app.restart()`, `app.getVersion()`, and the `app*` objects |
| [`skathi:runtime`](#skathiruntime)   | The Skathi version, the host's features, AOT and single-file              |
| [`skathi:os`](#skathios)             | Platform, architecture, OS release, host name, home and temp              |
| [`skathi:network`](#skathinetwork)   | Online, internet reachability, connection type, metered and roaming       |
| [`skathi:numeric`](#skathinumeric)   | Fixed-width integers and floats: `i8`…`u64`, `f16`, `f32`                 |
| [`skathi:toml`](#skathitoml)         | `TOML.parse()` and `TOML.stringify()`                                     |
| [`skathi:binary`](#skathibinary)     | Byte and bit readers and writers, byte order, LEB128, CRC-32 and CRC-64   |
| [`skathi:encoding`](#skathiencoding) | Hex, Base32, Base64, legacy text encodings and MIME types                 |

A name Skathi has no module for is a build error that lists the ones there are. The modules are
part of the host, so they work the same under `sk dev` and in a built app. A module the embedding
program registers is a different thing: that is [`native:<name>`](/embedding/native-modules/).

## skathi:app

```ts
import { app } from "skathi:app";

interface App {
  quit(options?: { exitCode?: number }): void;
  restart(options?: { args?: string[] }): void;
  getVersion(): string;
}
```

The module also exports every [App API](/app-api/) object (`appTray`, `appShell`, `appWindow` and
the rest). This is where to import them from; importing them from `@skathi-native/runtime` still
works but is deprecated since 0.7.

### app.quit

Ends the app the way [`appWindow.close()`](/window-api/) does: `pagehide` fires once,
`close-requested` and `beforeunload` are not asked, every window hides at once and the teardown
runs behind them, under the same deadline. See [Closing, hiding and quitting](/guides/closing-and-background/).

`exitCode` is the process's exit code, an integer from 0 to 255; the default is 0. Anything else
throws a `TypeError` and the app keeps running. Calling `quit()` again while the app is closing
does nothing.

```ts
import { app } from "skathi:app";

if (!(await migrate())) app.quit({ exitCode: 3 });
```

### app.restart

Ends the app as `quit()` does and starts it again, with the arguments this process was started
with or with `args`. The new process is started before the teardown begins. A single-instance app
restarts correctly: the new process waits for this one to end.

```ts
app.restart();
app.restart({ args: ["--safe-mode"] });
```

`args` must be an array of strings, or the call throws a `TypeError`.

### app.getVersion

The app's version: `"version"` in `sk.config.json`, else the one in `package.json`, else
`"0.0.0"`. It is the string `sk build` stamps into the executable's version resource and the macOS
bundle. The version of Skathi itself is `runtime.version`.

### Platforms

| Method         | Windows      | macOS                        | Linux                                           | Embedded                                                              |
| -------------- | ------------ | ---------------------------- | ----------------------------------------------- | --------------------------------------------------------------------- |
| `quit()`       | Yes          | Yes                          | Yes                                             | Tells the embedder the app closed; `Exit::exit_code` carries the code |
| `restart()`    | The same exe | The executable in the bundle | The AppImage (`$APPIMAGE`), else the executable | Throws `NotSupportedError`: the process is the embedder's             |
| `getVersion()` | Yes          | Yes                          | Yes                                             | The version in the embedder's payload                                 |

## skathi:runtime

```ts
import { Runtime, runtime } from "skathi:runtime";

interface Runtime {
  readonly version: string;
  readonly features: ReadonlySet<string>;
}
declare const Runtime: {
  isAot(): boolean;
  isSingleFile(): boolean;
};
```

- **`runtime.version`** is the version of Skathi the host was built from, for example `"0.7.0"`.
- **`runtime.features`** names the optional parts this host has: `audio`, `spatial-audio`,
  `storages`, `clipboard`, `indexeddb`, `webgpu`, `lottie`, `video`, `hotkeys`, `webview`, `battery`,
  `devtools`, `native-aot`, `no-jit`, `no-wasm`. An embedder chooses them
  ([Features](/embedding/features/)); the hosts `sk build` uses have the default set. Use it for
  diagnostics and bug reports. To decide whether to call an API, test the API itself
  (`typeof localStorage`, `appHotkeys.available`), as on the web.
- **`Runtime.isAot()`** is `true` while the app's code runs from its [native image](/guides/aot/).
  It is `false` under `sk dev`, in a build without `--aot`, with `SK_APP_ENGINE=jit`, and after a
  reload, which runs the bundle.
- **`Runtime.isSingleFile()`** is `true` when the app needs no file beside its executable: the
  payload is inside it and there are no asset packs, no `icu.skpk` and no native image file to load.
  A macOS `.app` bundle counts as one file. It is `false` under `sk dev` and for a payload read
  from a file.

```ts
import { app } from "skathi:app";
import { os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";

const report = {
  app: app.getVersion(),
  skathi: runtime.version,
  features: [...runtime.features],
  aot: Runtime.isAot(),
  os: `${os.platform} ${os.release} ${os.arch}`,
};
```

## skathi:os

```ts
import { os } from "skathi:os";

interface OS {
  readonly platform: "win32" | "darwin" | "linux";
  readonly arch: "x64" | "arm64";
  readonly machineArch: "x64" | "arm64";
  readonly release: string;
  readonly distro: { name: string; version: string; id: string } | null;
  readonly hostname: string;
  readonly homedir: string;
  readonly tmpdir: string;
}
```

The values are synchronous and use Node's names, so code and packages written against
`process.platform` and `os.arch()` read the same strings. `navigator.userAgentData` stays for code
written for a browser; its `getHighEntropyValues()` is asynchronous and reports Chromium's
`platformVersion`, not the OS release.

| Property      | Windows                             | macOS                         | Linux                                                                       |
| ------------- | ----------------------------------- | ----------------------------- | --------------------------------------------------------------------------- |
| `platform`    | `"win32"`                           | `"darwin"`                    | `"linux"`                                                                   |
| `arch`        | What the host was built for         | The same                      | The same                                                                    |
| `machineArch` | The CPU, also for an x64 app on Arm | The CPU, also under Rosetta   | The kernel's machine                                                        |
| `release`     | `"10.0.26100"`: major, minor, build | `"15.1"`: the product version | The kernel release, `"6.8.0-45-generic"`                                    |
| `distro`      | `null`                              | `null`                        | `NAME`, `VERSION_ID` and `ID` of `/etc/os-release`; `null` without the file |
| `hostname`    | The DNS host name                   | `gethostname`                 | `gethostname`                                                               |
| `homedir`     | The user's profile folder           | `$HOME`                       | `$HOME`                                                                     |
| `tmpdir`      | The user's temp folder              | `$TMPDIR`                     | `$TMPDIR`, else `/tmp`                                                      |

`arch` and `machineArch` differ when the app runs translated: an x64 build on Windows on Arm or
under Rosetta reads `arch: "x64"`, `machineArch: "arm64"`. `tmpdir` has no trailing separator. The
app's own folders are in [`appStorage`](/app-api/app-storage/).

## skathi:network

```ts
import { network } from "skathi:network";

interface Network extends EventTarget {
  readonly online: boolean;
  readonly internet: "full" | "limited" | "portal" | "none" | "unknown";
  readonly type: "ethernet" | "wifi" | "cellular" | "other" | "none" | "unknown";
  readonly metered: boolean | null;
  readonly roaming: boolean | null;
  readonly overDataLimit: boolean | null;
}
```

- **`online`** is `navigator.onLine`: the OS believes there is a network. A LAN without internet
  and a captive portal are both online.
- **`internet`** is the operating system's own connectivity check. Skathi sends no probe of its own;
  an app that needs its server checks its server. `"limited"` is a network without internet,
  `"portal"` a captive portal.
- **`type`** is the physical connection traffic takes. Behind a VPN it is the adapter under the
  tunnel, and `"other"` only when there is nothing but virtual adapters.
- **`metered`**, **`roaming`** and **`overDataLimit`** are `null` when the OS cannot tell.

The object fires `change` when any property changes, and `online` and `offline` together with
`window`'s events of the same name, each followed by a `change`.

```ts
import { network } from "skathi:network";

function syncPolicy() {
  return network.metered || network.roaming ? "wifi-only" : "always";
}
network.addEventListener("change", () => applyPolicy(syncPolicy()));
```

| Property        | Windows                                      | macOS                 | Linux               |
| --------------- | -------------------------------------------- | --------------------- | ------------------- |
| `online`        | Yes                                          | Always `true` for now | Yes                 |
| `internet`      | Yes, from the system's connectivity check    | `"unknown"` for now   | `"unknown"` for now |
| `type`          | The connected adapter with a default gateway | `"unknown"` for now   | `"unknown"` for now |
| `metered`       | Yes: a fixed or variable-cost connection     | `null` for now        | `null` for now      |
| `roaming`       | Yes                                          | `null`                | `null`              |
| `overDataLimit` | Yes                                          | `null` for now        | `null`              |

Windows needs Windows 10 2004 or later; before it every property reads as unknown.

## skathi:numeric

Fixed-width numbers without a new primitive: the types are branded `number`s (`bigint` for the
64-bit ones), so a value costs nothing to hold, and the functions do the arithmetic of that width.

```ts
import { i32, u8, u32, u64, f32 } from "skathi:numeric";

const a = u8(250);
u8.add(a, u8(10)); // 4: wraps
u8.checkedAdd(a, u8(10)); // null
u8.saturatingAdd(a, u8(10)); // 255
u8.overflowingAdd(a, u8(10)); // [4, true]

u32.mul(u32(0xffffffff), u32(2)); // 4294967294, exact
i32.div(i32(-7), i32(2)); // -3: truncates
u64.rotl(u64(1n << 63n), 1); // 1n
f32.add(f32(0.1), f32(0.2)); // rounded to f32 after the step
```

| Type         | Stored as | Typed array                       |
| ------------ | --------- | --------------------------------- |
| `i8`, `u8`   | `number`  | `Int8Array`, `Uint8Array`         |
| `i16`, `u16` | `number`  | `Int16Array`, `Uint16Array`       |
| `i32`, `u32` | `number`  | `Int32Array`, `Uint32Array`       |
| `i64`, `u64` | `bigint`  | `BigInt64Array`, `BigUint64Array` |
| `f16`        | `number`  | `Float16Array`                    |
| `f32`        | `number`  | `Float32Array`                    |

Each integer type is a function and a namespace:

- **Conversion.** `u8(x)` wraps as a cast does in Rust or C; `u8.checked(x)` throws a `RangeError`
  for a value that is not a `u8`; `u8.saturating(x)` clamps. `NaN` converts to 0.
- **Constants.** `MIN`, `MAX`, `BITS`.
- **Arithmetic.** `add`, `sub`, `mul`, `div`, `rem`, `neg` wrap. `div` truncates toward zero, and a
  zero divisor throws a `RangeError`. `checkedAdd`/`Sub`/`Mul` return `null` on overflow,
  `saturatingAdd`/`Sub`/`Mul` clamp, `overflowingAdd`/`Sub`/`Mul` return `[value, overflowed]`.
- **Bits.** `and`, `or`, `xor`, `not`, `shl`, `shr` (arithmetic for signed types, logical for
  unsigned), `rotl`, `rotr`, `clz`, `ctz`, `popcount`. A shift count is taken modulo `BITS`.
- **Comparison.** Values compare with the ordinary operators; `lt(a, b)` is there for symmetry.

`f16` and `f32` round on conversion and after `add`, `sub`, `mul`, `div` and `fma`.

The brand is a compile-time check only: passing a plain `number` where a `u8` is expected is a type
error, and the way in is the type's function. The module is plain JavaScript and works the same on
every platform.

## skathi:toml

TOML 1.1 in the shape of `JSON`.

```ts
import { TOML } from "skathi:toml";

const config = TOML.parse(text);
const text = TOML.stringify(config, { pretty: true });
```

**`TOML.parse(text)`** returns the document as an object, keys in document order.

| TOML                                    | JavaScript                                         |
| --------------------------------------- | -------------------------------------------------- |
| String, boolean, float                  | `string`, `boolean`, `number`                      |
| Integer                                 | `number`; `bigint` beyond ±2^53                    |
| Offset date-time                        | `Date`                                             |
| Local date-time, local date, local time | `Temporal.PlainDateTime`, `PlainDate`, `PlainTime` |
| Array, table, array of tables           | `Array`, object, `Array` of objects                |

Invalid TOML throws a `SyntaxError` with the position:
`TOML.parse: line 4, column 9: expected …`.

**`TOML.stringify(value, options?)`** writes an object as a document. `pretty: true` puts each
array element on its own line.

- `undefined`, functions and symbols are left out of a table, as `JSON.stringify` leaves them out
  of an object. In an array they throw a `TypeError`.
- `null` throws a `TypeError` that names the key (`window.title is null`): TOML has no null.
- A `bigint` is written as an integer, and throws a `RangeError` outside 64 bits.
- A `Date` is written as an offset date-time in UTC; the `Temporal` plain types as the local forms.
- A whole number is written as an integer, any other `number` as a float.

## skathi:binary

Cursors over bytes, bit streams, byte order, LEB128 and CRCs.

```ts
import { BinaryReader, BinaryWriter, CRC32 } from "skathi:binary";

const HEADER = { magic: "u32", version: "u16", count: "u16" } as const;

const writer = new BinaryWriter();
writer.writeStruct(HEADER, { magic: 0x534b3031, version: 1, count: 3 });
writer.writeAll([1.5, 2.5, 3.5], "f32");
const bytes = writer.finish();

const reader = new BinaryReader(bytes);
const header = reader.readStruct(HEADER);
const values = reader.readArray(Float32Array, header.count);
const checksum = CRC32.compute(bytes);
```

Each method does a batch of work in one call, because a call into the host costs more than a
scalar read does. For one scalar at a time, read it from `reader.view`, a standard `DataView`, at
`reader.position`, and move `position` yourself.

### BinaryReader

`new BinaryReader(source, { endian })` reads `source` (an `ArrayBuffer`, a typed array or a
`DataView`) in place, without a copy. `endian` is `"little"` by default.

| Member                   | What it does                                                              |
| ------------------------ | ------------------------------------------------------------------------- |
| `position`               | The byte the next read starts at; assignable                              |
| `remaining`              | Bytes from `position` to the end                                          |
| `view`                   | A `DataView` of the whole source                                          |
| `readArray(Type, count)` | `count` elements as a new `Type` (`Float32Array`, `Uint16Array`, …)       |
| `readStruct(layout)`     | An object with the layout's fields, read one after another                |
| `bytes(n)`               | The next `n` bytes as a `Uint8Array` over the source, not a copy          |
| `string(n, encoding?)`   | The next `n` bytes as text; any `TextDecoder` label, `"utf-8"` by default |
| `varUints(count)`        | `count` unsigned LEB128 values of up to 32 bits, as a `Uint32Array`       |

A read past the end throws a `RangeError` and leaves `position` where it was.

A layout is an object of field names and types: `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `u64`,
`i64`, `f32`, `f64`. Fields follow each other with no padding, in the order the object lists them.
`u64` and `i64` are `bigint`, the rest are `number`. Declare a layout `as const` and the result is
typed by it.

### BinaryWriter

`new BinaryWriter({ endian, initialCapacity })` grows as it is written to.

| Member                       | What it does                                                     |
| ---------------------------- | ---------------------------------------------------------------- |
| `writeAll(values, type)`     | Every value of an array or typed array as `type`; integers wrap  |
| `writeStruct(layout, value)` | The layout's fields of `value`                                   |
| `bytes(source)`              | The bytes of a buffer or a view                                  |
| `length`                     | Bytes written so far                                             |
| `view`                       | A `DataView` of what is written, to patch a length or a checksum |
| `finish()`                   | The bytes as a `Uint8Array`, handed over without a copy          |

The write methods return the writer. After `finish()` the writer is empty and can be used again.
`view` covers what was written when you took it: take it again after writing more.

### BitReader and BitWriter

```ts
import { BitReader, BitWriter } from "skathi:binary";

const packed = new BitWriter().write(5, 3).writeMany([1, 2, 3], 2).finish();
const bits = new BitReader(packed);
bits.read(3); // 5
bits.readMany(2, 3); // Uint32Array [1, 2, 3]
```

A value is 1 to 32 bits. The order is `"msb"` by default: a byte's high bit comes first, as in
most network formats and in JPEG. `"lsb"` takes the low bit first, as DEFLATE does. Pass the same
order to both. `align()` skips to the next byte boundary; `finish()` pads the last byte with zero
bits.

### Endian, VarInt, CRC32, CRC64

| Call                                       | What it does                                                          |
| ------------------------------------------ | --------------------------------------------------------------------- |
| `Endian.native`                            | `"little"` or `"big"`: this machine's byte order                      |
| `Endian.swap16(a)`, `swap32`, `swap64`     | Reverses the bytes of every element of a typed array, in place        |
| `VarInt.encodeAll(values, zigzag?)`        | LEB128 bytes of integers within ±2^53; negative ones need `zigzag`    |
| `VarInt.decodeAll(bytes, zigzag?)`         | Every value in `bytes`, as a `Float64Array`                           |
| `VarInt.size(value, zigzag?)`              | The bytes one value takes                                             |
| `CRC32.compute(bytes, seed?)`              | CRC-32 as zlib and PNG compute it; `seed` is the CRC of earlier bytes |
| `CRC64.compute(bytes, variant?)`           | A `bigint`; `"ecma"` (default) is CRC-64/XZ, `"nvme"` is CRC-64/NVME  |
| `CRC32.create()`, `CRC64.create(variant?)` | An object with `update(bytes)` and `digest()`, for data in pieces     |

The module is the same on every platform.

## skathi:encoding

Bytes as text and back, and MIME types.

```ts
import { Base32, Encoding, Hex, Mime } from "skathi:encoding";

Hex.encode(bytes); // "cafe01"
Base32.encode(bytes, { alphabet: "crockford" });
Encoding.decode(bytes, "windows-1251");
Mime.sniff(bytes); // "image/png"
```

| Object     | Calls                                                                                  |
| ---------- | -------------------------------------------------------------------------------------- |
| `Hex`      | `encode(bytes)`, `decode(text)`, `dump(bytes, { width, offset })`, `format(n, width?)` |
| `Base16`   | `encode` and `decode` of `Hex`, under the RFC 4648 name                                |
| `Base32`   | `encode(bytes, { alphabet, padding })`, `decode(text, { alphabet })`                   |
| `Base64`   | `encode(bytes, { alphabet, omitPadding })`, `decode(text, { alphabet })`               |
| `Mime`     | `fromName(name)`, `extensions(type)`, `sniff(bytes)`, `matches(type, accept)`          |
| `Encoding` | `decode(bytes, label)`, `encode(text, label)`, `isSupported(label)`, `detect(bytes)`   |

Text that is not valid for its codec throws a `SyntaxError` that says which character.

- **Hex** writes lowercase and reads either case. `Hex.dump` prints lines of offset, hex pairs and
  ASCII, the way `xxd` does. `Hex.format(255, 4)` is `"0x00ff"`, and it takes a `bigint`.
- **Base32** alphabets are `"rfc4648"` (default), `"hex"` (base32hex) and `"crockford"`. Decoding
  takes either case and optional padding. Crockford's also reads `O` as `0`, `I` and `L` as `1`,
  and skips hyphens.
- **Base64** alphabets are `"base64"` (default) and `"base64url"`. `decode` without `alphabet`
  takes the characters of both; with it, the other alphabet's characters throw. Whitespace is
  skipped and padding is optional.
- **Mime.fromName** answers from the table behind `File.type` and local `fetch()`, so the three
  agree; an unknown extension is `""`. `sniff` reads the signature in a file's first bytes and
  returns `null` when it knows none. `matches("image/png", "image/*, .pdf")` tests a type against
  an accept list of types, `type/*` patterns and extensions.
- **Encoding** takes any `TextDecoder` label. `TextEncoder` only writes UTF-8; `Encoding.encode`
  writes the legacy encodings as well (`windows-1251`, `shift_jis`, `gbk`, …). A character the
  encoding has no bytes for becomes `?`. An unknown label throws a `RangeError`. `detect` goes by
  the byte order mark only and returns `null` without one.

`Mime.extensions` on Windows also asks the registry for a type the table does not know. The rest is
the same on every platform.
