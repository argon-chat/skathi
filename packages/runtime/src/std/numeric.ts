// `skathi:numeric`: fixed-width integers and floats as branded numbers, with exact arithmetic.
// Plain JS on purpose: every operation is one or two machine instructions once the JIT inlines it,
// which a call into the host would cost a hundred times over.

type Brand<T, B extends string> = T & { readonly __kind: B };

export type i8 = Brand<number, "i8">;
export type i16 = Brand<number, "i16">;
export type i32 = Brand<number, "i32">;
export type u8 = Brand<number, "u8">;
export type u16 = Brand<number, "u16">;
export type u32 = Brand<number, "u32">;
export type i64 = Brand<bigint, "i64">;
export type u64 = Brand<bigint, "u64">;
export type f16 = Brand<number, "f16">;
export type f32 = Brand<number, "f32">;
export type f64 = number;

export interface IntOps<T, Input = number> {
  /** Converts by wrapping, as an `as` cast does in Rust: `u8(257)` is 1. */
  (x: Input): T;
  /** Converts, or throws a `RangeError` when `x` is not an integer of this type. */
  checked(x: Input): T;
  /** Converts, clamping to `MIN` and `MAX`. */
  saturating(x: Input): T;
  readonly MIN: T;
  readonly MAX: T;
  readonly BITS: number;
  add(a: T, b: T): T;
  sub(a: T, b: T): T;
  mul(a: T, b: T): T;
  /** Truncates toward zero; throws a `RangeError` for a zero divisor. */
  div(a: T, b: T): T;
  rem(a: T, b: T): T;
  neg(a: T): T;
  checkedAdd(a: T, b: T): T | null;
  checkedSub(a: T, b: T): T | null;
  checkedMul(a: T, b: T): T | null;
  saturatingAdd(a: T, b: T): T;
  saturatingSub(a: T, b: T): T;
  saturatingMul(a: T, b: T): T;
  overflowingAdd(a: T, b: T): [T, boolean];
  overflowingSub(a: T, b: T): [T, boolean];
  overflowingMul(a: T, b: T): [T, boolean];
  and(a: T, b: T): T;
  or(a: T, b: T): T;
  xor(a: T, b: T): T;
  not(a: T): T;
  /** The shift count is taken modulo `BITS`. */
  shl(a: T, n: number): T;
  /** Arithmetic for a signed type, logical for an unsigned one. */
  shr(a: T, n: number): T;
  rotl(a: T, n: number): T;
  rotr(a: T, n: number): T;
  clz(a: T): number;
  ctz(a: T): number;
  popcount(a: T): number;
  lt(a: T, b: T): boolean;
}

export interface FloatOps<T> {
  /** Rounds to this type's nearest value. */
  (x: number): T;
  add(a: T, b: T): T;
  sub(a: T, b: T): T;
  mul(a: T, b: T): T;
  div(a: T, b: T): T;
  /** `a * b + c`, rounded once at the end. */
  fma(a: T, b: T, c: T): T;
}

type Building<O> = { -readonly [K in keyof O]: O[K] };

const divisionByZero = (name: string) => new RangeError(`${name}: division by zero`);

function popcount32(x: number): number {
  x -= (x >>> 1) & 0x55555555;
  x = (x & 0x33333333) + ((x >>> 2) & 0x33333333);
  return Math.imul((x + (x >>> 4)) & 0x0f0f0f0f, 0x01010101) >>> 24;
}

