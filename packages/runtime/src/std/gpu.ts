// `skathi:gpu`: WGSL struct layouts and writers for them, so the offsets in JS and in the shader agree.
// TS on purpose: a layout is computed once, and `set` is a store or two per frame that the JIT inlines.

export { GpuTimer, readBuffer, readTexture } from "./gpu-device";
export type { GpuTimes, ReadTextureOptions, TexturePixels } from "./gpu-device";

export type VertexFormat =
  | "float32"
  | "float32x2"
  | "float32x3"
  | "float32x4"
  | "uint32"
  | "uint32x2"
  | "uint32x3"
  | "uint32x4"
  | "sint32"
  | "sint32x2"
  | "sint32x3"
  | "sint32x4"
  | "float16x2"
  | "float16x4"
  | NormalizedFormat;

type NormalizedFormat =
  "unorm8x2" | "unorm8x4" | "snorm8x2" | "snorm8x4" | "unorm16x2" | "unorm16x4" | "snorm16x2" | "snorm16x4";

/** Assignable to WebGPU's `GPUVertexBufferLayout`. */
export interface VertexBufferLayout {
  arrayStride: number;
  stepMode: "vertex" | "instance";
  attributes: { format: VertexFormat; offset: number; shaderLocation: number }[];
}

export type StructLayout = "uniform" | "storage" | "vertex";

type Scalar = "f32" | "u32" | "i32" | "f16";

type Node =
  | { kind: "scalar"; scalar: Scalar }
  | { kind: "vector"; scalar: Scalar; length: number }
  | { kind: "matrix"; columns: number; rows: number }
  | { kind: "atomic"; scalar: Scalar }
  | { kind: "normalized"; format: NormalizedFormat; signed: boolean; bytes: 1 | 2; length: 2 | 4 }
  | { kind: "array"; element: FieldType; count: number | undefined; stride: number };

const roundUp = (to: number, value: number) => Math.ceil(value / to) * to;
const scalarSize = (scalar: Scalar) => (scalar === "f16" ? 2 : 4);

const VERTEX: Partial<Record<string, VertexFormat>> = {
  f32: "float32",
  "vec2<f32>": "float32x2",
  "vec3<f32>": "float32x3",
  "vec4<f32>": "float32x4",
  u32: "uint32",
  "vec2<u32>": "uint32x2",
  "vec3<u32>": "uint32x3",
  "vec4<u32>": "uint32x4",
  i32: "sint32",
  "vec2<i32>": "sint32x2",
  "vec3<i32>": "sint32x3",
  "vec4<i32>": "sint32x4",
  "vec2<f16>": "float16x2",
  "vec4<f16>": "float16x4",
};

/** A WGSL type other than a struct: made by `wgsl`, never by hand. */
export class WgslType {
  readonly wgsl: string;
  readonly size: number;
  readonly align: number;
  /** The vertex format of an attribute of this type, or null when it cannot be one. */
  readonly vertexFormat: VertexFormat | null;
  readonly #node: Node;

  /** @internal */
  constructor(key: typeof KEY, text: string, size: number, align: number, node: Node) {
    if (key !== KEY) throw new TypeError("A WGSL type comes from wgsl, not from new WgslType()");
    this.wgsl = text;
    this.size = size;
    this.align = align;
    this.vertexFormat = node.kind === "normalized" ? node.format : (VERTEX[text] ?? null);
    this.#node = node;
    Object.freeze(this);
  }

  /** @internal */
  static node(type: unknown): Node | null {
    return typeof type === "object" && type !== null && #node in type ? type.#node : null;
  }
}

const KEY = Symbol("skathi:gpu");

function scalar(name: Scalar): WgslType {
  const size = scalarSize(name);
  return new WgslType(KEY, name, size, size, { kind: "scalar", scalar: name });
}

function scalarOf(type: WgslType, what: string): Scalar {
  const node = WgslType.node(type);
  if (node?.kind !== "scalar") {
    throw new TypeError(`${what} takes wgsl.f32, wgsl.u32, wgsl.i32 or wgsl.f16, not ${describe(type)}`);
  }
  return node.scalar;
}

function describe(type: unknown): string {
  const node = WgslType.node(type);
  if (node?.kind === "normalized") return `wgsl.${node.format}`;
  if (type instanceof WgslType) return type.wgsl;
  if (type instanceof Struct) return type.name;
  return typeof type === "boolean" || type === "bool" ? "a bool" : String(type);
}

