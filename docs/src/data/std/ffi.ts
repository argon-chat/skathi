import { importLine, section } from "./shared";

const CRASH =
  "The call runs inside the app's process. A fault in the library (an access violation, a wrong signature) ends the app at once: no `pagehide`, no crash dialog, unsaved data is lost. The host prints `[crash] ffi: <library>!<symbol>` to stderr first.";
const PERMISSION =
  'A library loads only if sk.config.json lists it under [`"permissions.ffi"`](/sk-config/#permissions). The rule is the same under `sk dev`. An app embedded in another program loads what that program allows with `Builder::allow_ffi()` instead, and the `ffi` feature of the `skathi` crate must be on.';

export const ffi = section("skathi:ffi", [
  {
    slug: "dlopen",
    name: "dlopen()",
    kind: "function",
    summary: "Loads a library that sk.config.json allows and returns its C functions as JS functions.",
    syntax: `${importLine("skathi:ffi", "dlopen")}

dlopen<S extends Record<string, Signature>>(name: string, symbols: S): NativeLibrary<S>

interface Signature {
  args?: readonly FFIType[]   // default: none
  returns?: FFIType           // default: "void"
  errno?: boolean             // the call answers { value, errno }
  optional?: boolean          // a missing symbol is null instead of an error
  variadic?: number           // named parameters of a variadic function
  nonblocking?: boolean       // the call runs on a worker thread and answers with a promise
}`,
    params: [
      {
        name: "name",
        type: "string",
        description:
          'A name from `"permissions.ffi"` in sk.config.json. It is not a path: the config says where the library is.',
      },
      {
        name: "symbols",
        type: "Record<string, Signature>",
        description:
          "The functions to look up, by their exported names, each with its argument and return [types](/std/ffi-types/). Write it as a literal so that TypeScript types every function from it.",
      },
    ],
    returns: "A [`NativeLibrary`](/std/native-library/) whose `symbols` has one function for each entry.",
    notes: [
      PERMISSION,
      '`"$APP/lib/sqlite3"` is a file in the app\'s folder; `sk build` copies it beside the executable at the same path. `"$SYSTEM/user32"` is a library the system\'s loader finds in its own folders. Without an extension the host adds `.dll`, `.dylib` or `.so`, and outside Windows it also tries `lib<name>`.',
      "`sk check` compares the names with the config before the app runs: a `dlopen()` of a name that is not listed is the error SK6002, an import of `skathi:ffi` in an app that lists nothing is SK6001, a name that is not a literal is the warning SK6003, and a package in node_modules that imports `skathi:ffi` is the warning SK6005.",
      "Use it for a library you already have as a binary. For code you write yourself, a [native module](/embedding/native-modules/) is the better path: it is typed from Rust, and a panic becomes a JS error instead of ending the app.",
      "A call is synchronous and runs on the JS thread: the window does not paint until it returns. For a function that takes long (a query, a decode, anything that waits), set `nonblocking: true`.",
      "`nonblocking: true` runs the call on one of four worker threads and the function returns a promise. A wrong argument rejects the promise instead of throwing. A string is copied for the call. A `buffer` is copied too: the library works on the copy, and the bytes are written back into your buffer when the call returns, before the promise settles. Until then your buffer is unchanged, and what you write into it in the meantime is overwritten. A `pointer` is passed as it is: the memory must stay valid until the promise settles.",
      "Calls of one library may run at the same time on different threads, in any order. If the library is not safe for that, wait for one promise before you make the next call.",
      "A [`NativeCallback`](/std/native-callback/) that a nonblocking call invokes is called from another thread: it runs on the JS thread a frame later, and only a `void` callback works.",
      "`errno: true` makes the call answer `{ value, errno }`. `errno` is the C `errno`, or `GetLastError()` on Windows, read on the same thread right after the call. It is whatever the last failing call left there: read it only when `value` says the call failed.",
      "`variadic` is for functions such as `printf`: list every argument of this call site in `args` and give the number of named parameters. Declaring a variadic function without it passes the arguments the wrong way on Windows and on Apple silicon.",
      "A struct cannot be passed or returned by value. Lay it out in a buffer, with [`BinaryWriter`](/std/binary-writer/) for example, and pass the buffer. A [struct layout](/std/struct-layout/) has no padding, so write the padding the C compiler adds as fields of its own.",
      "To hand the library a function of yours, make a [`NativeCallback`](/std/native-callback/) and pass its `pointer`.",
      CRASH,
    ],
    edgeCases: [
      '`NotAllowedError`: `name` is not in `"permissions.ffi"`, or the list is empty. The message has the entry to add.',
      "`NotFoundError`: the library, or a library it needs, could not be loaded, or it has no such symbol and the entry is not `optional`. The message has the system's own words.",
      "`NotSupportedError`: the host was built without the `ffi` feature. Hosts that `sk build` uses have it; an embedder turns it on.",
      '`TypeError`: a type that is not in the list, `"void"` as an argument, `"buffer"` as a return type, a struct.',
      "Calling a function with the wrong number of arguments, or with a value that is not of the declared type, throws a `TypeError` that names the function and the argument. An integer that does not fit throws a `RangeError`. Nothing is converted silently.",
      "A nonblocking call rejects with a `TypeError` for a `buffer` that is resizable or growable, since it could shrink before the bytes are written back. If its length changed anyway (the buffer was transferred), nothing is written back.",
      "`QuotaExceededError`: 256 nonblocking calls are already waiting or running.",
    ],
    examples: [
      {
        title: "Call a function of a system library",
        lang: "ts",
        note: 'sk.config.json: "permissions": { "ffi": [{ "name": "user32", "path": "$SYSTEM/user32" }] }',
        code: `import { dlopen } from "skathi:ffi";

const user32 = dlopen("user32", {
  MessageBoxW: { args: ["pointer", "wstring", "wstring", "u32"], returns: "i32" },
  GetSystemMetrics: { args: ["i32"], returns: "i32" },
});

const width = user32.symbols.GetSystemMetrics(0);
user32.symbols.MessageBoxW(null, \`The screen is \${width} px wide\`, "Skathi", 0);`,
      },
      {
        title: "Ship a library with the app and read an error code",
        lang: "ts",
        note: 'sk.config.json: "permissions": { "ffi": [{ "name": "sqlite3", "path": "$APP/lib/sqlite3" }] }',
        code: `import { dlopen, NativePointer } from "skathi:ffi";

const sqlite = dlopen("sqlite3", {
  sqlite3_libversion: { returns: "cstring" },
  sqlite3_open: { args: ["cstring", "buffer"], returns: "i32" },
  sqlite3_close: { args: ["pointer"], returns: "i32" },
  sqlite3_errstr: { args: ["i32"], returns: "cstring" },
});

console.log(sqlite.symbols.sqlite3_libversion()); // "3.46.0"

const out = new BigUint64Array(1);
const code = sqlite.symbols.sqlite3_open("notes.db", out);
if (code !== 0) throw new Error(sqlite.symbols.sqlite3_errstr(code) ?? "sqlite3_open failed");`,
      },
      {
        title: "A variadic function and an optional one",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";
import { Encoding } from "skathi:encoding";

const crt = dlopen("crt", {
  snprintf: {
    args: ["buffer", "usize", "cstring", "i32", "f64"],
    returns: "i32",
    variadic: 3,
  },
  explicit_bzero: { args: ["buffer", "usize"], optional: true },
});

const line = new Uint8Array(64);
const length = crt.symbols.snprintf(line, line.length, "%d items, %.1f%%", 12, 99.5);
Encoding.decode(line.subarray(0, length), "utf-8"); // "12 items, 99.5%"

crt.symbols.explicit_bzero?.(line, line.length);`,
      },
      {
        title: "A long call on a worker thread",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";

const codec = dlopen("codec", {
  codec_decode: {
    args: ["buffer", "usize", "buffer", "usize"],
    returns: "i32",
    nonblocking: true,
  },
});

const pixels = new Uint8Array(width * height * 4);
// The window keeps painting; pixels is filled when the promise settles.
const code = await codec.symbols.codec_decode(file, file.length, pixels, pixels.length);
if (code !== 0) throw new Error(\`codec_decode: \${code}\`);`,
      },
    ],
    related: ["ffi-types", "native-library", "native-pointer", "native-callback"],
    aliases: [
      "FFI",
      "foreign function interface",
      "bun:ffi",
      "DLL",
      "dylib",
      "shared library",
      "LoadLibrary",
      "Signature",
      "nonblocking",
      "async call",
      "worker thread",
    ],
  },
  {
    slug: "ffi-types",
    name: "FFIType",
    kind: "type",
    summary: "The C types a signature can name, and the JS value each one takes and gives back.",
    syntax: `type FFIType =
  | "void" | "bool"
  | "i8" | "u8" | "i16" | "u16" | "i32" | "u32"
  | "i64" | "u64" | "isize" | "usize"
  | "f32" | "f64"
  | "pointer" | "buffer" | "cstring" | "wstring"`,
    values: [
      { value: '"void"', status: "supported", note: "A return type only. The call returns `undefined`." },
      { value: '"bool"', status: "supported", note: "`true` or `false`, nothing else. C `bool` or `_Bool`." },
      {
        value: '"i8" "u8" "i16" "u16" "i32" "u32"',
        status: "supported",
        note: "Takes an integer `number` or a `bigint` that fits; returns a `number`. `int` is `i32`, and `long` is `i32` on Windows and `i64` elsewhere.",
      },
      {
        value: '"i64" "u64" "isize" "usize"',
        status: "supported",
        note: "Takes a `bigint`, or a `number` up to 2^53 − 1; always returns a `bigint`. `size_t` is `usize`.",
      },
      {
        value: '"f32" "f64"',
        status: "supported",
        note: "A `number`. `f32` is rounded to single precision on the way in.",
      },
      {
        value: '"pointer"',
        status: "supported",
        note: "Takes a [`NativePointer`](/std/native-pointer/) or `null`; returns a `NativePointer`, which is null for `NULL`. A number, a string or a buffer is a `TypeError`.",
      },
      {
        value: '"buffer"',
        status: "supported",
        note: "An argument only. Takes an `ArrayBuffer`, a typed array or a `DataView` and passes the address of its bytes, without a copy; `null` passes `NULL`. The address is good for this call only.",
      },
      {
        value: '"cstring"',
        status: "supported",
        note: "Takes a JS string and passes it as NUL-terminated UTF-8, encoded for this call; `null` passes `NULL`. As a return type it copies the text into a JS string (`null` for `NULL`) and does not free the C buffer.",
      },
      {
        value: '"wstring"',
        status: "supported",
        note: "The same as NUL-terminated UTF-16: the `W` functions of Windows. It is 16-bit on every platform, so it is not `wchar_t*` on macOS and Linux.",
      },
    ],
    notes: [
      "One table converts values for every call, so a `7` behaves the same whether the engine holds it as an integer or as a double.",
      "An integer is cut to its declared width on the way in and extended from it on the way out. A C function that returns `uint8_t` and leaves noise in the rest of the register still gives 0 to 255.",
      "`u32` stays unsigned: 2^31 comes back as 2147483648, not as a negative number.",
      "A `cstring` argument with a NUL character in it is a `TypeError`: the C side would see the text cut short.",
      "A string argument is encoded on every call. For a string passed many times, encode it once, keep it in a typed array and declare the parameter as `buffer`.",
      "A returned `cstring` or `wstring` is read up to its NUL. If the library allocated it and expects you to free it, declare the return as `pointer`, read it with `readCString()` and pass the pointer to the library's free function.",
      "There is no struct type. A function pointer is a `pointer`: pass the `pointer` of a [`NativeCallback`](/std/native-callback/).",
    ],
    examples: [
      {
        title: "64-bit values are bigints, on every path",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";

const crt = dlopen("crt", {
  strlen: { args: ["cstring"], returns: "usize" },
  abs: { args: ["i32"], returns: "i32" },
});

crt.symbols.strlen("héllo"); // 6n: bytes of UTF-8
crt.symbols.abs(-5);         // 5
crt.symbols.abs(-5n);        // 5
crt.symbols.abs(1.5);        // TypeError: argument 1 (i32) must be an integer
crt.symbols.abs(2 ** 31);    // RangeError: 2147483648 does not fit i32`,
      },
      {
        title: "A buffer as an out-parameter",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";

const kernel32 = dlopen("kernel32", {
  GetEnvironmentVariableW: { args: ["wstring", "buffer", "u32"], returns: "u32" },
});

const out = new Uint16Array(260);
const length = kernel32.symbols.GetEnvironmentVariableW("SystemRoot", out, out.length);
String.fromCharCode(...out.subarray(0, length)); // "C:\\\\Windows"`,
      },
    ],
    related: ["dlopen", "native-pointer", "struct-layout"],
    aliases: ["cstring", "wstring", "buffer", "pointer", "usize", "size_t", "wchar_t", "LPCWSTR", "char*"],
  },
  {
    slug: "native-library",
    name: "NativeLibrary",
    kind: "class",
    summary: "A loaded library: its functions, where it was loaded from, and a way to stop calling it.",
    syntax: `${importLine("skathi:ffi", "dlopen", "type NativeLibrary")}

readonly library.name: string
readonly library.path: string
readonly library.closed: boolean
readonly library.symbols: { [name]: NativeFunction | null }
library.close(): void

interface NativeFunction<S> {
  (...args): value            // typed from the signature
  readonly name: string
  readonly signature: S
}`,
    notes: [
      "`dlopen()` makes it; there is nothing to construct.",
      "`symbols` is frozen. A function is `null` only for an `optional` entry whose symbol the library does not have.",
      "`path` is the file the library was loaded from, or its name for a `$SYSTEM` library.",
      "`close()` makes later calls throw `InvalidStateError`. The library stays loaded for as long as the app runs: unloading code that something may still point into is not safe, and a library is loaded once however often it is opened.",
      "Opening the same name twice gives two objects over one loaded library. Use that to declare one symbol with two signatures.",
    ],
    examples: [
      {
        title: "Check what a library has before using it",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";

const codec = dlopen("codec", {
  codec_version: { returns: "u32" },
  codec_encode_fast: { args: ["buffer", "usize", "buffer"], returns: "i32", optional: true },
  codec_encode: { args: ["buffer", "usize", "buffer"], returns: "i32" },
});

const encode = codec.symbols.codec_encode_fast ?? codec.symbols.codec_encode;
console.log(codec.path, encode.name);`,
      },
    ],
    related: ["dlopen", "ffi-types"],
    aliases: ["NativeFunction", "symbols", "NativeLibrary.close", "NativeLibrary.path"],
  },
  {
    slug: "native-pointer",
    name: "NativePointer",
    kind: "class",
    summary: "An address in native memory, as an object and not a number, with reads and writes through it.",
    syntax: `${importLine("skathi:ffi", "NativePointer")}

static readonly NativePointer.NULL: NativePointer
static NativePointer.of(source: ArrayBuffer | ArrayBufferView, offset?: number): NativePointer
static NativePointer.fromAddress(address: bigint | number): NativePointer

readonly pointer.address: bigint
pointer.isNull(): boolean
pointer.offset(bytes: number): NativePointer
pointer.equals(other: NativePointer): boolean
pointer.readCString(maxBytes?: number): string
pointer.readWString(maxUnits?: number): string
pointer.read(length: number): Uint8Array
pointer.write(bytes: ArrayBuffer | ArrayBufferView): void`,
    notes: [
      "A pointer comes from a call that returns `pointer`, or from a buffer with `NativePointer.of()`. It cannot be constructed.",
      "`NativePointer.of(buffer)` is for an address the library keeps after the call returns. The pointer keeps the buffer alive, and so does every pointer made from it with `offset()`. For an address used only during a call, declare the parameter as `buffer` and pass the buffer itself.",
      "A returned `NULL` is a pointer object whose `isNull()` is `true`, not `null`. As an argument, `null` and `NativePointer.NULL` are the same.",
      "`address` is a `bigint`: addresses do not fit a `number` on every system.",
      "`read(length)` copies bytes out, and `write(bytes)` copies bytes in. Parse what `read()` returns with [`BinaryReader`](/std/binary-reader/). There is no view over native memory: a copy cannot outlive the memory it came from.",
      "`readCString()` and `readWString()` stop at the first NUL, or at the limit if one is given.",
      "Reading and writing are not checked: a wrong address or a length past the end is a fault that ends the app.",
      '`NativePointer.fromAddress()` needs `"permissions": { "ffiAddress": true }` in sk.config.json. Without it code can only reach memory a listed library handed out or a buffer it owns.',
    ],
    edgeCases: [
      "`NativePointer.of()` throws a `TypeError` for an empty or detached buffer and a `RangeError` for an offset past its end.",
      "`NativePointer.fromAddress()` throws `NotAllowedError` without the permission.",
      "`readCString()`, `readWString()`, `read()` and `write()` throw a `TypeError` on a null pointer.",
      "`new NativePointer()` throws a `TypeError`.",
    ],
    examples: [
      {
        title: "Read a string the library owns, then free it",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";

const lib = dlopen("report", {
  report_render: { args: ["cstring"], returns: "pointer" },
  report_free: { args: ["pointer"] },
});

const text = lib.symbols.report_render("weekly");
try {
  if (text.isNull()) throw new Error("report_render failed");
  console.log(text.readCString());
} finally {
  lib.symbols.report_free(text);
}`,
      },
      {
        title: "Give a library a buffer it keeps",
        lang: "ts",
        code: `import { dlopen, NativePointer } from "skathi:ffi";

const audio = dlopen("mixer", {
  mixer_set_output: { args: ["pointer", "usize"] },
  mixer_render: { args: ["u32"], returns: "u32" },
});

const samples = new Float32Array(4096);
const output = NativePointer.of(samples);
audio.symbols.mixer_set_output(output, samples.length);

audio.symbols.mixer_render(512);
console.log(samples[0]); // written by the library`,
      },
      {
        title: "Read a struct through a pointer",
        lang: "ts",
        code: `import { BinaryReader } from "skathi:binary";
import { dlopen } from "skathi:ffi";

const lib = dlopen("sensors", { sensors_latest: { returns: "pointer" } });

// struct reading { uint32_t id; float value; uint64_t at; }
const bytes = lib.symbols.sensors_latest().read(16);
const reading = new BinaryReader(bytes).readStruct({ id: "u32", value: "f32", at: "u64" });
reading.at; // a bigint`,
      },
    ],
    related: ["dlopen", "ffi-types", "binary-reader"],
    aliases: [
      "pointer",
      "NULL",
      "address",
      "NativePointer.of",
      "NativePointer.fromAddress",
      "readCString",
      "readWString",
      "ffiAddress",
    ],
  },
  {
    slug: "native-callback",
    name: "NativeCallback",
    kind: "class",
    summary: "A JS function as a C function pointer that a library can call.",
    syntax: `${importLine("skathi:ffi", "NativeCallback")}

new NativeCallback(callback: (...args) => value, signature: { args?: FFIType[]; returns?: FFIType })

static NativeCallback.live(): number

readonly callback.pointer: NativePointer
readonly callback.closed: boolean
callback.close(): void`,
    params: [
      {
        name: "callback",
        type: "function",
        description:
          "Called with the C arguments converted by the same rules as a function's return value: a `pointer` is a `NativePointer`, a `cstring` or `wstring` is a JS string or `null`, a 64-bit integer is a `bigint`.",
      },
      {
        name: "signature",
        type: "{ args?, returns? }",
        description:
          'The C signature the library expects. `args` cannot have `"buffer"`: memory arrives as a `"pointer"`. `returns` is `"void"` by default and cannot be `"buffer"`, `"cstring"` or `"wstring"`.',
      },
    ],
    notes: [
      'Pass `callback.pointer` where the library takes a function pointer, declared as `"pointer"` in the function\'s signature.',
      "A callback holds its JS function until `close()`, even when nothing in JS refers to the `NativeCallback` any more: the library may call for as long as it has the pointer. Close every callback you make, after the library has stopped using it.",
      "There are 128 slots. They are thunks built into the host, so nothing is generated at run time and callbacks work in a build without the JIT. `NativeCallback.live()` is how many are open: a number that only grows is a callback that is never closed.",
      "A call that arrives after `close()` runs nothing and returns 0. A slot is reused as late as possible, so a pointer kept by mistake reaches nothing for a long time, but it must not be relied on.",
      "An exception thrown inside the callback does not unwind through the library. The callback returns 0 to the library, later callbacks of the same call are skipped and return 0, and the exception is thrown from the `symbols` call that led to it, once the library returns.",
      "A value of the wrong type returned from the callback is a `TypeError`, thrown the same way.",
      'A library may call from a thread of its own. The JS function then runs on the JS thread, in the app\'s next frame: the arguments are taken on the calling thread (text is copied there), and the call returns to the library at once. So only a callback that returns `"void"` can be called from another thread. One that returns a value answers 0 there and logs a line. A `pointer` argument is passed as it is: the memory must still be valid a frame later.',
      "More than 4096 calls from other threads waiting for one frame are dropped, with a line in the log.",
    ],
    edgeCases: [
      "`TypeError`: the first argument is not a function, or the signature has a type a callback cannot receive or return.",
      "`NotSupportedError`: all 128 slots are taken, or the host was built without the `ffi` feature.",
      "`InvalidStateError`: reading `pointer` after `close()`. Calling `close()` again does nothing.",
    ],
    examples: [
      {
        title: "Sort with a comparison written in JS",
        lang: "ts",
        code: `import { dlopen, NativeCallback, type NativePointer } from "skathi:ffi";

const crt = dlopen("crt", {
  qsort: { args: ["buffer", "usize", "usize", "pointer"] },
});

const int32 = (p: NativePointer) => new Int32Array(p.read(4).buffer)[0]!;
const descending = new NativeCallback((a, b) => int32(b) - int32(a), {
  args: ["pointer", "pointer"],
  returns: "i32",
});

const numbers = new Int32Array([3, -1, 40, 7]);
try {
  crt.symbols.qsort(numbers, numbers.length, 4, descending.pointer);
} finally {
  descending.close();
}
numbers; // Int32Array [40, 7, 3, -1]`,
      },
      {
        title: "A callback the library calls from its own thread",
        lang: "ts",
        code: `import { dlopen, NativeCallback } from "skathi:ffi";

const player = dlopen("player", {
  player_on_progress: { args: ["pointer"] },
  player_start: { args: ["cstring"], returns: "i32" },
});

// void (*)(double seconds): the decoder thread calls it; it runs here a frame later.
const onProgress = new NativeCallback(
  (seconds) => {
    progress.value = seconds;
  },
  { args: ["f64"] },
);

player.symbols.player_on_progress(onProgress.pointer);
player.symbols.player_start("intro.ogg");

window.addEventListener("pagehide", () => {
  player.symbols.player_on_progress(null);
  onProgress.close();
});`,
      },
    ],
    related: ["dlopen", "ffi-types", "native-pointer"],
    aliases: [
      "callback",
      "function pointer",
      "JSCallback",
      "thunk",
      "NativeCallback.close",
      "NativeCallback.pointer",
      "NativeCallback.live",
      "threadsafe",
    ],
  },
]);