function int<T extends number>(name: string, bits: 8 | 16 | 32, signed: boolean): IntOps<T> {
  const spare = 32 - bits;
  const mask = bits === 32 ? 0xffffffff : (1 << bits) - 1;
  const min = signed ? -(2 ** (bits - 1)) : 0;
  const max = signed ? 2 ** (bits - 1) - 1 : 2 ** bits - 1;
  const wrap = (signed ? (x: number) => (x << spare) >> spare : (x: number) => (x & mask) >>> 0) as (x: number) => T;
  const fits = (x: number) => x >= min && x <= max;
  const clamp = (x: number) => (x < min ? min : x > max ? max : x) as T;
  const bitsOf = (x: number) => (x & mask) >>> 0;

  const ops = ((x: number) => wrap(x)) as ((x: number) => T) & Building<IntOps<T>>;
  ops.MIN = min as T;
  ops.MAX = max as T;
  ops.BITS = bits;
  ops.checked = (x) => {
    if (!Number.isInteger(x) || !fits(x)) throw new RangeError(`${name}.checked: ${x} is not an ${name}`);
    return x as T;
  };
  ops.saturating = (x) => (x !== x ? (0 as T) : clamp(Math.trunc(x)));
  ops.add = (a, b) => wrap(a + b);
  ops.sub = (a, b) => wrap(a - b);
  ops.mul = (a, b) => wrap(Math.imul(a, b));
  ops.div = (a, b) => {
    if (b === 0) throw divisionByZero(`${name}.div`);
    return wrap(Math.trunc(a / b));
  };
  ops.rem = (a, b) => {
    if (b === 0) throw divisionByZero(`${name}.rem`);
    return wrap(a % b);
  };
  ops.neg = (a) => wrap(-a);
  ops.checkedAdd = (a, b) => (fits(a + b) ? ((a + b) as T) : null);
  ops.checkedSub = (a, b) => (fits(a - b) ? ((a - b) as T) : null);
  ops.checkedMul = (a, b) => (fits(a * b) ? ((a * b) as T) : null);
  ops.saturatingAdd = (a, b) => clamp(a + b);
  ops.saturatingSub = (a, b) => clamp(a - b);
  ops.saturatingMul = (a, b) => clamp(a * b);
  ops.overflowingAdd = (a, b) => [wrap(a + b), !fits(a + b)];
  ops.overflowingSub = (a, b) => [wrap(a - b), !fits(a - b)];
  ops.overflowingMul = (a, b) => [wrap(Math.imul(a, b)), !fits(a * b)];
  ops.and = (a, b) => wrap(a & b);
  ops.or = (a, b) => wrap(a | b);
  ops.xor = (a, b) => wrap(a ^ b);
  ops.not = (a) => wrap(~a);
  ops.shl = (a, n) => wrap(a << (n & (bits - 1)));
  ops.shr = signed ? (a, n) => wrap(a >> (n & (bits - 1))) : (a, n) => wrap(a >>> (n & (bits - 1)));
  ops.rotl = (a, n) => {
    const by = n & (bits - 1);
    const raw = bitsOf(a);
    return wrap(by === 0 ? raw : (raw << by) | (raw >>> (bits - by)));
  };
  ops.rotr = (a, n) => ops.rotl(a, bits - (n & (bits - 1)));
  ops.clz = (a) => Math.clz32(bitsOf(a)) - spare;
  ops.ctz = (a) => {
    const raw = bitsOf(a);
    return raw === 0 ? bits : 31 - Math.clz32(raw & -raw);
  };
  ops.popcount = (a) => popcount32(bitsOf(a));
  ops.lt = (a, b) => a < b;
  return ops;
}