function vector(length: number): (type: WgslType) => WgslType {
  const made = new Map<Scalar, WgslType>();
  return (type) => {
    const of = scalarOf(type, `wgsl.vec${length}()`);
    let vec = made.get(of);
    if (!vec) {
      const unit = scalarSize(of);
      // A vec3 is aligned as a vec4.
      const align = (length === 3 ? 4 : length) * unit;
      vec = new WgslType(KEY, `vec${length}<${of}>`, length * unit, align, { kind: "vector", scalar: of, length });
      made.set(of, vec);
    }
    return vec;
  };
}

function matrix(columns: number, rows: number): WgslType {
  const column = (rows === 3 ? 4 : rows) * 4;
  return new WgslType(KEY, `mat${columns}x${rows}<f32>`, columns * column, column, { kind: "matrix", columns, rows });
}

function normalized(format: NormalizedFormat): WgslType {
  const signed = format.startsWith("s");
  const bytes = format.includes("16") ? 2 : 1;
  const length = format.endsWith("4") ? 4 : 2;
  return new WgslType(KEY, `vec${length}<f32>`, length * bytes, bytes, {
    kind: "normalized",
    format,
    signed,
    bytes,
    length,
  });
}

const atomics = new Map<Scalar, WgslType>();

type FieldType = WgslType | Struct<Fields>;
type Fields = Record<string, FieldType>;

export const wgsl = {
  f32: scalar("f32"),
  u32: scalar("u32"),
  i32: scalar("i32"),
  /** Needs `enable f16;` in the shader and the `shader-f16` feature on the device. */
  f16: scalar("f16"),
  vec2: vector(2),
  vec3: vector(3),
  vec4: vector(4),
  mat2x2f: matrix(2, 2),
  mat3x3f: matrix(3, 3),
  mat4x4f: matrix(4, 4),
  /** Vertex attributes stored as normalized integers: the shader sees `vec4<f32>` or `vec2<f32>` in 0..1 (unorm) or -1..1 (snorm). */
  unorm8x2: normalized("unorm8x2"),
  unorm8x4: normalized("unorm8x4"),
  snorm8x2: normalized("snorm8x2"),
  snorm8x4: normalized("snorm8x4"),
  unorm16x2: normalized("unorm16x2"),
  unorm16x4: normalized("unorm16x4"),
  snorm16x2: normalized("snorm16x2"),
  snorm16x4: normalized("snorm16x4"),
  atomic(type: WgslType): WgslType {
    const of = scalarOf(type, "wgsl.atomic()");
    if (of !== "u32" && of !== "i32") throw new TypeError(`WGSL has atomic<u32> and atomic<i32>, not atomic<${of}>`);
    let atomic = atomics.get(of);
    if (!atomic) {
      atomic = new WgslType(KEY, `atomic<${of}>`, 4, 4, { kind: "atomic", scalar: of });
      atomics.set(of, atomic);
    }
    return atomic;
  },
  /** `array<T, count>`, or with no count a runtime-sized array: the last field of a storage struct. */
  array(element: FieldType, count?: number): WgslType {
    if (!(element instanceof Struct) && !WgslType.node(element)) {
      throw new TypeError(`wgsl.array() takes a WGSL type or a struct, not ${describe(element)}`);
    }
    if (count !== undefined && (!Number.isInteger(count) || count < 1)) {
      throw new RangeError(`An array has a whole number of elements, 1 or more, not ${count}`);
    }
    const inner = WgslType.node(element);
    if (inner?.kind === "normalized") {
      throw new TypeError(`${describe(element)} is a vertex attribute format and cannot be the element of an array`);
    }
    if (inner?.kind === "array" && inner.count === undefined) {
      throw new TypeError("A runtime-sized array cannot be the element of an array");
    }
    if (element instanceof Struct && element.runtimeArray) {
      throw new TypeError(`${element.name} ends in a runtime-sized array, so it cannot be the element of an array`);
    }
    const name = element instanceof Struct ? element.name : element.wgsl;
    const stride = roundUp(element.align, element.size);
    return new WgslType(
      KEY,
      count === undefined ? `array<${name}>` : `array<${name}, ${count}>`,
      (count ?? 0) * stride,
      element.align,
      { kind: "array", element, count, stride },
    );
  },
} as const;

