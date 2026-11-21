import { describe, expect, test } from "bun:test";
import { f16, f32, i16, i32, i64, i8, u16, u32, u64, u8 } from "../src/std/numeric";

describe("skathi:numeric", () => {
  test("conversion wraps, checks or saturates", () => {
    expect<unknown[]>([u8(257), u8(-1), i8(128), i8(-129), i16(0x8000), u16(-1)]).toEqual([
      1, 255, -128, 127, -32768, 65535,
    ]);
    expect<unknown[]>([i32(2 ** 31), u32(-1), u32(2 ** 32 + 5), i32(1.9), i32(NaN)]).toEqual([
      -(2 ** 31),
      2 ** 32 - 1,
      5,
      1,
      0,
    ]);
    expect(() => u8.checked(256)).toThrow(RangeError);
    expect(() => i32.checked(1.5)).toThrow(RangeError);
    expect<unknown>(u32.checked(2 ** 32 - 1)).toBe(u32.MAX);
    expect<unknown[]>([u8.saturating(300), i8.saturating(-300), i8.saturating(NaN), u32.saturating(Infinity)]).toEqual([
      255,
      -128,
      0,
      2 ** 32 - 1,
    ]);
  });

  test("arithmetic stays in the type", () => {
    expect<unknown>(i32.add(i32.MAX, i32(1))).toBe(i32.MIN);
    expect<unknown>(u32.sub(u32(0), u32(1))).toBe(u32.MAX);
    expect<unknown>(u32.mul(u32(0xffffffff), u32(0xffffffff))).toBe(u32(1));
    expect<unknown>(i32.mul(i32(0x7fffffff), i32(0x7fffffff))).toBe(i32(1));
    expect<unknown>(u8.mul(u8(200), u8(3))).toBe(u8(88));
    expect<unknown[]>([i32.div(i32(-7), i32(2)), i32.rem(i32(-7), i32(2)), i32.div(i32.MIN, i32(-1))]).toEqual([
      -3,
      -1,
      i32.MIN,
    ]);
    expect<unknown>(u32.div(u32.MAX, u32(2))).toBe(u32(0x7fffffff));
    expect(() => u8.div(u8(1), u8(0))).toThrow(RangeError);
    expect<unknown>(i8.neg(i8.MIN)).toBe(i8.MIN);
  });

  test("checked, saturating and overflowing forms report the overflow", () => {
    expect<unknown[]>([i8.checkedAdd(i8(100), i8(27)), i8.checkedAdd(i8(100), i8(28))]).toEqual([127, null]);
    expect<unknown>(u32.checkedMul(u32(65536), u32(65536))).toBeNull();
    expect<unknown>(u32.checkedMul(u32(65535), u32(65537))).toBe(u32.MAX);
    expect<unknown[]>([u8.saturatingSub(u8(1), u8(2)), i16.saturatingMul(i16(300), i16(300))]).toEqual([0, 32767]);
    expect<unknown[]>(u8.overflowingAdd(u8(255), u8(2))).toEqual([u8(1), true]);
    expect<unknown[]>(i32.overflowingMul(i32(3), i32(4))).toEqual([i32(12), false]);
  });

  test("shifts, rotations and bit counts use the type's width", () => {
    expect<unknown[]>([u8.shl(u8(0x81), 1), u8.shl(u8(1), 9), i8.shr(i8(-128), 7), u8.shr(u8(0x80), 7)]).toEqual([
      2, 2, -1, 1,
    ]);
    expect<unknown[]>([u32.shr(u32(0x80000000), 31), i32.shr(i32.MIN, 31)]).toEqual([1, -1]);
    expect<unknown[]>([
      u8.rotl(u8(0x81), 1),
      u8.rotr(u8(0x81), 1),
      u32.rotl(u32(0x80000001), 4),
      u16.rotl(u16(1), 16),
    ]).toEqual([0x03, 0xc0, 0x18, 1]);
    expect<unknown[]>([u8.clz(u8(1)), i8.clz(i8(-1)), u16.clz(u16(0)), u32.clz(u32(1))]).toEqual([7, 0, 16, 31]);
    expect<unknown[]>([u8.ctz(u8(0)), u8.ctz(u8(0x80)), u32.ctz(u32(0x80000000)), i32.ctz(i32(12))]).toEqual([
      8, 7, 31, 2,
    ]);
    expect<unknown[]>([u8.popcount(u8(0xff)), i8.popcount(i8(-1)), u32.popcount(u32.MAX)]).toEqual([8, 8, 32]);
    expect<unknown>(u8.not(u8(0))).toBe(u8.MAX);
  });

  test("the 64-bit types are bigint with the same rules", () => {
    expect<unknown>(i64.add(i64.MAX, i64(1))).toBe(i64.MIN);
    expect<unknown>(u64(-1)).toBe(u64.MAX);
    expect<unknown>(u64.mul(u64.MAX, u64.MAX)).toBe(u64(1));
    expect<unknown>(i64.checkedMul(i64(2n ** 62n), i64(2))).toBeNull();
    expect<unknown>(i64.saturating(Infinity)).toBe(i64.MAX);
    expect(() => i64.checked(1.5)).toThrow(RangeError);
    expect(() => u64.checked(-1)).toThrow(RangeError);
    expect<unknown>(i64.div(i64(-7), i64(2))).toBe(i64(-3));
    expect<unknown[]>([u64.shr(u64.MAX, 63), i64.shr(i64.MIN, 63)]).toEqual([1n, -1n]);
    expect<unknown[]>([u64.rotl(u64(1n << 63n), 1), u64.rotr(u64(1), 1), u64.rotl(u64(5), 64)]).toEqual([
      1n,
      1n << 63n,
      5n,
    ]);
    expect<unknown[]>([
      u64.clz(u64(1)),
      u64.clz(u64(0)),
      i64.clz(i64(-1)),
      u64.ctz(u64(1n << 40n)),
      u64.ctz(u64(0)),
    ]).toEqual([63, 64, 0, 40, 64]);
    expect<unknown>(u64.popcount(u64.MAX)).toBe(64);
  });

  test("floats round after every step", () => {
    expect<unknown>(f32(0.1)).toBe(Math.fround(0.1));
    expect<unknown>(f32.add(f32(0.1), f32(0.2))).toBe(Math.fround(Math.fround(0.1) + Math.fround(0.2)));
    expect<unknown>(f32.add(f32(16777216), f32(1))).toBe(16777216);
    expect<unknown>(f16(65520)).toBe(Infinity);
    expect<unknown>(f16.mul(f16(1.5), f16(2))).toBe(3);
  });
});
