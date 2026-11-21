import { OK, importLine, section } from "./shared";

const SAME_EVERYWHERE = "The module is the same on every platform.";
const BATCH =
  "Each method does a batch of work in one call, because a call into the host costs more than a scalar read does.";
const BIT_ORDER =
  'The order is `"msb"` by default: a byte\'s high bit comes first, as in most network formats and in JPEG. `"lsb"` takes the low bit first, as DEFLATE does. Pass the same order to the reader and the writer.';
const NOT_BYTES =
  "A `bytes` argument that is not an `ArrayBuffer`, a typed array or a `DataView` throws a `TypeError`.";

export const binary = section("skathi:binary", [
  {
    slug: "binary-reader",
    name: "BinaryReader",
    kind: "class",
    summary: "A cursor that reads typed arrays, structs, byte ranges, text and LEB128 values from a buffer in place.",
    syntax: `${importLine("skathi:binary", "BinaryReader")}

new BinaryReader(source: BufferSource, options?: { endian?: "little" | "big" })

reader.position: number
readonly reader.remaining: number
readonly reader.view: DataView

reader.readArray<T extends TypedArray>(type: new (length: number) => T, count: number): T
reader.readStruct<L extends StructLayout>(layout: L): StructOf<L>
reader.bytes(n: number): Uint8Array
reader.string(n: number, encoding?: string): string
reader.varUints(count: number): Uint32Array`,
    params: [
      {
        name: "source",
        type: "BufferSource",
        description: "An `ArrayBuffer`, a typed array or a `DataView`. It is read in place, without a copy.",
      },
      {
        name: "options.endian",
        type: '"little" | "big"',
        description: "The byte order of `readArray` and `readStruct`.",
        optional: true,
        default: '"little"',
      },
    ],
    values: [
      { value: "position", status: OK, note: "The byte the next read starts at. Assignable." },
      { value: "remaining", status: OK, note: "Bytes from `position` to the end." },
      { value: "view", status: OK, note: "A `DataView` of the whole source, whatever the position." },
      {
        value: "readArray(Type, count)",
        status: OK,
        note: "`count` elements as a new `Type` (`Float32Array`, `Uint16Array`, …), read in the reader's byte order.",
      },
      {
        value: "readStruct(layout)",
        status: OK,
        note: "An object with the layout's fields, read one after another with no padding.",
      },
      { value: "bytes(n)", status: OK, note: "The next `n` bytes as a `Uint8Array` over the source, not a copy." },
      {
        value: "string(n, encoding?)",
        status: OK,
        note: 'The next `n` bytes as text. `encoding` is any `TextDecoder` label, `"utf-8"` by default.',
      },
      {
        value: "varUints(count)",
        status: OK,
        note: "`count` unsigned LEB128 values of up to 32 bits, as a `Uint32Array`.",
      },
    ],
    notes: [
      `${BATCH} For one scalar at a time, read it from \`reader.view\`, a standard \`DataView\`, at \`reader.position\`, and move \`position\` yourself.`,
      "The reader does not copy the source. A reader over a typed array or a `DataView` covers that view's bytes only, and `position` counts from the view's start.",
      "`readArray` returns a new typed array that owns its data. It takes any typed array constructor, the `BigInt64Array`, `BigUint64Array` and `Float16Array` ones included.",
      "`bytes(n)` returns a view that shares the source's buffer: a write through it changes the source.",
      "`string` does not remove a byte order mark, and bytes that are not valid in the encoding decode to U+FFFD.",
      "`readStruct` takes a [StructLayout](/std/struct-layout/). Declare the layout `as const` and the result is typed by it.",
      "Every read moves `position` past what it read.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Read a header and the records after it",
        lang: "ts",
        code: `import { BinaryReader } from "skathi:binary";

const HEADER = { magic: "u32", version: "u16", count: "u16" } as const;

export function readSamples(file: ArrayBuffer): Float32Array {
  const reader = new BinaryReader(file);
  const header = reader.readStruct(HEADER);
  if (header.magic !== 0x534b3031) throw new Error("Not a sample file.");
  return reader.readArray(Float32Array, header.count);
}`,
      },
      {
        title: "A big-endian record with LEB128 values and text",
        lang: "ts",
        code: `import { BinaryReader } from "skathi:binary";

const bytes = new Uint8Array([0x01, 0x02, 0xe5, 0x8e, 0x26, 0x7f, 0x68, 0xc3, 0xa9, 0x6c, 0x6c, 0x6f]);
const reader = new BinaryReader(bytes, { endian: "big" });

reader.readArray(Uint16Array, 1); // Uint16Array [0x0102]
reader.varUints(2); // Uint32Array [624485, 127]
reader.string(6); // "héllo"
reader.remaining; // 0`,
      },
      {
        title: "One scalar at a time through the DataView",
        lang: "ts",
        code: `import { BinaryReader } from "skathi:binary";

export function readTag(reader: BinaryReader): number {
  const tag = reader.view.getUint8(reader.position);
  reader.position += 1;
  return tag;
}`,
      },
      {
        title: "A byte range that shares the source",
        lang: "ts",
        code: `import { BinaryReader } from "skathi:binary";

const source = new Uint8Array([9, 8, 7, 6, 5]);
const reader = new BinaryReader(source.subarray(1));
reader.position = 1;

const window = reader.bytes(2); // Uint8Array [7, 6]
window.buffer === source.buffer; // true
window.byteOffset; // 2`,
      },
    ],
    edgeCases: [
      "A read past the end throws a `RangeError` and leaves `position` where it was: `BinaryReader.bytes: 1 bytes wanted, 0 remain`.",
      "Setting `position` past the end throws a `RangeError`. The end itself is allowed.",
      "A count, a length or a position that is negative or not an integer throws a `RangeError`.",
      'A `source` that is not an `ArrayBuffer`, a typed array or a `DataView` throws a `TypeError`. So does an `endian` other than `"little"` or `"big"`.',
      "`readArray` with a `type` that is not a typed array constructor throws a `TypeError`.",
      '`readStruct` with an unknown field type throws a `TypeError` that names the field: `BinaryReader.readStruct: field "a" has the type "u24"; use u8, i8, u16, i16, u32, i32, u64, i64, f32 or f64`.',
      "`string` with a label the host does not know throws a `RangeError`.",
      "`varUints` throws a `RangeError` for a value cut short by the end, a value that does not fit 32 bits, or a `count` larger than the bytes that remain. `position` stays where it was.",
    ],
    related: ["binary-writer", "struct-layout", "bit-reader", "var-int", "endian", "encoding"],
    aliases: [
      "BinaryReader.position",
      "BinaryReader.remaining",
      "BinaryReader.view",
      "BinaryReader.readArray",
      "BinaryReader.readStruct",
      "BinaryReader.bytes",
      "BinaryReader.string",
      "BinaryReader.varUints",
      "DataView",
      "byte cursor",
      "parse binary file",
      "little endian",
      "big endian",
      "LEB128",
    ],
  },
  {
    slug: "binary-writer",
    name: "BinaryWriter",
    kind: "class",
    summary:
      "A growing buffer that writes arrays of numbers, structs and raw bytes, and hands the result over as a `Uint8Array`.",
    syntax: `${importLine("skathi:binary", "BinaryWriter")}

new BinaryWriter(options?: { endian?: "little" | "big"; initialCapacity?: number })

readonly writer.length: number
readonly writer.view: DataView

writer.writeAll(values: ArrayLike<number>, type: "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "f32" | "f64"): this
writer.writeStruct<L extends StructLayout>(layout: L, value: { [K in keyof L]: number | bigint }): this
writer.bytes(source: BufferSource): this
writer.finish(): Uint8Array`,
    params: [
      {
        name: "options.endian",
        type: '"little" | "big"',
        description: "The byte order of `writeAll` and `writeStruct`.",
        optional: true,
        default: '"little"',
      },
      {
        name: "options.initialCapacity",
        type: "number",
        description: "Bytes to allocate up front.",
        optional: true,
        default: "256",
      },
    ],
    values: [
      {
        value: "writeAll(values, type)",
        status: OK,
        note: "Every value of an array or a typed array as `type`. Integers wrap as they do in a typed array.",
      },
      {
        value: "writeStruct(layout, value)",
        status: OK,
        note: "The layout's fields of `value`, one after another with no padding.",
      },
      { value: "bytes(source)", status: OK, note: "The bytes of a buffer or a view, copied in." },
      { value: "length", status: OK, note: "Bytes written so far." },
      {
        value: "view",
        status: OK,
        note: "A `DataView` of what is written, to patch a length or a checksum.",
      },
      { value: "finish()", status: OK, note: "The bytes as a `Uint8Array`, handed over without a copy." },
    ],
    notes: [
      "The writer grows as it is written to. `initialCapacity` saves the regrowth when the size is known.",
      "`writeAll`, `writeStruct` and `bytes` return the writer, so calls chain.",
      BATCH,
      "`writeAll` has no 64-bit integer type. Write `u64` and `i64` fields with `writeStruct`, or a `BigUint64Array` with `bytes`.",
      "`writeStruct` takes a [StructLayout](/std/struct-layout/). A `u64` or `i64` field takes a `bigint` or an integer `number`; a `bigint` wraps to 64 bits.",
      "An integer type drops a value's fraction and wraps it. `NaN` and the infinities are written as 0.",
      "`view` covers what was written when you took it. Take it again after writing more.",
      "After `finish()` the writer is empty and can be used again.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Write a header, the records and a checksum",
        lang: "ts",
        code: `import { BinaryReader, BinaryWriter, CRC32 } from "skathi:binary";

const HEADER = { magic: "u32", version: "u16", count: "u16" } as const;

const writer = new BinaryWriter();
writer.writeStruct(HEADER, { magic: 0x534b3031, version: 1, count: 3 });
writer.writeAll([1.5, 2.5, 3.5], "f32");
const bytes = writer.finish();

const reader = new BinaryReader(bytes);
const header = reader.readStruct(HEADER);
const values = reader.readArray(Float32Array, header.count); // Float32Array [1.5, 2.5, 3.5]
const checksum = CRC32.compute(bytes);`,
      },
      {
        title: "Patch the total size into the header",
        lang: "ts",
        code: `import { BinaryWriter } from "skathi:binary";

const HEAD = { magic: "u32", size: "u16", id: "i64" } as const;

const writer = new BinaryWriter({ endian: "big", initialCapacity: 4 });
writer.writeStruct(HEAD, { magic: 0xcafebabe, size: 0, id: -2n });
writer
  .writeAll([1.5, -2], "f32")
  .writeAll(new Uint16Array([0x0102]), "u16")
  .bytes(new Uint8Array([0xe5, 0x8e, 0x26, 0x7f]))
  .bytes(new TextEncoder().encode("héllo"));

writer.length; // 34
writer.view.setUint16(4, writer.length);
const bytes = writer.finish();
writer.length; // 0`,
      },
    ],
    edgeCases: [
      '`writeAll` with a `type` outside its list, or without one, throws a `TypeError`. `"u64"` and `"i64"` are outside the list.',
      "`writeStruct` with an unknown field type throws a `TypeError` that names the field. A `value` that is not an object throws a `TypeError`.",
      "`writeStruct` with a `u64` or `i64` field given a `number` that is not an integer, or one beyond ±2^53, throws a `RangeError`.",
      "`bytes` with a `source` that is not an `ArrayBuffer`, a typed array or a `DataView` throws a `TypeError`.",
      'An `endian` other than `"little"` or `"big"` throws a `TypeError`. An `initialCapacity` that is negative or not an integer throws a `RangeError`.',
    ],
    related: ["binary-reader", "struct-layout", "bit-writer", "var-int", "crc32"],
    aliases: [
      "BinaryWriter.writeAll",
      "BinaryWriter.writeStruct",
      "BinaryWriter.bytes",
      "BinaryWriter.finish",
      "BinaryWriter.view",
      "BinaryWriter.length",
      "initialCapacity",
      "byte buffer",
      "serialize",
      "write binary file",
      "little endian",
      "big endian",
    ],
  },
  {
    slug: "struct-layout",
    name: "StructLayout",
    kind: "type",
    summary:
      "An object of field names and types that tells `readStruct` and `writeStruct` which scalars follow each other.",
    syntax: `import type { FieldType, StructLayout, StructOf } from "skathi:binary";

type FieldType = "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "u64" | "i64" | "f32" | "f64"
type StructLayout = Record<string, FieldType>
type StructOf<L extends StructLayout> = {
  [K in keyof L]: L[K] extends "u64" | "i64" ? bigint : number
}`,
    values: [
      { value: "u8", status: OK, note: "1 byte, unsigned. A `number`." },
      { value: "i8", status: OK, note: "1 byte, signed. A `number`." },
      { value: "u16", status: OK, note: "2 bytes, unsigned. A `number`." },
      { value: "i16", status: OK, note: "2 bytes, signed. A `number`." },
      { value: "u32", status: OK, note: "4 bytes, unsigned. A `number`." },
      { value: "i32", status: OK, note: "4 bytes, signed. A `number`." },
      { value: "u64", status: OK, note: "8 bytes, unsigned. A `bigint`." },
      { value: "i64", status: OK, note: "8 bytes, signed. A `bigint`." },
      { value: "f32", status: OK, note: "4 bytes, IEEE 754 single precision. A `number`." },
      { value: "f64", status: OK, note: "8 bytes, IEEE 754 double precision. A `number`." },
    ],
    notes: [
      "Fields follow each other with no padding, in the order the object lists them.",
      "`u64` and `i64` are `bigint`, the rest are `number`.",
      "Declare a layout `as const` and the result of `readStruct` is typed by it.",
      "Every field uses the byte order of the reader or the writer.",
      "A layout is a plain object. Keep it in a constant and pass the same one to the reader and the writer.",
    ],
    examples: [
      {
        title: "One layout for both directions",
        lang: "ts",
        code: `import { BinaryReader, BinaryWriter } from "skathi:binary";

const HEAD = { magic: "u32", size: "u16", id: "i64" } as const;

const bytes = new BinaryWriter({ endian: "big" }).writeStruct(HEAD, { magic: 0xcafebabe, size: 34, id: -2n }).finish();
bytes.length; // 14

const head = new BinaryReader(bytes, { endian: "big" }).readStruct(HEAD);
head.magic; // 3405691582 (0xcafebabe)
head.size; // 34
head.id; // -2n`,
      },
    ],
    edgeCases: [
      'A field type outside the list throws a `TypeError` that names the field: `BinaryReader.readStruct: field "a" has the type "u24"; use u8, i8, u16, i16, u32, i32, u64, i64, f32 or f64`.',
      "A layout that is not an object throws a `TypeError`.",
    ],
    related: ["binary-reader", "binary-writer", "integer-types-64"],
    aliases: ["FieldType", "StructOf", "readStruct", "writeStruct", "struct", "record layout", "packed struct"],
  },
  {
    slug: "bit-reader",
    name: "BitReader",
    kind: "class",
    summary: "Reads values of 1 to 32 bits from a `Uint8Array`, high bit first or low bit first.",
    syntax: `${importLine("skathi:binary", "BitReader")}

new BitReader(source: Uint8Array, order?: "msb" | "lsb")

reader.read(bits: number): number
reader.readMany(bits: number, count: number): Uint32Array
reader.align(): void`,
    params: [
      { name: "source", type: "Uint8Array", description: "The bytes to read. They are read in place." },
      {
        name: "order",
        type: '"msb" | "lsb"',
        description: "Which bit of a byte comes first.",
        optional: true,
        default: '"msb"',
      },
    ],
    notes: [
      "A value is 1 to 32 bits. `read(bits)` returns the next `bits` bits as an unsigned number.",
      "`readMany(bits, count)` returns `count` values of `bits` bits each as a `Uint32Array`, in one call.",
      BIT_ORDER,
      "`align()` skips to the next byte boundary. At a boundary it does nothing.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Read packed fields",
        lang: "ts",
        code: `import { BitReader, BitWriter } from "skathi:binary";

const packed = new BitWriter().write(5, 3).writeMany([1, 2, 3], 2).finish(); // Uint8Array [173, 128]

const bits = new BitReader(packed);
bits.read(3); // 5
bits.readMany(2, 3); // Uint32Array [1, 2, 3]
bits.align();`,
      },
      {
        title: "DEFLATE's bit order",
        lang: "ts",
        code: `import { BitReader } from "skathi:binary";

const bits = new BitReader(new Uint8Array([205, 1]), "lsb");
bits.read(3); // 5
bits.readMany(2, 3); // Uint32Array [1, 2, 3]`,
      },
    ],
    edgeCases: [
      "A read past the end throws a `RangeError`: `BitReader.read: 1 bits wanted, 0 remain`.",
      "A `bits` outside 1 to 32, or not an integer, throws a `RangeError`.",
      'A `source` that is not a typed array throws a `TypeError`. So does an `order` other than `"msb"` or `"lsb"`.',
    ],
    related: ["bit-writer", "binary-reader"],
    aliases: [
      "BitReader.read",
      "BitReader.readMany",
      "BitReader.align",
      "bit stream",
      "bitstream",
      "msb",
      "lsb",
      "DEFLATE",
      "bit packing",
    ],
  },
  {
    slug: "bit-writer",
    name: "BitWriter",
    kind: "class",
    summary: "Packs values of 1 to 32 bits into bytes, high bit first or low bit first.",
    syntax: `${importLine("skathi:binary", "BitWriter")}

new BitWriter(order?: "msb" | "lsb")

writer.write(value: number, bits: number): this
writer.writeMany(values: ArrayLike<number>, bits: number): this
writer.finish(): Uint8Array`,
    params: [
      {
        name: "order",
        type: '"msb" | "lsb"',
        description: "Which bit of a byte is filled first.",
        optional: true,
        default: '"msb"',
      },
    ],
    notes: [
      "A value is 1 to 32 bits. `write(value, bits)` writes the low `bits` bits of `value`; higher bits are dropped.",
      "`writeMany(values, bits)` writes every value of an array or a typed array with the same width, in one call.",
      BIT_ORDER,
      "`write` and `writeMany` return the writer, so calls chain.",
      "`finish()` returns the bytes as a new `Uint8Array` and pads the last byte with zero bits. The writer is empty again and can be reused.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Pack a 3-bit tag and three 2-bit values",
        lang: "ts",
        code: `import { BitWriter } from "skathi:binary";

new BitWriter().write(5, 3).writeMany([1, 2, 3], 2).finish();
// Uint8Array [173, 128]: 101 01 10 11, then 1 and seven zero bits

new BitWriter("lsb").write(5, 3).writeMany([1, 2, 3], 2).finish();
// Uint8Array [205, 1]`,
      },
    ],
    edgeCases: [
      "A `bits` outside 1 to 32, or not an integer, throws a `RangeError`.",
      'An `order` other than `"msb"` or `"lsb"` throws a `TypeError`.',
    ],
    related: ["bit-reader", "binary-writer"],
    aliases: [
      "BitWriter.write",
      "BitWriter.writeMany",
      "BitWriter.finish",
      "bit stream",
      "bitstream",
      "msb",
      "lsb",
      "bit packing",
    ],
  },
  {
    slug: "endian",
    name: "Endian",
    kind: "namespace",
    summary: "Tells this machine's byte order and reverses the bytes of every element of a typed array.",
    syntax: `${importLine("skathi:binary", "Endian")}

readonly Endian.native: "little" | "big"
Endian.swap16(array: Uint16Array | Int16Array | Float16Array): void
Endian.swap32(array: Uint32Array | Int32Array | Float32Array): void
Endian.swap64(array: BigUint64Array | BigInt64Array | Float64Array): void`,
    notes: [
      '`Endian.native` is `"little"` or `"big"`: this machine\'s byte order, which is the order a typed array stores its elements in.',
      "`swap16`, `swap32` and `swap64` reverse the bytes of every element in place. They return nothing and allocate nothing.",
      "A typed array over a subrange of a buffer swaps that subrange only.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Use big-endian samples on a little-endian machine",
        lang: "ts",
        code: `import { Endian } from "skathi:binary";

export function fromBigEndian(samples: Int16Array): Int16Array {
  if (Endian.native === "little") Endian.swap16(samples);
  return samples;
}

const word = new Uint16Array([0x1234]);
Endian.swap16(word);
word[0]; // 0x3412`,
      },
    ],
    edgeCases: [
      "An array whose elements are not of the function's size throws a `TypeError`: `Endian.swap16: the array must be a typed array of 2-byte elements`. A `DataView` and an `ArrayBuffer` throw too.",
    ],
    related: ["binary-reader", "binary-writer"],
    aliases: [
      "Endian.native",
      "Endian.swap16",
      "Endian.swap32",
      "Endian.swap64",
      "byte order",
      "byte swap",
      "bswap",
      "endianness",
      "little endian",
      "big endian",
    ],
  },
  {
    slug: "var-int",
    name: "VarInt",
    kind: "namespace",
    summary:
      "Encodes and decodes arrays of integers as LEB128 variable-length bytes, with optional zigzag for signed values.",
    syntax: `${importLine("skathi:binary", "VarInt")}

VarInt.encodeAll(values: ArrayLike<number>, zigzag?: boolean): Uint8Array
VarInt.decodeAll(bytes: BufferSource, zigzag?: boolean): Float64Array
VarInt.size(value: number, zigzag?: boolean): number`,
    params: [
      {
        name: "values",
        type: "ArrayLike<number>",
        description: "An array or a typed array of integers within ±2^53.",
      },
      {
        name: "zigzag",
        type: "boolean",
        description:
          "Map signed integers to unsigned ones first (0, -1, 1, -2 become 0, 1, 2, 3), as Protocol Buffers' `sint` types do. Needed for negative values.",
        optional: true,
        default: "false",
      },
    ],
    notes: [
      "`encodeAll` returns the LEB128 bytes of every value, one after another: seven bits a byte, low bits first, the high bit set on every byte but a value's last.",
      "`decodeAll` reads every value in `bytes` to the end and returns them as a `Float64Array`.",
      "`size` returns the bytes one value takes: 1 for 0 to 127, 2 up to 16383, and so on.",
      "Pass the same `zigzag` to `encodeAll`, `decodeAll` and `size`.",
      "To read a known number of values from the middle of a buffer, use [BinaryReader](/std/binary-reader/)'s `varUints(count)`.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Round-trip signed values",
        lang: "ts",
        code: `import { VarInt } from "skathi:binary";

const bytes = VarInt.encodeAll([0, 300, -1, 2 ** 40], true);
VarInt.decodeAll(bytes, true); // Float64Array [0, 300, -1, 1099511627776]

VarInt.encodeAll([300]); // Uint8Array [0xac, 0x02]
VarInt.size(300); // 2
VarInt.decodeAll(new Uint8Array([0xe5, 0x8e, 0x26, 0x7f])); // Float64Array [624485, 127]`,
      },
    ],
    edgeCases: [
      "`encodeAll` and `size` throw a `RangeError` for a value that is not an integer, is beyond ±2^53, or is negative without `zigzag`.",
      "`decodeAll` throws a `RangeError` for a value cut short by the end of `bytes`, a value longer than 64 bits, or a value beyond ±2^53. The message gives the byte the value starts at.",
      NOT_BYTES,
    ],
    related: ["binary-reader", "binary-writer"],
    aliases: [
      "VarInt.encodeAll",
      "VarInt.decodeAll",
      "VarInt.size",
      "LEB128",
      "varint",
      "zigzag",
      "variable-length integer",
      "protobuf",
    ],
  },
  {
    slug: "crc32",
    name: "CRC32",
    kind: "namespace",
    summary: "Computes the CRC-32 checksum that zlib and PNG use, in one call or over data that arrives in pieces.",
    syntax: `${importLine("skathi:binary", "CRC32")}

CRC32.compute(bytes: BufferSource, seed?: number): number
CRC32.create(): { update(bytes: BufferSource): void; digest(): number }`,
    params: [
      { name: "bytes", type: "BufferSource", description: "An `ArrayBuffer`, a typed array or a `DataView`." },
      {
        name: "seed",
        type: "number",
        description: "The CRC of the bytes before these, to continue a checksum.",
        optional: true,
        default: "0",
      },
    ],
    returns: "The checksum as an unsigned 32-bit `number`.",
    notes: [
      "`CRC32.compute(bytes)` is CRC-32 as zlib and PNG compute it. The check value of the ASCII text `123456789` is `0xcbf43926`.",
      "`seed` chains calls: `CRC32.compute(b, CRC32.compute(a))` is the CRC of `a` followed by `b`.",
      "`CRC32.create()` returns an object with `update(bytes)` and `digest()`, for data in pieces. `digest()` returns the CRC of everything passed to `update` so far.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "One call, chained calls and a stream",
        lang: "ts",
        code: `import { CRC32 } from "skathi:binary";

const utf8 = (text: string) => new TextEncoder().encode(text);

CRC32.compute(utf8("123456789")); // 0xcbf43926
CRC32.compute(utf8("56789"), CRC32.compute(utf8("1234"))); // 0xcbf43926

const stream = CRC32.create();
stream.update(utf8("1234"));
stream.update(utf8("56789"));
stream.digest(); // 0xcbf43926`,
      },
      {
        title: "Checksum data that arrives in chunks",
        lang: "ts",
        code: `import { CRC32 } from "skathi:binary";

export async function checksum(chunks: AsyncIterable<Uint8Array>): Promise<number> {
  const crc = CRC32.create();
  for await (const chunk of chunks) crc.update(chunk);
  return crc.digest();
}`,
      },
    ],
    edgeCases: [NOT_BYTES],
    related: ["crc64", "binary-writer"],
    aliases: [
      "CRC32.compute",
      "CRC32.create",
      "update",
      "digest",
      "checksum",
      "crc",
      "CRC-32",
      "zlib",
      "PNG",
      "cyclic redundancy check",
    ],
  },
  {
    slug: "crc64",
    name: "CRC64",
    kind: "namespace",
    summary: "Computes a CRC-64 checksum as a `bigint`, in the XZ or the NVMe variant.",
    syntax: `${importLine("skathi:binary", "CRC64")}

CRC64.compute(bytes: BufferSource, variant?: "ecma" | "nvme"): bigint
CRC64.create(variant?: "ecma" | "nvme"): { update(bytes: BufferSource): void; digest(): bigint }`,
    params: [
      { name: "bytes", type: "BufferSource", description: "An `ArrayBuffer`, a typed array or a `DataView`." },
      {
        name: "variant",
        type: '"ecma" | "nvme"',
        description: "Which CRC-64 to compute.",
        optional: true,
        default: '"ecma"',
      },
    ],
    returns: "The checksum as an unsigned 64-bit `bigint`.",
    values: [
      {
        value: '"ecma"',
        status: OK,
        note: "CRC-64/XZ: the ECMA-182 polynomial, as xz and Go compute it. The check value of `123456789` is `0x995dc9bbdf1939fa`.",
      },
      { value: '"nvme"', status: OK, note: "CRC-64/NVME. The check value of `123456789` is `0xae8b14860a799888`." },
    ],
    notes: [
      "`CRC64.create(variant?)` returns an object with `update(bytes)` and `digest()`, for data in pieces. `digest()` returns the CRC of everything passed to `update` so far.",
      "`compute` has no seed argument. Use `create()` to continue a checksum.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Both variants",
        lang: "ts",
        code: `import { CRC64 } from "skathi:binary";

const bytes = new TextEncoder().encode("123456789");

CRC64.compute(bytes).toString(16); // "995dc9bbdf1939fa"
CRC64.compute(bytes, "nvme").toString(16); // "ae8b14860a799888"

const stream = CRC64.create("nvme");
stream.update(bytes.subarray(0, 4));
stream.update(bytes.subarray(4));
stream.digest().toString(16); // "ae8b14860a799888"`,
      },
    ],
    edgeCases: ['A `variant` other than `"ecma"` or `"nvme"` throws a `TypeError`.', NOT_BYTES],
    related: ["crc32", "integer-types-64"],
    aliases: [
      "CRC64.compute",
      "CRC64.create",
      "update",
      "digest",
      "checksum",
      "crc",
      "CRC-64",
      "CRC-64/XZ",
      "CRC-64/NVME",
      "ECMA-182",
      "xz",
    ],
  },
]);