interface Member {
  name: string;
  type: FieldType;
  offset: number;
  /** `@align` and `@size` the uniform rules add; 0 for none. */
  alignAttribute: number;
  sizeAttribute: number;
}

export type VectorLike = { x: number; y: number; z?: number; w?: number };
/** What `set` takes: a number, numbers in order, `{ x, y, z, w }`, or for a struct field an object by field name. */
export type FieldValue =
  number | ArrayLike<number> | VectorLike | { readonly [field: string]: unknown } | readonly unknown[];

export interface WriterOptions {
  /** How many structs lie one after another in the target. Default: as many as fit, or 1 in a new buffer. */
  count?: number;
  /** Where the first struct starts in the target, in bytes. */
  byteOffset?: number;
  /** For a struct that ends in a runtime-sized array: its length in a new buffer. */
  elements?: number;
}

export class Struct<F extends Fields = Fields> {
  readonly name: string;
  readonly layout: StructLayout;
  /** In bytes, padding included. With a runtime-sized array: the size with no elements. */
  readonly size: number;
  readonly align: number;
  readonly offsets: { readonly [K in keyof F]: number };
  readonly fields: F;
  /** @internal */
  readonly members: readonly Member[];
  /** @internal */
  readonly runtimeArray: Member | null;

  /** @internal */
  constructor(key: typeof KEY, name: string, fields: F, layout: StructLayout) {
    if (key !== KEY) throw new TypeError("A struct comes from struct(), not from new Struct()");
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new TypeError(`${JSON.stringify(name)} is not a WGSL name`);
    const members: Member[] = [];
    const offsets: Record<string, number> = {};
    let end = 0;
    let align = 1;
    let floor = 0;
    let runtimeArray: Member | null = null;
    const names = Object.keys(fields);
    if (names.length === 0) throw new TypeError(`struct ${name} has no fields`);
    for (const field of names) {
      const given: unknown = fields[field];
      if (!(given instanceof Struct) && !WgslType.node(given)) {
        const hint =
          given === "bool" || typeof given === "boolean" ? ": WGSL has no bool in a buffer, use wgsl.u32" : "";
        throw new TypeError(`${name}.${field} is ${describe(given)}, not a WGSL type or a struct${hint}`);
      }
      const type = given as FieldType;
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(field)) throw new TypeError(`${JSON.stringify(field)} is not a WGSL name`);
      if (runtimeArray)
        throw new TypeError(`${name}.${runtimeArray.name} is a runtime-sized array, so it must be the last field`);
      const member = place(name, field, type, layout, Math.max(end, floor));
      members.push(member);
      offsets[field] = member.offset;
      end = member.offset + type.size;
      floor = member.offset + (member.sizeAttribute || type.size);
      align = Math.max(align, member.alignAttribute || type.align);
      const node = WgslType.node(type);
      if (node?.kind === "array" && node.count === undefined) runtimeArray = member;
    }
    this.name = name;
    this.layout = layout;
    this.fields = fields;
    this.members = members;
    this.runtimeArray = runtimeArray;
    this.offsets = offsets as { [K in keyof F]: number };
    if (layout === "vertex") {
      // WebGPU wants a stride that is a multiple of 4.
      this.align = 1;
      this.size = roundUp(4, end);
    } else {
      this.align = align;
      this.size = roundUp(align, Math.max(end, floor));
    }
    Object.freeze(this.offsets);
    Object.freeze(this);
  }

  /** The size of one struct whose runtime-sized array has `elements` elements. */
  sizeOf(elements: number): number {
    const last = this.runtimeArray;
    if (!last) return this.size;
    const node = WgslType.node(last.type);
    const stride = node?.kind === "array" ? node.stride : 0;
    return roundUp(this.align, last.offset + elements * stride);
  }

  /** The declaration of this struct and of the structs it holds, innermost first. */
  toWGSL(): string {
    const order: Struct[] = [];
    const visit = (s: Struct) => {
      if (order.includes(s)) return;
      for (const member of s.members) {
        const node = WgslType.node(member.type);
        const inner = member.type instanceof Struct ? member.type : node?.kind === "array" ? node.element : null;
        if (inner instanceof Struct) visit(inner);
      }
      order.push(s);
    };
    visit(this as unknown as Struct);
    return order
      .map((s) => {
        const lines = s.members.map((m) => {
          const attributes =
            (m.alignAttribute ? `@align(${m.alignAttribute}) ` : "") +
            (m.sizeAttribute ? `@size(${m.sizeAttribute}) ` : "");
          return `  ${attributes}${m.name}: ${m.type instanceof Struct ? m.type.name : m.type.wgsl},`;
        });
        return `struct ${s.name} {\n${lines.join("\n")}\n}`;
      })
      .join("\n\n");
  }

  /** A vertex buffer layout of the fields given a shader location. */
  vertexLayout(
    locations: { readonly [K in keyof F]?: number },
    stepMode: "vertex" | "instance" = "vertex",
  ): VertexBufferLayout {
    const attributes: VertexBufferLayout["attributes"] = [];
    for (const [field, location] of Object.entries(locations) as [string, number | undefined][]) {
      if (location === undefined) continue;
      const member = this.members.find((m) => m.name === field);
      if (!member) throw new TypeError(`${this.name} has no field ${JSON.stringify(field)}`);
      const format = member.type instanceof Struct ? null : member.type.vertexFormat;
      if (!format) throw new TypeError(`${this.name}.${field} (${describe(member.type)}) cannot be a vertex attribute`);
      attributes.push({ format, offset: member.offset, shaderLocation: location });
    }
    attributes.sort((a, b) => a.shaderLocation - b.shaderLocation);
    return { arrayStride: this.size, stepMode, attributes };
  }

  /** A writer over `target`, or over a new buffer of its own. */
  writer(target?: ArrayBufferLike | ArrayBufferView, options: WriterOptions = {}): StructWriter<F> {
    const offset = options.byteOffset ?? 0;
    let buffer: ArrayBufferLike;
    let start = offset;
    let room: number;
    let elements = options.elements ?? 0;
    if (target === undefined) {
      room = this.runtimeArray ? this.sizeOf(elements) : this.size * (options.count ?? 1);
      buffer = new ArrayBuffer(offset + room);
    } else if (ArrayBuffer.isView(target)) {
      buffer = target.buffer;
      start = target.byteOffset + offset;
      room = target.byteLength - offset;
    } else {
      buffer = target;
      room = target.byteLength - offset;
    }
    if (!Number.isInteger(start) || start < 0 || start % 4 !== 0) {
      throw new RangeError(`A struct starts at a byte offset that is a multiple of 4, not ${start}`);
    }
    let count: number;
    if (this.runtimeArray) {
      const node = WgslType.node(this.runtimeArray.type);
      const stride = node?.kind === "array" ? node.stride : 1;
      if (target !== undefined) elements = Math.max(0, Math.floor((room - this.runtimeArray.offset) / stride));
      count = 1;
      if ((options.count ?? 1) !== 1)
        throw new RangeError(`${this.name} ends in a runtime-sized array, so a buffer holds one`);
      if (room < this.runtimeArray.offset)
        throw new RangeError(`${this.name} needs ${this.runtimeArray.offset} bytes, the target has ${room}`);
    } else {
      count = options.count ?? Math.floor(room / this.size);
      if (!Number.isInteger(count) || count < 1 || count * this.size > room) {
        throw new RangeError(`${count} of ${this.name} need ${count * this.size} bytes, the target has ${room}`);
      }
    }
    return new StructWriter<F>(KEY, this as unknown as Struct, buffer, start, count, elements);
  }
}

