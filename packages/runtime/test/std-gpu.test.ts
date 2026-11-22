import { describe, expect, test } from "bun:test";
import { struct, wgsl } from "../src/std/gpu";

const f32s = (buffer: ArrayBufferLike) => [...new Float32Array(buffer)];

describe("skathi:gpu", () => {
  // The worked example of the WGSL specification, "Alignment and Size".
  test("a storage struct is laid out as the WGSL specification's example", () => {
    const A = struct("A", { u: wgsl.f32, v: wgsl.f32, w: wgsl.vec2(wgsl.f32), x: wgsl.f32 }, { layout: "storage" });
    expect([A.align, A.size, A.offsets]).toEqual([8, 24, { u: 0, v: 4, w: 8, x: 16 }]);
    const B = struct(
      "B",
      {
        a: wgsl.vec2(wgsl.f32),
        b: wgsl.vec3(wgsl.f32),
        c: wgsl.f32,
        d: wgsl.f32,
        e: A,
        f: wgsl.vec3(wgsl.f32),
        g: wgsl.array(A, 3),
        h: wgsl.i32,
      },
      { layout: "storage" },
    );
    expect([B.align, B.size]).toEqual([16, 160]);
    expect(B.offsets).toEqual({ a: 0, b: 16, c: 28, d: 32, e: 40, f: 64, g: 80, h: 152 });
    expect(B.toWGSL()).toStartWith("struct A {\n  u: f32,");
    expect(B.toWGSL()).toContain("  g: array<A, 3>,\n  h: i32,\n}");
  });

  // The same example in the uniform address space, as the specification rewrites it.
  test("a uniform struct aligns its structs and arrays to 16 and says so in the WGSL", () => {
    const A = struct("A", { u: wgsl.f32, v: wgsl.f32, w: wgsl.vec2(wgsl.f32), x: wgsl.f32, pad: wgsl.vec2(wgsl.f32) });
    expect(A.size).toBe(32);
    const B = struct("B", {
      a: wgsl.vec2(wgsl.f32),
      b: wgsl.vec3(wgsl.f32),
      c: wgsl.f32,
      d: wgsl.f32,
      e: A,
      f: wgsl.vec3(wgsl.f32),
      g: wgsl.array(A, 3),
      h: wgsl.i32,
    });
    expect(B.offsets).toEqual({ a: 0, b: 16, c: 28, d: 32, e: 48, f: 80, g: 96, h: 192 });
    expect([B.align, B.size]).toEqual([16, 208]);
    expect(B.toWGSL()).toContain("  @align(16) e: A,");
    expect(B.toWGSL()).toContain("  @align(16) g: array<A, 3>,");

    const Small = struct("Small", { v: wgsl.f32 });
    const Holder = struct("Holder", { s: Small, after: wgsl.f32 });
    expect(Holder.offsets).toEqual({ s: 0, after: 16 });
    expect(Holder.toWGSL()).toContain("  @align(16) @size(16) s: Small,");
  });

  test("what a uniform buffer cannot hold is refused with the way out", () => {
    expect(() => struct("U", { weights: wgsl.array(wgsl.f32, 4) })).toThrow(/multiple of 16 bytes.*layout: "storage"/s);
    expect(() => struct("U", { tail: wgsl.array(wgsl.f32) })).toThrow(/runtime-sized array needs layout: "storage"/);
    expect(() => struct("U", { on: "bool" as never })).toThrow(/use wgsl\.u32/);
    const S = struct("S", { v: wgsl.f32 }, { layout: "storage" });
    expect(() => struct("U", { s: S })).toThrow(/Declare it with layout: "uniform"/);
    expect(() => struct("T", { a: wgsl.array(wgsl.f32), b: wgsl.f32 }, { layout: "storage" })).toThrow(/last field/);
  });

  test("a writer pads matrix columns, takes vectors as arrays or objects and checks integers", () => {
    const U = struct("U", {
      normal: wgsl.mat3x3f,
      tint: wgsl.vec3(wgsl.f32),
      frame: wgsl.u32,
      offsets: wgsl.array(wgsl.vec4(wgsl.f32), 2),
    });
    expect(U.offsets).toEqual({ normal: 0, tint: 48, frame: 60, offsets: 64 });
    const w = U.writer();
    w.setAll({ normal: [1, 2, 3, 4, 5, 6, 7, 8, 9], tint: { x: 0.5, y: 0.25, z: 1 }, frame: 7 });
    w.set("offsets", [1, 2, 3, 4, 5, 6, 7, 8]);
    const f = f32s(w.buffer);
    expect(f.slice(0, 12)).toEqual([1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0]);
    expect(f.slice(12, 15)).toEqual([0.5, 0.25, 1]);
    expect(new Uint32Array(w.buffer)[15]).toBe(7);
    expect(f.slice(16, 24)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(() => w.set("frame", -1)).toThrow(RangeError);
    expect(() => w.set("frame", 1.5)).toThrow(RangeError);
    expect(() => w.set("tint", [1, 2])).toThrow(RangeError);
    expect(() => w.set("nope" as never, 1)).toThrow(/has no field/);
    expect(f32s(w.clear().buffer).every((v) => v === 0)).toBe(true);
  });

  test("a writer walks an array of structs in someone else's buffer, and views a nested struct", () => {
    const Light = struct("Light", { position: wgsl.vec3(wgsl.f32), power: wgsl.f32 });
    const Scene = struct("Scene", { count: wgsl.u32, lights: wgsl.array(Light, 2) });
    expect(Scene.offsets).toEqual({ count: 0, lights: 16 });

    const backing = new Float32Array(2 + 4 * 3);
    const lights = Light.writer(backing, { byteOffset: 8 });
    expect([lights.count, lights.byteOffset, lights.byteLength]).toEqual([3, 8, 48]);
    lights.at(2).set("power", 9);
    lights.at(0).set("position", [1, 2, 3]);
    expect([...backing.slice(2, 6)]).toEqual([1, 2, 3, 0]);
    expect(backing[2 + 8 + 3]).toBe(9);
    expect(() => lights.at(3)).toThrow(RangeError);

    const scene = Scene.writer();
    scene.set("lights", [{ power: 1 }, { position: [4, 5, 6], power: 2 }]);
    scene.view("lights", 0).set("position", { x: 7, y: 8, z: 9 });
    expect(f32s(scene.buffer).slice(4, 12)).toEqual([7, 8, 9, 1, 4, 5, 6, 2]);
  });

  test("a runtime-sized array takes its length from the buffer", () => {
    const Particles = struct(
      "Particles",
      { alive: wgsl.u32, items: wgsl.array(wgsl.vec4(wgsl.f32)) },
      { layout: "storage" },
    );
    expect([Particles.size, Particles.sizeOf(3)]).toEqual([16, 64]);
    const own = Particles.writer(undefined, { elements: 3 });
    expect([own.elements, own.byteLength]).toEqual([3, 64]);
    own.set("items", [1, 2, 3, 4, 5, 6, 7, 8]);
    expect(f32s(own.buffer).slice(4, 12)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(Particles.writer(new ArrayBuffer(48)).elements).toBe(2);
  });

  test("a vertex struct packs its fields and gives the pipeline its layout", () => {
    const Vertex = struct(
      "Vertex",
      { position: wgsl.vec3(wgsl.f32), uv: wgsl.vec2(wgsl.f16), color: wgsl.vec4(wgsl.f16), layer: wgsl.u32 },
      { layout: "vertex" },
    );
    expect(Vertex.offsets).toEqual({ position: 0, uv: 12, color: 16, layer: 24 });
    expect(Vertex.size).toBe(28);
    expect(Vertex.vertexLayout({ color: 2, position: 0, uv: 1 }, "instance")).toEqual({
      arrayStride: 28,
      stepMode: "instance",
      attributes: [
        { format: "float32x3", offset: 0, shaderLocation: 0 },
        { format: "float16x2", offset: 12, shaderLocation: 1 },
        { format: "float16x4", offset: 16, shaderLocation: 2 },
      ],
    });
    const w = Vertex.writer().set("uv", [0.5, 1]).set("layer", 3);
    expect([...new Float16Array(w.buffer, 12, 2)]).toEqual([0.5, 1]);
    expect(() => struct("V", { m: wgsl.mat4x4f }, { layout: "vertex" })).toThrow(/scalars and vectors/);
  });

  test("a normalized vertex field packs its components, at its component size, and the stride stays a multiple of 4", () => {
    const Vertex = struct(
      "Vertex",
      {
        flag: wgsl.unorm8x2,
        normal: wgsl.snorm16x4,
        color: wgsl.unorm8x4,
        edge: wgsl.snorm8x2,
        uv: wgsl.unorm16x2,
        tail: wgsl.unorm8x2,
      },
      { layout: "vertex" },
    );
    expect(Vertex.offsets).toEqual({ flag: 0, normal: 2, color: 10, edge: 14, uv: 16, tail: 20 });
    expect(Vertex.size).toBe(24);
    expect(Vertex.toWGSL()).toContain("  normal: vec4<f32>,\n  color: vec4<f32>,\n  edge: vec2<f32>,");
    expect(Vertex.vertexLayout({ color: 0, normal: 1, edge: 2, uv: 3, flag: 4 }).attributes).toEqual([
      { format: "unorm8x4", offset: 10, shaderLocation: 0 },
      { format: "snorm16x4", offset: 2, shaderLocation: 1 },
      { format: "snorm8x2", offset: 14, shaderLocation: 2 },
      { format: "unorm16x2", offset: 16, shaderLocation: 3 },
      { format: "unorm8x2", offset: 0, shaderLocation: 4 },
    ]);

    const w = Vertex.writer(undefined, { count: 2 }).at(1);
    w.setAll({
      color: [0, 0.5, 1, 1.5],
      edge: { x: -1, y: -2 },
      normal: [-1, -0.5, 0.25, 1],
      uv: [0.5, -0.1],
      tail: [0.501 / 255, 1.5 / 255],
    });
    const bytes = new Uint8Array(w.buffer, 24, 24);
    expect([...bytes.slice(10, 14)]).toEqual([0, 128, 255, 255]);
    expect([...new Int8Array(w.buffer, 24 + 14, 2)]).toEqual([-127, -127]);
    expect([...new Int16Array(w.buffer, 24 + 2, 4)]).toEqual([-32767, -16384, 8192, 32767]);
    expect([...new Uint16Array(w.buffer, 24 + 16, 2)]).toEqual([32768, 0]);
    expect([...bytes.slice(20, 22)]).toEqual([1, 2]);
    expect([...new Uint8Array(w.buffer, 0, 24)].every((b) => b === 0)).toBe(true);
    expect(() => w.set("color", [1, 1, 1])).toThrow(RangeError);
    expect(() => w.set("color", [Number.NaN, 0, 0, 0])).toThrow(RangeError);
  });

  test("a buffer struct refuses a normalized field", () => {
    expect(() => struct("U", { color: wgsl.unorm8x4 })).toThrow(TypeError);
    expect(() => struct("S", { color: wgsl.snorm16x2 }, { layout: "storage" })).toThrow(TypeError);
    expect(() => wgsl.array(wgsl.unorm8x4, 4)).toThrow(TypeError);
  });
});
