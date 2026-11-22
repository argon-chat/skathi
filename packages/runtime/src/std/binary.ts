// `skathi:binary`: cursors over bytes, bit streams, byte order, LEB128 and CRCs. The host installs the objects.

import type { NativeBuffer } from "./memory";

/** Bytes read in place: a `NativeBuffer` freed by then throws `InvalidStateError`. */
export type Bytes = BufferSource | NativeBuffer;
export type Endian = "little" | "big";
export type BitOrder = "msb" | "lsb";

/** A field of a struct layout. `u64` and `i64` are `bigint` in JS, the rest are `number`. */
export type FieldType = "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "u64" | "i64" | "f32" | "f64";
export type StructLayout = Record<string, FieldType>;
export type StructOf<L extends StructLayout> = {
  [K in keyof L]: L[K] extends "u64" | "i64" ? bigint : number;
};

type NumberArray =
  | Int8Array
  | Uint8Array
  | Uint8ClampedArray
  | Int16Array
  | Uint16Array
  | Int32Array
  | Uint32Array
  | Float16Array
  | Float32Array
  | Float64Array
  | BigInt64Array
  | BigUint64Array;

/** Reads many values per call. For one scalar at a time use `view` at `position` and move `position`. */
export interface BinaryReader {
  /** The byte the next read starts at. Setting it past the end throws a `RangeError`. */
  position: number;
  readonly remaining: number;
  /** The whole source, whatever the position. */
  readonly view: DataView;
  /** `count` elements as a new typed array of `type`, read in the reader's byte order. */
  readArray<T extends NumberArray>(type: new (length: number) => T, count: number): T;
  /** The layout's fields one after another, with no padding between them. */
  readStruct<L extends StructLayout>(layout: L): StructOf<L>;
  /** The next `n` bytes as a view of the source, not a copy. */
  bytes(n: number): Uint8Array;
  /** The next `n` bytes as text; `encoding` is a `TextDecoder` label, `"utf-8"` by default. */
  string(n: number, encoding?: string): string;
  /** `count` unsigned LEB128 values of up to 32 bits. */
  varUints(count: number): Uint32Array;
}

export interface BinaryReaderConstructor {
  readonly prototype: BinaryReader;
  /**
   * A reader over `source` itself, without a copy. `endian` is `"little"` by default. Over a
   * `NativeBuffer`, a read once it is freed throws `InvalidStateError`.
   */
  new (source: Bytes, options?: { endian?: Endian }): BinaryReader;
}

export interface BinaryWriter {
  /** What is written so far, for patching a header. Take it again after writing more. */
  readonly view: DataView;
  readonly length: number;
  /** Each value as `type`; integers wrap as they do in a typed array. */
  writeAll(values: ArrayLike<number>, type: "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "f32" | "f64"): this;
  /** The layout's fields of `value`, one after another, with no padding between them. */
  writeStruct<L extends StructLayout>(layout: L, value: { [K in keyof L]: number | bigint }): this;
  bytes(source: Bytes): this;
  /** The bytes written, handed over without a copy; the writer is empty again. */
  finish(): Uint8Array;
}

export interface BinaryWriterConstructor {
  readonly prototype: BinaryWriter;
  new (options?: { endian?: Endian; initialCapacity?: number }): BinaryWriter;
}

export interface BitReader {
  /** The next `bits` bits (1 to 32) as an unsigned number. */
  read(bits: number): number;
  readMany(bits: number, count: number): Uint32Array;
  /** Skips to the next byte boundary. */
  align(): void;
}

export interface BitReaderConstructor {
  readonly prototype: BitReader;
  /** `"msb"` takes a byte's high bit first and is the default; `"lsb"` is DEFLATE's order. */
  new (source: Uint8Array, order?: BitOrder): BitReader;
}

export interface BitWriter {
  /** The low `bits` bits (1 to 32) of `value`. */
  write(value: number, bits: number): this;
  writeMany(values: ArrayLike<number>, bits: number): this;
  /** The bytes written, the last one padded with zero bits; the writer is empty again. */
  finish(): Uint8Array;
}

export interface BitWriterConstructor {
  readonly prototype: BitWriter;
  new (order?: BitOrder): BitWriter;
}

export interface EndianOps {
  /** This machine's byte order. */
  readonly native: Endian;
  /** Reverses the bytes of every element in place. */
  swap16(array: Uint16Array | Int16Array | Float16Array): void;
  swap32(array: Uint32Array | Int32Array | Float32Array): void;
  swap64(array: BigUint64Array | BigInt64Array | Float64Array): void;
}

export interface VarIntOps {
  /** LEB128. Values are integers within ±2^53; negative ones need `zigzag`. */
  encodeAll(values: ArrayLike<number>, zigzag?: boolean): Uint8Array;
  /** Every value in `bytes`. Throws a `RangeError` for a value cut short or beyond 2^53. */
  decodeAll(bytes: Bytes, zigzag?: boolean): Float64Array;
  /** The bytes `value` takes. */
  size(value: number, zigzag?: boolean): number;
}

export interface Crc32Ops {
  /** CRC-32 as zlib and PNG compute it. `seed` is the CRC of the bytes before these. */
  compute(bytes: Bytes, seed?: number): number;
  create(): { update(bytes: Bytes): void; digest(): number };
}

export interface Crc64Ops {
  /** `"ecma"` is CRC-64/XZ (the ECMA-182 polynomial, as xz and Go compute it); `"nvme"` is CRC-64/NVME. */
  compute(bytes: Bytes, variant?: "ecma" | "nvme"): bigint;
  create(variant?: "ecma" | "nvme"): { update(bytes: Bytes): void; digest(): bigint };
}

declare const __sk_std: {
  binary: {
    BinaryReader: BinaryReaderConstructor;
    BinaryWriter: BinaryWriterConstructor;
    BitReader: BitReaderConstructor;
    BitWriter: BitWriterConstructor;
    Endian: EndianOps;
    VarInt: VarIntOps;
    CRC32: Crc32Ops;
    CRC64: Crc64Ops;
  };
};

export const BinaryReader: BinaryReaderConstructor = __sk_std.binary.BinaryReader;
export const BinaryWriter: BinaryWriterConstructor = __sk_std.binary.BinaryWriter;
export const BitReader: BitReaderConstructor = __sk_std.binary.BitReader;
export const BitWriter: BitWriterConstructor = __sk_std.binary.BitWriter;
export const Endian: EndianOps = __sk_std.binary.Endian;
export const VarInt: VarIntOps = __sk_std.binary.VarInt;
export const CRC32: Crc32Ops = __sk_std.binary.CRC32;
export const CRC64: Crc64Ops = __sk_std.binary.CRC64;