function place(struct: string, field: string, type: FieldType, layout: StructLayout, from: number): Member {
  const node = WgslType.node(type);
  if (layout === "vertex") {
    if (!node || (node.kind !== "scalar" && node.kind !== "vector" && node.kind !== "normalized")) {
      throw new TypeError(`${struct}.${field}: a vertex struct holds scalars and vectors, not ${describe(type)}`);
    }
    // WebGPU wants an attribute at a multiple of its component size, 4 at most.
    const unit = node.kind === "normalized" ? node.bytes : scalarSize(node.scalar);
    return { name: field, type, offset: roundUp(unit, from), alignAttribute: 0, sizeAttribute: 0 };
  }
  if (node?.kind === "normalized") {
    throw new TypeError(
      `${struct}.${field}: ${describe(type)} is a vertex attribute format, which a ${layout} buffer cannot hold. ` +
        `Use wgsl.vec${node.length}(wgsl.f32), or pack the bytes into a wgsl.u32 and unpack them in the shader.`,
    );
  }
  let align = type.align;
  let size = 0;
  if (layout === "uniform") {
    if (type instanceof Struct) {
      requireUniform(struct, field, type);
      align = roundUp(16, align);
      // What follows a struct starts at least a multiple of 16 after its start.
      size = roundUp(16, type.size);
    } else if (node?.kind === "array") {
      if (node.count === undefined) {
        throw new TypeError(`${struct}.${field}: a runtime-sized array needs layout: "storage"`);
      }
      if (node.element instanceof Struct) requireUniform(struct, field, node.element);
      if (node.stride % 16 !== 0) {
        const fix =
          node.element instanceof Struct
            ? `give ${node.element.name} fields up to a multiple of 16 bytes`
            : "use an array of vec4, or a struct padded to 16 bytes";
        throw new TypeError(
          `${struct}.${field}: in a uniform buffer an array element takes a multiple of 16 bytes, and ${describe(node.element)} takes ${node.stride}. ` +
            `Either ${fix}, or pass layout: "storage" for a storage buffer.`,
        );
      }
      align = roundUp(16, align);
    }
  }
  return {
    name: field,
    type,
    offset: roundUp(align, from),
    alignAttribute: align === type.align ? 0 : align,
    sizeAttribute: size === type.size ? 0 : size,
  };
}