function int64<T extends bigint>(name: string, signed: boolean): IntOps<T, number | bigint> {
  const wrap = (signed ? (x: bigint) => BigInt.asIntN(64, x) : (x: bigint) => BigInt.asUintN(64, x)) as (
    x: bigint,
  ) => T;
  const min = signed ? -(2n ** 63n) : 0n;
  const max = signed ? 2n ** 63n - 1n : 2n ** 64n - 1n;
  const fits = (x: bigint) => x >= min && x <= max;
  const clamp = (x: bigint) => (x < min ? min : x > max ? max : x) as T;
  const big = (x: number | bigint) => (typeof x === "bigint" ? x : Number.isFinite(x) ? BigInt(Math.trunc(x)) : 0n);
  const bitsOf = (x: bigint) => BigInt.asUintN(64, x);
  const count = (n: number) => BigInt(n & 63);

  const ops = ((x: number | bigint) => wrap(big(x))) as ((x: number | bigint) => T) &
    Building<IntOps<T, number | bigint>>;
  ops.MIN = min as T;
  ops.MAX = max as T;
  ops.BITS = 64;
  ops.checked = (x) => {
    if ((typeof x === "number" && !Number.isInteger(x)) || !fits(big(x))) {
      throw new RangeError(`${name}.checked: ${x} is not an ${name}`);
    }
    return big(x) as T;
  };
  ops.saturating = (x) =>
    clamp(typeof x === "number" && !Number.isFinite(x) ? (x > 0 ? max : x < 0 ? min : 0n) : big(x));
  ops.add = (a: bigint, b: bigint) => wrap(a + b);
  ops.sub = (a: bigint, b: bigint) => wrap(a - b);
  ops.mul = (a: bigint, b: bigint) => wrap(a * b);
  ops.div = (a: bigint, b: bigint) => {
    if (b === 0n) throw divisionByZero(`${name}.div`);
    return wrap(a / b);
  };
  ops.rem = (a: bigint, b: bigint) => {
    if (b === 0n) throw divisionByZero(`${name}.rem`);
    return wrap(a % b);
  };
  ops.neg = (a: bigint) => wrap(-a);
  ops.checkedAdd = (a: bigint, b: bigint) => (fits(a + b) ? ((a + b) as T) : null);
  ops.checkedSub = (a: bigint, b: bigint) => (fits(a - b) ? ((a - b) as T) : null);
  ops.checkedMul = (a: bigint, b: bigint) => (fits(a * b) ? ((a * b) as T) : null);
  ops.saturatingAdd = (a: bigint, b: bigint) => clamp(a + b);
  ops.saturatingSub = (a: bigint, b: bigint) => clamp(a - b);
  ops.saturatingMul = (a: bigint, b: bigint) => clamp(a * b);
  ops.overflowingAdd = (a: bigint, b: bigint) => [wrap(a + b), !fits(a + b)];
  ops.overflowingSub = (a: bigint, b: bigint) => [wrap(a - b), !fits(a - b)];
  ops.overflowingMul = (a: bigint, b: bigint) => [wrap(a * b), !fits(a * b)];
  ops.and = (a: bigint, b: bigint) => wrap(a & b);
  ops.or = (a: bigint, b: bigint) => wrap(a | b);
  ops.xor = (a: bigint, b: bigint) => wrap(a ^ b);
  ops.not = (a: bigint) => wrap(~a);
  ops.shl = (a: bigint, n: number) => wrap(a << count(n));
  ops.shr = signed
    ? (a: bigint, n: number) => wrap(a >> count(n))
    : (a: bigint, n: number) => wrap(bitsOf(a) >> count(n));
  ops.rotl = (a: bigint, n: number) => {
    const raw = bitsOf(a);
    return wrap((raw << count(n)) | (raw >> (64n - count(n))));
  };
  ops.rotr = (a: bigint, n: number) => ops.rotl(a as T, 64 - (n & 63));
  ops.clz = (a: bigint) => {
    const raw = bitsOf(a);
    const high = Number(raw >> 32n);
    return high !== 0 ? Math.clz32(high) : 32 + Math.clz32(Number(raw & 0xffffffffn));
  };
  ops.ctz = (a: bigint) => {
    const raw = bitsOf(a);
    return raw === 0n ? 64 : 63 - ops.clz((raw & -raw) as T);
  };
  ops.popcount = (a: bigint) => {
    const raw = bitsOf(a);
    return popcount32(Number(raw >> 32n)) + popcount32(Number(raw & 0xffffffffn));
  };
  ops.lt = (a: bigint, b: bigint) => a < b;
  return ops;
}

function float<T extends number>(round: (x: number) => number): FloatOps<T> {
  const to = round as (x: number) => T;
  const ops = ((x: number) => to(x)) as FloatOps<T>;
  ops.add = (a, b) => to(a + b);
  ops.sub = (a, b) => to(a - b);
  ops.mul = (a, b) => to(a * b);
  ops.div = (a, b) => to(a / b);
  ops.fma = (a, b, c) => to(a * b + c);
  return ops;
}

export const i8: IntOps<i8> = int("i8", 8, true);
export const i16: IntOps<i16> = int("i16", 16, true);
export const i32: IntOps<i32> = int("i32", 32, true);
export const u8: IntOps<u8> = int("u8", 8, false);
export const u16: IntOps<u16> = int("u16", 16, false);
export const u32: IntOps<u32> = int("u32", 32, false);
export const i64: IntOps<i64, number | bigint> = int64("i64", true);
export const u64: IntOps<u64, number | bigint> = int64("u64", false);
export const f16: FloatOps<f16> = float(Math.f16round);
export const f32: FloatOps<f32> = float(Math.fround);
