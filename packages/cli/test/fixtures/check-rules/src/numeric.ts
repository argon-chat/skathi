import { i32, u8, type f32, type i64, type u64 } from "skathi:numeric";

declare const a: i32, b: i32, small: u8, ratio: f32, n: number, big: i64, unsigned: u64;
declare let total: i32;

export const mixed = [a + small, small < a, ratio * a, big + unsigned];
total += small;

export const fine = [a + b, a + 1, a * n, i32(small) + a, i32.add(a, b), small + small, a === b, -a + b];