function requireUniform(struct: string, field: string, inner: Struct): void {
  if (inner.layout !== "uniform") {
    throw new TypeError(
      `${struct}.${field}: ${inner.name} has layout ${JSON.stringify(inner.layout)} and is used in a uniform struct. Declare it with layout: "uniform".`,
    );
  }
}

/**
 * Declares a WGSL struct and computes its layout.
 * `"uniform"` (the default) fits a uniform buffer and a storage buffer alike; `"storage"` allows what only
 * a storage buffer does; `"vertex"` packs the fields for a vertex buffer.
 */
export function struct<F extends Fields>(name: string, fields: F, options: { layout?: StructLayout } = {}): Struct<F> {
  const layout = options.layout ?? "uniform";
  if (layout !== "uniform" && layout !== "storage" && layout !== "vertex") {
    throw new TypeError(`layout is "uniform", "storage" or "vertex", not ${JSON.stringify(layout)}`);
  }
  for (const type of Object.values(fields)) {
    if (type instanceof Struct && layout !== "vertex" && type.layout === "vertex") {
      throw new TypeError(`${name}: ${type.name} is a vertex struct and cannot be a field of a buffer struct`);
    }
  }
  return new Struct(KEY, name, { ...fields }, layout);
}

const U32_MAX = 0xffff_ffff;
const I32_MIN = -0x8000_0000;
const I32_MAX = 0x7fff_ffff;

export class StructWriter<F extends Fields = Fields> {
  readonly buffer: ArrayBufferLike;
  /** Where the first struct starts in `buffer`, and how many bytes the structs take. */
  readonly byteOffset: number;
  readonly byteLength: number;
  readonly count: number;
  /** The length of the runtime-sized array, 0 without one. */
  readonly elements: number;
  readonly #struct: Struct;
  readonly #members: Map<string, Member>;
  readonly #f32: Float32Array;
  readonly #u32: Uint32Array;
  readonly #i32: Int32Array;
  #f16: Float16Array | null = null;
  #u8: Uint8Array | null = null;
  #i8: Int8Array | null = null;
  #u16: Uint16Array | null = null;
  #i16: Int16Array | null = null;
  #base: number;

  /** @internal */
  constructor(key: typeof KEY, of: Struct, buffer: ArrayBufferLike, start: number, count: number, elements: number) {
    if (key !== KEY) throw new TypeError("A writer comes from a struct's writer(), not from new StructWriter()");
    this.#struct = of;
    this.#members = new Map(of.members.map((m) => [m.name, m]));
    this.buffer = buffer;
    this.byteOffset = start;
    this.byteLength = of.runtimeArray ? of.sizeOf(elements) : count * of.size;
    this.count = count;
    this.elements = elements;
    const words = buffer.byteLength >> 2;
    this.#f32 = new Float32Array(buffer, 0, words);
    this.#u32 = new Uint32Array(buffer, 0, words);
    this.#i32 = new Int32Array(buffer, 0, words);
    this.#base = start;
  }

