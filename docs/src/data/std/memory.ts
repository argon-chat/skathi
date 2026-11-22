import { OK, importLine, section } from "./shared";

const SAME_EVERYWHERE = "The module is the same on every platform.";

export const memory = section("skathi:memory", [
  {
    slug: "native-buffer",
    name: "NativeBuffer",
    kind: "class",
    summary: "Memory the host allocates, seen from JS as an `ArrayBuffer` without a copy, and freed when you say so.",
    syntax: `${importLine("skathi:memory", "NativeBuffer")}

static NativeBuffer.alloc(byteLength: number, options?: { zeroed?: boolean; alignment?: number }): NativeBuffer

readonly buffer.byteLength: number
readonly buffer.buffer: ArrayBuffer
readonly buffer.freed: boolean
buffer.free(): void`,
    params: [
      { name: "byteLength", type: "number", description: "The size in bytes. 0 is allowed." },
      {
        name: "options.zeroed",
        type: "boolean",
        description: "`false` skips clearing the memory, which then holds whatever was there before.",
        optional: true,
        default: "true",
      },
      {
        name: "options.alignment",
        type: "number",
        description: "The address is a multiple of it: a power of two up to 65536.",
        optional: true,
        default: "16",
      },
    ],
    values: [
      { value: "byteLength", status: OK, note: "The size in bytes; 0 once freed." },
      {
        value: "buffer",
        status: OK,
        note: "An `ArrayBuffer` over the memory. Make typed arrays and `DataView`s of it as of any other.",
      },
      { value: "freed", status: OK, note: "`true` after `free()`, or once `buffer` was transferred away." },
      {
        value: "free()",
        status: OK,
        note: "Detaches `buffer`: every typed array and `DataView` over it reads as empty from then on. A second call does nothing.",
      },
    ],
    notes: [
      "The memory lives until `free()` or until the buffer is collected, whichever comes first. The collector is told the size, but it runs when it decides to: free a large buffer as soon as you are done with it.",
      "After `free()` no JS view can reach the memory: views of a detached buffer have length 0, a read or a write through a `DataView` throws a `TypeError`, and a write through a typed array goes nowhere.",
      "Passing a freed buffer to `BinaryReader`, an FFI call, `NativePointer.of()` or a `MemoryView` throws `InvalidStateError`. A `BinaryReader` made before throws it from its next read.",
      "An FFI call takes a `NativeBuffer` for a `buffer` parameter. A callback that frees it during the call does not free it under the library: the memory goes back once the call returns.",
      "A nonblocking FFI call works on the memory in place, without the copy a typed array gets. Until it settles, `buffer` is detached, as after a transfer, and the buffer cannot be passed anywhere else; afterwards `buffer` is a new `ArrayBuffer` over the same memory. Make views again after the call.",
      "`buffer.transfer()` moves the memory to the new `ArrayBuffer`, and the `NativeBuffer` counts as freed.",
      "A native module returns and takes them as `skathi::NativeBuffer`, without a copy: a buffer made in Rust is this class, and a task has the buffer the way a nonblocking FFI call does. See [native module values](/embedding/native-modules/values/#nativebuffer).",
      "`new NativeBuffer()` throws a `TypeError`: buffers come from `NativeBuffer.alloc()` or an [`Arena`](/std/arena/).",
      SAME_EVERYWHERE,
    ],
    edgeCases: [
      "`alloc()` throws a `RangeError` for a negative or fractional size, an alignment that is not a power of two up to 65536, or memory that cannot be had.",
    ],
    examples: [
      {
        title: "Fill a buffer, hand it to C, give the memory back",
        lang: "ts",
        code: `import { dlopen } from "skathi:ffi";
import { NativeBuffer } from "skathi:memory";

const codec = dlopen("codec", { encode: { args: ["buffer", "usize"], returns: "i32" } });

const frame = NativeBuffer.alloc(1920 * 1080 * 4);
try {
  new Uint8Array(frame.buffer).fill(255);
  codec.symbols.encode(frame, frame.byteLength);
} finally {
  frame.free();
}`,
      },
      {
        title: "Views die with the buffer",
        lang: "ts",
        code: `import { NativeBuffer } from "skathi:memory";

const scratch = NativeBuffer.alloc(16);
const bytes = new Uint8Array(scratch.buffer);
bytes[0] = 7;
scratch.free();

bytes.length; // 0
scratch.freed; // true`,
      },
    ],
    related: ["arena", "memory-view", "native-pointer", "binary-reader"],
  },
  {
    slug: "arena",
    name: "Arena",
    kind: "class",
    summary: "Hands out `NativeBuffer`s from larger chunks, and frees all of them in one `reset()`.",
    syntax: `${importLine("skathi:memory", "Arena")}

new Arena(options?: { chunkSize?: number })

arena.alloc(byteLength: number, alignment?: number): NativeBuffer
readonly arena.used: number
arena.reset(): void`,
    params: [
      {
        name: "options.chunkSize",
        type: "number",
        description: "The memory the arena takes at a time. A buffer bigger than that gets a chunk of its own.",
        optional: true,
        default: "65536",
      },
    ],
    values: [
      {
        value: "alloc(byteLength, alignment?)",
        status: OK,
        note: "A zeroed `NativeBuffer` at an address that is a multiple of `alignment`, 16 by default.",
      },
      { value: "used", status: OK, note: "The bytes handed out since the last `reset()`." },
      {
        value: "reset()",
        status: OK,
        note: "Frees every buffer the arena handed out and starts over in the same chunks.",
      },
    ],
    notes: [
      "An arena is for buffers that live together and die together, such as those of one frame: uploads to the GPU, arguments for a C library, a decoder's scratch space.",
      "Each `alloc()` takes the next bytes of the current chunk, so it costs far less than `NativeBuffer.alloc()`. Freeing one buffer with `free()` detaches it, but its bytes are not reused before `reset()`.",
      "A chunk that a [`NativePointer`](/std/native-pointer/) or a call in progress still holds is not handed out again after `reset()`: the arena lets it go and takes a new one.",
      "The chunks stay with the arena between resets and go back when the arena is collected.",
      SAME_EVERYWHERE,
    ],
    edgeCases: [
      "`alloc()` throws a `RangeError` for a negative or fractional size or an alignment that is not a power of two up to 65536.",
    ],
    examples: [
      {
        title: "Buffers for one frame",
        lang: "ts",
        code: `import { Arena } from "skathi:memory";

const frame = new Arena({ chunkSize: 1 << 20 });

export function draw(device: GPUDevice, vertices: GPUBuffer, points: Float32Array) {
  const staging = frame.alloc(points.byteLength);
  new Float32Array(staging.buffer).set(points);
  device.queue.writeBuffer(vertices, 0, staging.buffer);
  frame.reset();
}`,
      },
    ],
    related: ["native-buffer"],
  },
  {
    slug: "memory-view",
    name: "MemoryView",
    kind: "class",
    summary: "Elements of one number type at a fixed stride, such as one attribute of interleaved vertices.",
    syntax: `${importLine("skathi:memory", "MemoryView")}

new MemoryView(source: NativeBuffer | ArrayBuffer, layout: MemoryLayout)

readonly view.length: number
view.get(index: number): number
view.set(index: number, value: number): void
view.copyFrom(values: ArrayLike<number>, start?: number): void`,
    params: [
      { name: "source", type: "NativeBuffer | ArrayBuffer", description: "The memory the view reads and writes." },
      {
        name: "layout",
        type: "MemoryLayout",
        description:
          "The element type, where element 0 starts, the stride and the number of elements. See [MemoryLayout](/std/memory-layout/).",
      },
    ],
    values: [
      { value: "length", status: OK, note: "The number of elements." },
      { value: "get(index)", status: OK, note: "Element `index`." },
      {
        value: "set(index, value)",
        status: OK,
        note: "Writes element `index`, converted as a typed array of the type converts it: integers wrap, `f32` rounds.",
      },
      {
        value: "copyFrom(values, start?)",
        status: OK,
        note: "Writes `values` to the elements from `start` (0 by default) on, in one call into the host.",
      },
    ],
    notes: [
      "A typed array has no stride: it reads consecutive elements. A `MemoryView` steps `stride` bytes from one element to the next, which is how one field of an array of structs is laid out.",
      "Elements are little-endian, as the memory of every platform Skathi runs on is.",
      "`get()` and `set()` run in JS over a `DataView`: one element through the host would cost more than the read. `copyFrom()` is one host call however many values there are; use it for more than a few.",
      "For whole WGSL structs with their alignment rules, use the writers of [`skathi:gpu`](/std/struct/).",
      SAME_EVERYWHERE,
    ],
    edgeCases: [
      "The constructor throws a `TypeError` for a source that is not a `NativeBuffer` or an `ArrayBuffer`, or an unknown type; a `RangeError` for a stride smaller than the element or elements past the end of the source; and `InvalidStateError` for a freed `NativeBuffer`.",
      "`get()` and `set()` throw a `RangeError` for an index outside `0 … length - 1`. Once the memory is freed, both throw a `TypeError`.",
      "`copyFrom()` throws a `RangeError` when the values do not fit from `start`, and `InvalidStateError` once the memory is freed, even when converting a value is what freed it.",
    ],
    examples: [
      {
        title: "The texture coordinates of interleaved vertices",
        lang: "ts",
        code: `import { MemoryView, NativeBuffer } from "skathi:memory";

// { pos: vec3f, uv: vec2f }: 20 bytes a vertex, uv at byte 12.
const vertices = NativeBuffer.alloc(20 * 3);
const u = new MemoryView(vertices, { type: "f32", byteOffset: 12, stride: 20, length: 3 });
const v = new MemoryView(vertices, { type: "f32", byteOffset: 16, stride: 20, length: 3 });

u.copyFrom([0, 1, 0.5]);
v.copyFrom([0, 0, 1]);
v.get(2); // 1`,
      },
    ],
    related: ["memory-layout", "native-buffer"],
  },
  {
    slug: "memory-layout",
    name: "MemoryLayout",
    kind: "type",
    summary: "Where the elements of a `MemoryView` are and what type they have.",
    syntax: `import type { Element, MemoryLayout } from "skathi:memory";

type Element = "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "f32" | "f64"
interface MemoryLayout {
  type: Element
  byteOffset?: number   // default: 0
  stride?: number       // default: the size of the type
  length: number
}`,
    values: [
      { value: "u8", status: OK, note: "1 byte, unsigned." },
      { value: "i8", status: OK, note: "1 byte, signed." },
      { value: "u16", status: OK, note: "2 bytes, unsigned." },
      { value: "i16", status: OK, note: "2 bytes, signed." },
      { value: "u32", status: OK, note: "4 bytes, unsigned." },
      { value: "i32", status: OK, note: "4 bytes, signed." },
      { value: "f32", status: OK, note: "4 bytes, IEEE 754 single precision." },
      { value: "f64", status: OK, note: "8 bytes, IEEE 754 double precision." },
    ],
    notes: [
      "`byteOffset` is where element 0 starts; `stride` is the distance in bytes from one element to the next, at least the size of the type. Neither has to be aligned.",
      "The elements must fit the source: `byteOffset + (length - 1) × stride + size` bytes at most.",
    ],
    related: ["memory-view"],
  },
]);
