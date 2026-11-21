import { importLine, section } from "./shared";

const LAYOUTS =
  '`"uniform"`, the default, follows the rules of a uniform buffer, and a struct declared with it is valid in a storage buffer too. `"storage"` allows what only a storage buffer does. `"vertex"` packs the fields for a vertex buffer.';

export const gpu = section("skathi:gpu", [
  {
    slug: "wgsl",
    name: "wgsl",
    kind: "object",
    summary: "The WGSL types a struct field can have, each with its size and alignment in a buffer.",
    syntax: `${importLine("skathi:gpu", "wgsl")}

wgsl.f32, wgsl.u32, wgsl.i32, wgsl.f16: WgslType
wgsl.vec2(scalar: WgslType): WgslType
wgsl.vec3(scalar: WgslType): WgslType
wgsl.vec4(scalar: WgslType): WgslType
wgsl.mat2x2f, wgsl.mat3x3f, wgsl.mat4x4f: WgslType
wgsl.atomic(scalar: WgslType): WgslType
wgsl.array(element: WgslType | Struct, count?: number): WgslType

readonly type.wgsl: string
readonly type.size: number
readonly type.align: number
readonly type.vertexFormat: string | null`,
    notes: [
      "A type is a value: `wgsl.vec3(wgsl.f32)` is the same object every time, and `type.wgsl` is its spelling in a shader, `vec3<f32>`.",
      "`size` and `align` are in bytes, as the WGSL specification gives them. A `vec3<f32>` takes 12 bytes and is aligned to 16; a `mat3x3<f32>` takes 48, three columns of 16.",
      "`wgsl.array(element, count)` is `array<T, N>`. Its elements stand a stride apart: the element's size rounded up to its alignment.",
      '`wgsl.array(element)` with no count is a runtime-sized array. It can only be the last field of a `"storage"` struct.',
      "`wgsl.atomic()` takes `wgsl.u32` or `wgsl.i32`.",
      "`wgsl.f16` needs `enable f16;` in the shader and the `shader-f16` feature on the device.",
      "`vertexFormat` is the `GPUVertexFormat` of a vertex attribute of the type, or `null` when the type cannot be one.",
      "There is no `bool`: WGSL does not allow one in a buffer. Use `wgsl.u32`.",
    ],
    examples: [
      {
        title: "Sizes and alignments",
        lang: "ts",
        code: `import { wgsl } from "skathi:gpu";

const vec3f = wgsl.vec3(wgsl.f32);
vec3f.wgsl; // "vec3<f32>"
vec3f.size; // 12
vec3f.align; // 16
vec3f.vertexFormat; // "float32x3"

wgsl.mat3x3f.size; // 48
wgsl.array(wgsl.vec4(wgsl.f32), 8).size; // 128`,
      },
    ],
    related: ["struct", "struct-class", "struct-writer"],
    aliases: ["WgslType", "vec3", "mat4x4", "WGSL types", "alignment"],
  },
  {
    slug: "struct",
    name: "struct()",
    kind: "function",
    summary: "Declares a WGSL struct and computes where each field lies in a buffer.",
    syntax: `${importLine("skathi:gpu", "struct")}

struct(
  name: string,
  fields: Record<string, WgslType | Struct>,
  options?: { layout?: "uniform" | "storage" | "vertex" },
): Struct`,
    notes: [
      "The fields lie in the order of the object's keys. A field is a [`wgsl`](/std/wgsl/) type or another struct.",
      LAYOUTS,
      'In a `"uniform"` struct a field that is a struct or an array is aligned to 16 bytes, and the declaration `toWGSL()` writes says so with `@align` and `@size`, so the shader agrees.',
      'A `"uniform"` struct refuses what a uniform buffer cannot hold, and the error says the way out: an array whose elements do not take a multiple of 16 bytes, a runtime-sized array, a struct declared with another layout.',
      'A `"vertex"` struct holds scalars and vectors only. Each lies right after the one before, at a multiple of its component size, and the struct\'s size, the stride, is rounded up to 4.',
      "`struct()` throws a `TypeError` for a field that is not a type, a name WGSL does not accept, or a struct with no fields.",
    ],
    examples: [
      {
        title: "A uniform block",
        lang: "ts",
        code: `import { struct, wgsl } from "skathi:gpu";

const Light = struct("Light", {
  position: wgsl.vec3(wgsl.f32),
  power: wgsl.f32,
});

const Scene = struct("Scene", {
  viewProjection: wgsl.mat4x4f,
  normal: wgsl.mat3x3f,
  time: wgsl.f32,
  lights: wgsl.array(Light, 4),
});

Scene.size; // 192
Scene.offsets; // { viewProjection: 0, normal: 64, time: 112, lights: 128 }`,
      },
      {
        title: "A packed vertex",
        lang: "ts",
        code: `import { struct, wgsl } from "skathi:gpu";

const Vertex = struct(
  "Vertex",
  {
    position: wgsl.vec3(wgsl.f32),
    uv: wgsl.vec2(wgsl.f16),
    color: wgsl.vec4(wgsl.f16),
  },
  { layout: "vertex" },
);

Vertex.size; // 24
Vertex.offsets; // { position: 0, uv: 12, color: 16 }`,
      },
    ],
    related: ["wgsl", "struct-class", "struct-writer"],
    aliases: ["defineStruct", "uniform buffer", "std140", "std430", "struct layout", "padding"],
  },
  {
    slug: "struct-class",
    name: "Struct",
    kind: "class",
    summary: "A declared struct: its size, the offsets of its fields, its WGSL text, a vertex layout and writers.",
    syntax: `${importLine("skathi:gpu", "struct")}

readonly struct.name: string
readonly struct.layout: "uniform" | "storage" | "vertex"
readonly struct.size: number
readonly struct.align: number
readonly struct.offsets: { [field: string]: number }
readonly struct.fields: Record<string, WgslType | Struct>

struct.sizeOf(elements: number): number
struct.toWGSL(): string
struct.vertexLayout(locations: { [field: string]: number }, stepMode?: "vertex" | "instance"): GPUVertexBufferLayout
struct.writer(target?: ArrayBuffer | ArrayBufferView, options?: { count?: number; byteOffset?: number; elements?: number }): StructWriter`,
    notes: [
      "A struct is made by [`struct()`](/std/struct/) and is immutable.",
      "`size` is in bytes with the padding at the end, so it is also the distance between two structs in an array. `offsets` gives each field's first byte.",
      "`toWGSL()` gives the declaration to paste into a shader, with the structs it holds declared first. The layout in JS and in the shader come from the same description, so they cannot drift apart.",
      "`vertexLayout()` gives a `GPUVertexBufferLayout` for a render pipeline: `arrayStride` is the struct's size, and each field named in `locations` becomes an attribute at that shader location. A field that cannot be an attribute throws a `TypeError`.",
      "`writer()` with no target makes a buffer for one struct, or for `count` of them. With a target it writes into that buffer, from `byteOffset`, which must be a multiple of 4; `count` defaults to as many structs as fit.",
      "For a struct that ends in a runtime-sized array, `size` counts no elements and `sizeOf(n)` gives the size with `n`. A writer over a target takes the array's length from the room there is; a writer with a buffer of its own takes it from `elements`.",
    ],
    examples: [
      {
        title: "One description for the shader and the buffer",
        lang: "ts",
        code: `import { struct, wgsl } from "skathi:gpu";

declare const device: GPUDevice;

const Params = struct("Params", {
  tint: wgsl.vec3(wgsl.f32),
  time: wgsl.f32,
});

const module = device.createShaderModule({
  code: \`\${Params.toWGSL()}
@group(0) @binding(0) var<uniform> params: Params;
// ...\`,
});

const buffer = device.createBuffer({
  size: Params.size,
  usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
});
const writer = Params.writer();

export function frame(seconds: number): void {
  writer.set("tint", [1, 0.5, 0.25]).set("time", seconds);
  device.queue.writeBuffer(buffer, 0, writer.buffer);
}`,
      },
      {
        title: "A vertex buffer layout",
        lang: "ts",
        code: `import { struct, wgsl } from "skathi:gpu";

const Instance = struct(
  "Instance",
  { offset: wgsl.vec2(wgsl.f32), color: wgsl.vec4(wgsl.f32) },
  { layout: "vertex" },
);

export const instances = Instance.vertexLayout({ offset: 2, color: 3 }, "instance");
// { arrayStride: 24, stepMode: "instance", attributes: [
//   { format: "float32x2", offset: 0, shaderLocation: 2 },
//   { format: "float32x4", offset: 8, shaderLocation: 3 } ] }`,
      },
    ],
    related: ["struct", "struct-writer", "wgsl"],
    aliases: ["toWGSL", "vertexLayout", "GPUVertexBufferLayout", "offsets", "StructDef"],
  },
  {
    slug: "struct-writer",
    name: "StructWriter",
    kind: "class",
    summary: "Writes the fields of a struct, or of an array of structs, into a buffer at the right offsets.",
    syntax: `${importLine("skathi:gpu", "struct")}

readonly writer.buffer: ArrayBuffer
readonly writer.byteOffset: number
readonly writer.byteLength: number
readonly writer.count: number
readonly writer.elements: number

writer.set(field: string, value: FieldValue): this
writer.setAll(values: { [field: string]: FieldValue }): this
writer.at(index: number): this
writer.view(field: string, index?: number): StructWriter
writer.clear(): this`,
    notes: [
      "A writer comes from a struct's `writer()`. `buffer`, `byteOffset` and `byteLength` are what `queue.writeBuffer()` takes.",
      "`set()` takes a number for a scalar. For a vector it takes the numbers in order, as an array or a typed array, or an object with `x`, `y`, `z` and `w`.",
      "A matrix takes its numbers column by column. Nine numbers for a `mat3x3f` are spread over its three padded columns; twelve are taken as the buffer holds them.",
      "An array of scalars, vectors or matrices takes one flat run of numbers, and the writer puts each element at its stride. A field that is a struct takes an object by field name, and an array of structs an array of such objects. Fields left out keep their bytes.",
      "A `u32` or `i32` field throws a `RangeError` for a number that is not a whole number in its range, instead of wrapping it.",
      "`at(index)` moves the writer to another struct of the buffer; the calls after it write there.",
      "`view(field, index)` gives a writer over a struct inside this one, in the same buffer: the struct in `field`, or element `index` of an array of structs. Use it where one element changes each frame.",
      "`clear()` zeroes every struct of the writer.",
      "The writer is JavaScript, not a host call: a `set()` is a store or two into typed arrays, which the JIT inlines.",
    ],
    examples: [
      {
        title: "Many structs in one buffer",
        lang: "ts",
        code: `import { struct, wgsl } from "skathi:gpu";

const Particle = struct(
  "Particle",
  { position: wgsl.vec2(wgsl.f32), velocity: wgsl.vec2(wgsl.f32), life: wgsl.f32 },
  { layout: "storage" },
);

const COUNT = 1024;
const particles = Particle.writer(undefined, { count: COUNT });

for (let i = 0; i < COUNT; i++) {
  particles.at(i).set("position", [Math.random(), Math.random()]).set("life", 1);
}

particles.byteLength; // 24576: 1024 of 24 bytes`,
      },
      {
        title: "Nested structs and a matrix",
        lang: "ts",
        code: `import { struct, wgsl } from "skathi:gpu";

const Light = struct("Light", { position: wgsl.vec3(wgsl.f32), power: wgsl.f32 });
const Scene = struct("Scene", { normal: wgsl.mat3x3f, lights: wgsl.array(Light, 2) });

const scene = Scene.writer();
scene.setAll({
  normal: [1, 0, 0, 0, 1, 0, 0, 0, 1],
  lights: [{ position: [0, 4, 0], power: 10 }, { power: 2 }],
});

// One element, without touching the others.
scene.view("lights", 1).set("position", { x: 3, y: 1, z: 0 });`,
      },
    ],
    related: ["struct-class", "struct", "wgsl"],
    aliases: ["set", "setAll", "writeBuffer", "uniform writer", "mat3x3 padding"],
  },
]);