  /** Moves to the struct at `index`; the writes that follow go there. */
  at(index: number): this {
    if (!Number.isInteger(index) || index < 0 || index >= this.count) {
      throw new RangeError(`This writer holds ${this.count} of ${this.#struct.name}; there is none at ${index}`);
    }
    this.#base = this.byteOffset + index * this.#struct.size;
    return this;
  }

  set<K extends keyof F & string>(field: K, value: FieldValue): this {
    const member = this.#members.get(field);
    if (!member) throw new TypeError(`${this.#struct.name} has no field ${JSON.stringify(field)}`);
    this.#write(member.type, this.#base + member.offset, value, field);
    return this;
  }

  setAll(values: { readonly [K in keyof F]?: FieldValue }): this {
    for (const field in values) {
      const value = values[field];
      if (value !== undefined) this.set(field, value);
    }
    return this;
  }

  /** A writer over the struct in `field`, or over element `index` of an array of structs, in the same buffer. */
  view<K extends keyof F & string>(field: K, index = 0): StructWriter {
    const member = this.#members.get(field);
    if (!member) throw new TypeError(`${this.#struct.name} has no field ${JSON.stringify(field)}`);
    const node = WgslType.node(member.type);
    let inner: FieldType = member.type;
    let offset = this.#base + member.offset;
    if (node?.kind === "array") {
      const length = node.count ?? this.elements;
      if (!Number.isInteger(index) || index < 0 || index >= length) {
        throw new RangeError(`${this.#struct.name}.${field} has ${length} elements; there is none at ${index}`);
      }
      inner = node.element;
      offset += index * node.stride;
    }
    if (!(inner instanceof Struct))
      throw new TypeError(`${this.#struct.name}.${field} is ${describe(inner)}, not a struct`);
    return new StructWriter(KEY, inner, this.buffer, offset, 1, 0);
  }

  /** Zeroes every struct of the writer. */
  clear(): this {
    new Uint8Array(this.buffer, this.byteOffset, this.byteLength).fill(0);
    return this;
  }

  #put(kind: Scalar, at: number, value: number, path: string): void {
    switch (kind) {
      case "f32":
        this.#f32[at >> 2] = value;
        return;
      case "u32":
        if (!Number.isInteger(value) || value < 0 || value > U32_MAX)
          throw new RangeError(`${path}: ${value} is not a u32`);
        this.#u32[at >> 2] = value;
        return;
      case "i32":
        if (!Number.isInteger(value) || value < I32_MIN || value > I32_MAX)
          throw new RangeError(`${path}: ${value} is not an i32`);
        this.#i32[at >> 2] = value;
        return;
      case "f16":
        (this.#f16 ??= new Float16Array(this.buffer, 0, this.buffer.byteLength >> 1))[at >> 1] = value;
    }
  }

