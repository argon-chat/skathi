// `skathi:memory`: native memory JS uses without a copy and frees when it says so. The host
// installs `NativeBuffer` and `Arena`; `MemoryView` is JS because its per-element `get` and `set`
// cost less over a `DataView` than a call into the host would.

export type Element = "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "f32" | "f64";

/** Memory the host allocated, seen through `buffer` without a copy, until `free()` or collection. */
export interface NativeBuffer {
  /** 0 once freed. */
  readonly byteLength: number;
  /** An `ArrayBuffer` over the memory. Detached, and so empty, once the buffer is freed. */
  readonly buffer: ArrayBuffer;
  readonly freed: boolean;
  /** Detaches `buffer` and every view of it, and gives the memory back. A second call does nothing. */
  free(): void;
}

export interface NativeBufferStatics {
  readonly prototype: NativeBuffer;
  /** Zeroed unless `zeroed: false`; `alignment` is a power of two up to 65536, 16 by default. */
  alloc(byteLength: number, options?: { zeroed?: boolean; alignment?: number }): NativeBuffer;
}

/** A bump allocator for buffers that live until the next `reset()`, such as one frame's. */
export interface Arena {
  /** A zeroed buffer of `byteLength` bytes at a multiple of `alignment` (16 by default). */
  alloc(byteLength: number, alignment?: number): NativeBuffer;
  /** Bytes handed out since the last `reset()`. */
  readonly used: number;
  /** Frees every buffer the arena handed out and starts over in the same memory. */
  reset(): void;
}

export interface ArenaConstructor {
  readonly prototype: Arena;
  /** `chunkSize` is the memory taken at a time, 64 KiB by default; a bigger buffer gets a chunk of its own. */
  new (options?: { chunkSize?: number }): Arena;
}

export interface MemoryLayout {
  readonly type: Element;
  /** Where element 0 starts, 0 by default. */
  readonly byteOffset?: number;
  /** Bytes from one element to the next, the element's size by default. */
  readonly stride?: number;
  readonly length: number;
}

declare const __sk_std: {
  memory: {
    NativeBuffer: NativeBufferStatics;
    Arena: ArenaConstructor;
    copyInto(
      view: DataView,
      type: Element,
      stride: number,
      length: number,
      values: ArrayLike<number>,
      start: number | undefined,
    ): void;
  };
};

export const NativeBuffer: NativeBufferStatics = __sk_std.memory.NativeBuffer;
export const Arena: ArenaConstructor = __sk_std.memory.Arena;

const SIZES: Readonly<Record<Element, number>> = { u8: 1, i8: 1, u16: 2, i16: 2, u32: 4, i32: 4, f32: 4, f64: 8 };
const KINDS: readonly Element[] = ["u8", "i8", "u16", "i16", "u32", "i32", "f32", "f64"];

function whole(value: number | undefined, fallback: number, what: string): number {
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value < 0)
    throw new RangeError(`MemoryView: ${what} must be a non-negative integer`);
  return value;
}

/** Elements of one type at a fixed stride, such as one attribute of interleaved vertices. Little-endian. */
export class MemoryView {
  readonly length: number;
  readonly #view: DataView;
  readonly #type: Element;
  readonly #kind: number;
  readonly #stride: number;

  constructor(source: NativeBuffer | ArrayBuffer, layout: MemoryLayout) {
    let buffer: ArrayBuffer;
    if (source instanceof ArrayBuffer) {
      buffer = source;
    } else if (NativeBuffer.prototype.isPrototypeOf(source)) {
      if (source.freed) throw new DOMException("MemoryView: the NativeBuffer is freed", "InvalidStateError");
      buffer = source.buffer;
    } else {
      throw new TypeError("MemoryView: the source must be a NativeBuffer or an ArrayBuffer");
    }
    const kind = typeof layout === "object" && layout !== null ? KINDS.indexOf(layout.type) : -1;
    if (kind < 0) throw new TypeError(`MemoryView: type must be one of ${KINDS.map((k) => `"${k}"`).join(", ")}`);
    if (layout.length === undefined) throw new TypeError("MemoryView: the layout needs a length");
    const size = SIZES[layout.type];
    const offset = whole(layout.byteOffset, 0, "byteOffset");
    const stride = whole(layout.stride, size, "stride");
    const length = whole(layout.length, 0, "length");
    if (stride < size)
      throw new RangeError(`MemoryView: stride ${stride} is less than the ${size} bytes of a ${layout.type}`);
    const span = length === 0 ? 0 : (length - 1) * stride + size;
    if (offset + span > buffer.byteLength) {
      throw new RangeError(
        `MemoryView: ${length} elements from byte ${offset} need ${offset + span} bytes; the buffer has ${buffer.byteLength}`,
      );
    }
    this.length = length;
    this.#view = new DataView(buffer, offset, span);
    this.#type = layout.type;
    this.#kind = kind;
    this.#stride = stride;
  }

  /** Throws a `RangeError` outside `0 … length - 1`, and a `TypeError` once the memory is freed. */
  get(index: number): number {
    if (index >>> 0 !== index || index >= this.length) throw new RangeError(`MemoryView.get: no element ${index}`);
    const at = index * this.#stride;
    const view = this.#view;
    switch (this.#kind) {
      case 0:
        return view.getUint8(at);
      case 1:
        return view.getInt8(at);
      case 2:
        return view.getUint16(at, true);
      case 3:
        return view.getInt16(at, true);
      case 4:
        return view.getUint32(at, true);
      case 5:
        return view.getInt32(at, true);
      case 6:
        return view.getFloat32(at, true);
      default:
        return view.getFloat64(at, true);
    }
  }

  /** Converts `value` as a typed array of the type would. */
  set(index: number, value: number): void {
    if (index >>> 0 !== index || index >= this.length) throw new RangeError(`MemoryView.set: no element ${index}`);
    const at = index * this.#stride;
    const view = this.#view;
    switch (this.#kind) {
      case 0:
        return view.setUint8(at, value);
      case 1:
        return view.setInt8(at, value);
      case 2:
        return view.setUint16(at, value, true);
      case 3:
        return view.setInt16(at, value, true);
      case 4:
        return view.setUint32(at, value, true);
      case 5:
        return view.setInt32(at, value, true);
      case 6:
        return view.setFloat32(at, value, true);
      default:
        return view.setFloat64(at, value, true);
    }
  }

  /** Writes `values` to the elements from `start` on, in one call into the host. */
  copyFrom(values: ArrayLike<number>, start?: number): void {
    __sk_std.memory.copyInto(this.#view, this.#type, this.#stride, this.length, values, start);
  }
}
