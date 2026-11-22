import { OK, importLine, section } from "./shared";

const BRAND =
  "The brand is a compile-time check only. Passing a plain `number` where a `u8` is expected is a type error, and the way in is the type's function. At run time a value is an ordinary `number` or `bigint`, so it costs nothing to hold.";
const PLAIN_JS = "The module is plain JavaScript and works the same on every platform.";
const COMPARE = "Values compare with the ordinary operators (`<`, `===`). `lt(a, b)` is there for symmetry.";
const OPERATORS = (type: string, plain: string, unlike: string) =>
  `Operators on two values of one type are plain JavaScript: \`a + b\` for two \`${type}\`s is a \`${plain}\` and is not ${unlike}. Pass it through the type's function, \`${type}(a + b)\`, or call \`${type}.add(a, b)\`. Two different types in one operator, such as a \`u8\` and an \`i32\`, are an \`sk check\` error ([SK7001](/sk-check/sk7001/)): convert one side first.`;
const INLINED = (forms: string, example: string) =>
  `\`sk build\` inlines ${forms} in the files that import \`skathi:numeric\`, with the same result for every input: ${example}. In a .vue or .svelte component, in an app with a Vite config, and under \`sk dev\`, they stay calls.`;

export const numeric = section("skathi:numeric", [
  {
    slug: "integer-types",
    name: "i8, i16, i32, u8, u16, u32",
    kind: "namespace",
    summary:
      "Integers of 8, 16 and 32 bits, signed and unsigned, as branded `number`s with wrapping, checked, saturating and overflowing arithmetic.",
    syntax: `${importLine("skathi:numeric", "i8", "i16", "i32", "u8", "u16", "u32")}

type u8 = number & { readonly __kind: "u8" }

u8(x: number): u8
u8.checked(x: number): u8
u8.saturating(x: number): u8

readonly u8.MIN: u8
readonly u8.MAX: u8
readonly u8.BITS: number

u8.add(a: u8, b: u8): u8
u8.sub(a: u8, b: u8): u8
u8.mul(a: u8, b: u8): u8
u8.div(a: u8, b: u8): u8
u8.rem(a: u8, b: u8): u8
u8.neg(a: u8): u8

u8.checkedAdd(a: u8, b: u8): u8 | null
u8.checkedSub(a: u8, b: u8): u8 | null
u8.checkedMul(a: u8, b: u8): u8 | null
u8.saturatingAdd(a: u8, b: u8): u8
u8.saturatingSub(a: u8, b: u8): u8
u8.saturatingMul(a: u8, b: u8): u8
u8.overflowingAdd(a: u8, b: u8): [u8, boolean]
u8.overflowingSub(a: u8, b: u8): [u8, boolean]
u8.overflowingMul(a: u8, b: u8): [u8, boolean]

u8.and(a: u8, b: u8): u8
u8.or(a: u8, b: u8): u8
u8.xor(a: u8, b: u8): u8
u8.not(a: u8): u8
u8.shl(a: u8, n: number): u8
u8.shr(a: u8, n: number): u8
u8.rotl(a: u8, n: number): u8
u8.rotr(a: u8, n: number): u8
u8.clz(a: u8): number
u8.ctz(a: u8): number
u8.popcount(a: u8): number

u8.lt(a: u8, b: u8): boolean`,
    values: [
      { value: "i8", status: OK, note: "8 bits, signed: -128 to 127. Stored as `number`. Typed array: `Int8Array`." },
      { value: "u8", status: OK, note: "8 bits, unsigned: 0 to 255. Stored as `number`. Typed array: `Uint8Array`." },
      {
        value: "i16",
        status: OK,
        note: "16 bits, signed: -32768 to 32767. Stored as `number`. Typed array: `Int16Array`.",
      },
      {
        value: "u16",
        status: OK,
        note: "16 bits, unsigned: 0 to 65535. Stored as `number`. Typed array: `Uint16Array`.",
      },
      {
        value: "i32",
        status: OK,
        note: "32 bits, signed: -2147483648 to 2147483647. Stored as `number`. Typed array: `Int32Array`.",
      },
      {
        value: "u32",
        status: OK,
        note: "32 bits, unsigned: 0 to 4294967295. Stored as `number`. Typed array: `Uint32Array`.",
      },
    ],
    notes: [
      "Each type is a function and a namespace. The six types share one API; the signatures above use `u8`.",
      "Conversion: `u8(x)` wraps as a cast does in Rust or C, so `u8(257)` is 1, `u8(-1)` is 255 and `i8(200)` is -56. A fraction is dropped. `u8.checked(x)` throws for a value that is not a `u8`. `u8.saturating(x)` drops the fraction and clamps to `MIN` and `MAX`.",
      "Constants: `MIN`, `MAX` and `BITS`.",
      "`add`, `sub`, `mul`, `div`, `rem` and `neg` wrap. `mul` is exact for 32 bits: `u32.mul(u32(0xffffffff), u32(2))` is 4294967294.",
      "`div` truncates toward zero: `i32.div(i32(-7), i32(2))` is -3. `rem` has the sign of the dividend.",
      "`checkedAdd`, `checkedSub` and `checkedMul` return `null` on overflow. `saturatingAdd`, `saturatingSub` and `saturatingMul` clamp. `overflowingAdd`, `overflowingSub` and `overflowingMul` return `[value, overflowed]` with the wrapped value.",
      "Bits: `and`, `or`, `xor`, `not`, `shl`, `shr`, `rotl`, `rotr`, `clz`, `ctz`, `popcount`. `shr` is arithmetic for the signed types and logical for the unsigned ones. A shift or rotation count is taken modulo `BITS`.",
      "`clz` and `ctz` count within the type's width: `u8.clz(u8(1))` is 7, and `u8.ctz(u8(0))` is 8.",
      COMPARE,
      OPERATORS("i32", "number", "wrapped"),
      INLINED(
        "the conversion, `MIN`, `MAX`, `BITS`, `add`, `sub`, `mul`, `neg`, `and`, `or`, `xor`, `not`, `shl`, `shr`, `clz` and `lt`",
        "`i32.add(a, b)` becomes `(a + b) | 0`",
      ),
      BRAND,
      PLAIN_JS,
    ],
    examples: [
      {
        title: "Four ways to add past the maximum",
        lang: "ts",
        code: `import { i32, u8, u32 } from "skathi:numeric";

const a = u8(250);
u8.add(a, u8(10)); // 4: wraps
u8.checkedAdd(a, u8(10)); // null
u8.saturatingAdd(a, u8(10)); // 255
u8.overflowingAdd(a, u8(10)); // [4, true]

u32.mul(u32(0xffffffff), u32(2)); // 4294967294, exact
i32.div(i32(-7), i32(2)); // -3: truncates`,
      },
      {
        title: "FNV-1a hash in 32 bits",
        lang: "ts",
        code: `import { u32 } from "skathi:numeric";

export function fnv1a(bytes: Uint8Array): u32 {
  let hash = u32(2166136261);
  for (const byte of bytes) hash = u32.mul(u32.xor(hash, u32(byte)), u32(16777619));
  return hash;
}

fnv1a(new TextEncoder().encode("skathi")); // 4079123825`,
      },
      {
        title: "Accept a value from outside",
        lang: "ts",
        code: `import { u8 } from "skathi:numeric";

u8(257); // 1: wraps
u8.saturating(300); // 255
u8.saturating(-5); // 0

export function parseLevel(input: string): u8 | null {
  try {
    return u8.checked(Number(input));
  } catch {
    return null;
  }
}`,
      },
    ],
    edgeCases: [
      "`checked(x)` throws a `RangeError` when `x` is not an integer or is outside `MIN` to `MAX`: `u8.checked: 300 is not an u8`. `NaN` and a fraction throw too.",
      "`div` and `rem` throw a `RangeError` for a zero divisor: `i32.div: division by zero`.",
      "`NaN` converts to 0, with the function and with `saturating`. `Infinity` converts to 0 with the function and clamps with `saturating`.",
      "`i8.div(i8.MIN, i8(-1))` and `i8.neg(i8.MIN)` wrap back to `MIN`.",
    ],
    related: ["integer-types-64", "float-types", "binary-reader", "binary-writer"],
    aliases: [
      "i8",
      "i16",
      "i32",
      "u8",
      "u16",
      "u32",
      "IntOps",
      "fixed-width integer",
      "wrapping arithmetic",
      "checked arithmetic",
      "saturating arithmetic",
      "overflow",
      "u8.checked",
      "u8.saturating",
      "u8.add",
      "u8.checkedAdd",
      "u8.saturatingAdd",
      "u8.overflowingAdd",
      "u32.mul",
      "i32.div",
      "rotl",
      "rotr",
      "clz",
      "ctz",
      "popcount",
      "MIN",
      "MAX",
      "BITS",
      "branded number",
    ],
  },
  {
    slug: "integer-types-64",
    name: "i64, u64",
    kind: "namespace",
    summary:
      "Integers of 64 bits, signed and unsigned, as branded `bigint`s with the same functions as the smaller integer types.",
    syntax: `${importLine("skathi:numeric", "i64", "u64")}

type u64 = bigint & { readonly __kind: "u64" }

u64(x: number | bigint): u64
u64.checked(x: number | bigint): u64
u64.saturating(x: number | bigint): u64

readonly u64.MIN: u64
readonly u64.MAX: u64
readonly u64.BITS: number

u64.add(a: u64, b: u64): u64
u64.sub(a: u64, b: u64): u64
u64.mul(a: u64, b: u64): u64
u64.div(a: u64, b: u64): u64
u64.rem(a: u64, b: u64): u64
u64.neg(a: u64): u64

u64.checkedAdd(a: u64, b: u64): u64 | null
u64.checkedSub(a: u64, b: u64): u64 | null
u64.checkedMul(a: u64, b: u64): u64 | null
u64.saturatingAdd(a: u64, b: u64): u64
u64.saturatingSub(a: u64, b: u64): u64
u64.saturatingMul(a: u64, b: u64): u64
u64.overflowingAdd(a: u64, b: u64): [u64, boolean]
u64.overflowingSub(a: u64, b: u64): [u64, boolean]
u64.overflowingMul(a: u64, b: u64): [u64, boolean]

u64.and(a: u64, b: u64): u64
u64.or(a: u64, b: u64): u64
u64.xor(a: u64, b: u64): u64
u64.not(a: u64): u64
u64.shl(a: u64, n: number): u64
u64.shr(a: u64, n: number): u64
u64.rotl(a: u64, n: number): u64
u64.rotr(a: u64, n: number): u64
u64.clz(a: u64): number
u64.ctz(a: u64): number
u64.popcount(a: u64): number

u64.lt(a: u64, b: u64): boolean`,
    values: [
      {
        value: "i64",
        status: OK,
        note: "64 bits, signed: -9223372036854775808n to 9223372036854775807n. Stored as `bigint`. Typed array: `BigInt64Array`.",
      },
      {
        value: "u64",
        status: OK,
        note: "64 bits, unsigned: 0n to 18446744073709551615n. Stored as `bigint`. Typed array: `BigUint64Array`.",
      },
    ],
    notes: [
      "The API is the one of the [smaller integer types](/std/integer-types/). The differences: values are `bigint`, and the three conversions take a `number` or a `bigint`.",
      "Conversion: `u64(x)` wraps, so `u64(-1)` is 18446744073709551615n and `i64(2n ** 63n)` is `i64.MIN`. A `number` loses its fraction first. `u64.checked(x)` throws for a value that is not a `u64`. `u64.saturating(x)` clamps, and takes `Infinity` and `-Infinity` to `MAX` and `MIN`.",
      "`add`, `sub`, `mul`, `div`, `rem` and `neg` wrap. `div` truncates toward zero. The `checked`, `saturating` and `overflowing` forms of `add`, `sub` and `mul` behave as they do for the smaller types.",
      "`shr` is arithmetic for `i64` and logical for `u64`. A shift or rotation count is a `number`, taken modulo 64.",
      "`clz`, `ctz` and `popcount` return a `number`. `u64.ctz(u64(0))` is 64.",
      "`MIN`, `MAX` are `bigint`s; `BITS` is the number 64.",
      COMPARE,
      OPERATORS("u64", "bigint", "wrapped"),
      INLINED(
        "the conversion of a `bigint`, `MIN`, `MAX`, `BITS`, `add`, `sub`, `mul`, `neg`, `and`, `or`, `xor`, `not` and `lt`",
        "`u64.add(a, b)` becomes `BigInt.asUintN(64, a + b)`",
      ),
      BRAND,
      PLAIN_JS,
    ],
    examples: [
      {
        title: "Wrap, check and rotate",
        lang: "ts",
        code: `import { i64, u64 } from "skathi:numeric";

u64.add(u64.MAX, u64(1)); // 0n: wraps
u64.checkedAdd(u64.MAX, u64(1)); // null
u64.overflowingMul(u64(1n << 63n), u64(2)); // [0n, true]
u64.rotl(u64(1n << 63n), 1); // 1n
i64.div(i64(-7), i64(2)); // -3n: truncates
u64.popcount(u64.MAX); // 64`,
      },
      {
        title: "FNV-1a hash in 64 bits",
        lang: "ts",
        code: `import { u64 } from "skathi:numeric";

export function fnv1a64(bytes: Uint8Array): u64 {
  let hash = u64(0xcbf29ce484222325n);
  for (const byte of bytes) hash = u64.mul(u64.xor(hash, u64(byte)), u64(0x100000001b3n));
  return hash;
}

fnv1a64(new TextEncoder().encode("skathi")).toString(16); // "f0d49a4c3fccf451"`,
      },
    ],
    edgeCases: [
      "`checked(x)` throws a `RangeError` for a `number` that is not an integer and for a value outside `MIN` to `MAX`: `u64.checked: -1 is not an u64`.",
      "`div` and `rem` throw a `RangeError` for a zero divisor: `i64.div: division by zero`.",
      "`NaN` converts to 0n. `Infinity` converts to 0n with the function and to `MAX` with `saturating`.",
      "Mixing a plain `bigint` or a `number` into the arithmetic functions is a type error; convert with `u64(x)` first.",
    ],
    related: ["integer-types", "float-types", "crc64", "struct-layout"],
    aliases: [
      "i64",
      "u64",
      "bigint",
      "64-bit integer",
      "IntOps",
      "u64.checked",
      "u64.saturating",
      "u64.add",
      "u64.mul",
      "u64.rotl",
      "u64.popcount",
      "i64.div",
      "wrapping arithmetic",
    ],
  },
  {
    slug: "float-types",
    name: "f16, f32",
    kind: "namespace",
    summary: "Half and single precision floats as branded `number`s that round to their width after every step.",
    syntax: `${importLine("skathi:numeric", "f16", "f32")}

type f32 = number & { readonly __kind: "f32" }
type f64 = number

f32(x: number): f32
f32.add(a: f32, b: f32): f32
f32.sub(a: f32, b: f32): f32
f32.mul(a: f32, b: f32): f32
f32.div(a: f32, b: f32): f32
f32.fma(a: f32, b: f32, c: f32): f32`,
    values: [
      { value: "f16", status: OK, note: "IEEE 754 half precision. Stored as `number`. Typed array: `Float16Array`." },
      { value: "f32", status: OK, note: "IEEE 754 single precision. Stored as `number`. Typed array: `Float32Array`." },
    ],
    notes: [
      "Each type is a function and a namespace. `f16` and `f32` share one API; the signatures above use `f32`.",
      "`f32(x)` rounds to the nearest `f32`, as `Math.fround` does. `f16(x)` rounds to the nearest `f16`, as `Math.f16round` does.",
      "`add`, `sub`, `mul` and `div` compute the result and round it to the type. A chain of them gives what the same chain gives in a language with native floats of that width.",
      "`fma(a, b, c)` is `a * b + c`, rounded once at the end.",
      "`f64` is exported as a type only. It is an alias of `number`.",
      OPERATORS("f32", "number", "rounded"),
      INLINED("the conversion, `add`, `sub`, `mul`, `div` and `fma`", "`f32.add(a, b)` becomes `Math.fround(a + b)`"),
      BRAND,
      PLAIN_JS,
    ],
    examples: [
      {
        title: "Match the arithmetic of a Float32Array",
        lang: "ts",
        code: `import { f32 } from "skathi:numeric";

f32(0.1); // 0.10000000149011612
f32.add(f32(0.1), f32(0.2)); // 0.30000001192092896, which is f32(0.3)
f32.add(f32(16777216), f32(1)); // 16777216: 16777217 is not an f32
f32.fma(f32(1.5), f32(2), f32(0.25)); // 3.25`,
      },
      {
        title: "Half precision",
        lang: "ts",
        code: `import { f16 } from "skathi:numeric";

f16(0.1); // 0.0999755859375
f16.add(f16(2048), f16(1)); // 2048: 2049 is not an f16
f16(65536); // Infinity`,
      },
    ],
    edgeCases: [
      "A value beyond the type's range rounds to `Infinity` or `-Infinity`: `f16(65536)` and `f32(1e40)` are `Infinity`.",
      "`div` by zero does not throw. It gives `Infinity`, `-Infinity` or `NaN`, as `/` does.",
    ],
    related: ["integer-types", "integer-types-64", "binary-reader"],
    aliases: [
      "f16",
      "f32",
      "f64",
      "FloatOps",
      "half float",
      "single precision",
      "Math.fround",
      "Math.f16round",
      "f32.add",
      "f32.sub",
      "f32.mul",
      "f32.div",
      "f32.fma",
      "fused multiply-add",
    ],
  },
]);