  // Clamped to the range the shader sees, then rounded to the nearest step, halves away from zero.
  #putNormalized(node: Extract<Node, { kind: "normalized" }>, at: number, index: number, value: number, path: string) {
    if (typeof value !== "number" || Number.isNaN(value)) throw new RangeError(`${path}: ${value} is not a number`);
    const steps = node.signed ? (node.bytes === 1 ? 0x7f : 0x7fff) : node.bytes === 1 ? 0xff : 0xffff;
    const clamped = Math.min(1, Math.max(node.signed ? -1 : 0, value));
    const q = Math.round(Math.abs(clamped) * steps);
    const stored = clamped < 0 ? -q : q;
    if (node.bytes === 1) {
      (node.signed ? (this.#i8 ??= new Int8Array(this.buffer)) : (this.#u8 ??= new Uint8Array(this.buffer)))[
        at + index
      ] = stored;
    } else {
      const words = this.buffer.byteLength >> 1;
      (node.signed
        ? (this.#i16 ??= new Int16Array(this.buffer, 0, words))
        : (this.#u16 ??= new Uint16Array(this.buffer, 0, words)))[(at >> 1) + index] = stored;
    }
  }

  #numbers(value: unknown, path: string, expected: string): ArrayLike<number> {
    if (typeof value === "object" && value !== null && typeof (value as ArrayLike<number>).length === "number") {
      return value as ArrayLike<number>;
    }
    throw new TypeError(`${path} takes ${expected}`);
  }

  #write(type: FieldType, at: number, value: unknown, path: string): void {
    if (type instanceof Struct) {
      if (typeof value !== "object" || value === null)
        throw new TypeError(`${path} takes an object with the fields of ${type.name}`);
      for (const member of type.members) {
        const inner = (value as Record<string, unknown>)[member.name];
        if (inner !== undefined) this.#write(member.type, at + member.offset, inner, `${path}.${member.name}`);
      }
      return;
    }
    const node = WgslType.node(type);
    if (!node) return;
    switch (node.kind) {
      case "scalar":
      case "atomic":
        if (typeof value !== "number") throw new TypeError(`${path} takes a number`);
        this.#put(node.scalar, at, value, path);
        return;
      case "vector": {
        const step = scalarSize(node.scalar);
        if (typeof value === "object" && value !== null && typeof (value as ArrayLike<number>).length !== "number") {
          const v = value as VectorLike;
          this.#put(node.scalar, at, v.x, path);
          this.#put(node.scalar, at + step, v.y, path);
          if (node.length > 2) this.#put(node.scalar, at + 2 * step, v.z ?? 0, path);
          if (node.length > 3) this.#put(node.scalar, at + 3 * step, v.w ?? 0, path);
          return;
        }
        const from = this.#numbers(value, path, `${node.length} numbers or { x, y, z, w }`);
        if (from.length < node.length) throw new RangeError(`${path} takes ${node.length} numbers, got ${from.length}`);
        for (let i = 0; i < node.length; i++) this.#put(node.scalar, at + i * step, from[i] ?? 0, path);
        return;
      }
      case "normalized": {
        if (typeof value === "object" && value !== null && typeof (value as ArrayLike<number>).length !== "number") {
          const v = value as VectorLike;
          this.#putNormalized(node, at, 0, v.x, path);
          this.#putNormalized(node, at, 1, v.y, path);
          if (node.length > 2) {
            this.#putNormalized(node, at, 2, v.z ?? 0, path);
            this.#putNormalized(node, at, 3, v.w ?? 0, path);
          }
          return;
        }
        const from = this.#numbers(value, path, `${node.length} numbers or { x, y, z, w }`);
        if (from.length < node.length) throw new RangeError(`${path} takes ${node.length} numbers, got ${from.length}`);
        for (let i = 0; i < node.length; i++) this.#putNormalized(node, at, i, from[i] ?? 0, path);
        return;
      }
      case "matrix": {
        const from = this.#numbers(value, path, `${node.columns * node.rows} numbers, column by column`);
        const column = (node.rows === 3 ? 4 : node.rows) * 4;
        // Tight (9 for a mat3x3) or as the buffer holds it (12): the writer pads the columns of the first.
        const tight = from.length === node.columns * node.rows;
        if (!tight && from.length !== (node.columns * column) / 4) {
          throw new RangeError(`${path} takes ${node.columns * node.rows} numbers, got ${from.length}`);
        }
        const step = tight ? node.rows : column / 4;
        for (let c = 0; c < node.columns; c++) {
          for (let r = 0; r < node.rows; r++) this.#f32[(at + c * column + r * 4) >> 2] = from[c * step + r] ?? 0;
        }
        return;
      }
      case "array": {
        const length = node.count ?? this.elements;
        const element = node.element;
        const inner = WgslType.node(element);
        const from = this.#numbers(value, path, "an array");
        if (element instanceof Struct || inner?.kind === "array") {
          if (from.length > length) throw new RangeError(`${path} has ${length} elements, got ${from.length}`);
          for (let i = 0; i < from.length; i++) this.#write(element, at + i * node.stride, from[i], `${path}[${i}]`);
          return;
        }
        // Scalars, vectors and matrices come as one flat run of numbers.
        const each =
          inner?.kind === "vector" ? inner.length : inner?.kind === "matrix" ? inner.columns * inner.rows : 1;
        if (from.length % each !== 0 || from.length / each > length) {
          throw new RangeError(
            `${path} takes up to ${length * each} numbers, ${each} for each element, got ${from.length}`,
          );
        }
        if (each === 1) {
          for (let i = 0; i < from.length; i++) this.#write(element, at + i * node.stride, from[i], path);
          return;
        }
        const one = new Array<number>(each);
        for (let i = 0; i < from.length / each; i++) {
          for (let k = 0; k < each; k++) one[k] = from[i * each + k] ?? 0;
          this.#write(element, at + i * node.stride, one, path);
        }
      }
    }
  }
}
