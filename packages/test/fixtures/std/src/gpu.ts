import { GpuTimer, readBuffer, readTexture, struct, wgsl } from "skathi:gpu";

const Light = struct("Light", { position: wgsl.vec3(wgsl.f32), power: wgsl.f32 });
const Small = struct("Small", { v: wgsl.f32 });
const Scene = struct("Scene", {
  normal: wgsl.mat3x3f,
  tint: wgsl.vec3(wgsl.f32),
  frame: wgsl.u32,
  offsets: wgsl.array(wgsl.vec4(wgsl.f32), 2),
  small: Small,
  after: wgsl.f32,
  lights: wgsl.array(Light, 2),
  tail: wgsl.f32,
});

const READS = [
  "u.normal[0].x",
  "u.normal[0].z",
  "u.normal[1].y",
  "u.normal[2].x",
  "u.normal[2].z",
  "u.tint.x",
  "u.tint.z",
  "f32(u.frame)",
  "u.offsets[0].x",
  "u.offsets[1].w",
  "u.small.v",
  "u.after",
  "u.lights[0].power",
  "u.lights[1].position.y",
  "u.lights[1].power",
  "u.tail",
];
export const EXPECTED = [1, 3, 5, 7, 9, 0.5, 1, 7, 11, 18, 21, 22, 31, 42, 44, 50];

/** A compute shader reads the struct the writer filled, field by field, into a storage buffer. */
export async function gpu(): Promise<{
  read: number[];
  size: number;
  messages: string[];
  timed: string[] | null;
  pixels: number[];
  row: number;
}> {
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error("no adapter");
  const timestamps = adapter.features.has("timestamp-query");
  const device = await adapter.requestDevice({ requiredFeatures: timestamps ? ["timestamp-query"] : [] });
  const timer = new GpuTimer(device);
  const code = `${Scene.toWGSL()}

struct Out { values: array<f32, ${READS.length}> }
@group(0) @binding(0) var<uniform> u: Scene;
@group(0) @binding(1) var<storage, read_write> out: Out;

@compute @workgroup_size(1)
fn main() {
${READS.map((read, i) => `  out.values[${i}] = ${read};`).join("\n")}
}`;
  device.pushErrorScope("validation");
  const module = device.createShaderModule({ code });
  const info = await module.getCompilationInfo();
  const messages = info.messages.map((m) => `${m.type}: ${m.message}`);

  const writer = Scene.writer();
  writer.setAll({
    normal: [1, 2, 3, 4, 5, 6, 7, 8, 9],
    tint: { x: 0.5, y: 0.25, z: 1 },
    frame: 7,
    offsets: [11, 12, 13, 14, 15, 16, 17, 18],
    small: { v: 21 },
    after: 22,
    lights: [{ power: 31 }, { position: [41, 42, 43], power: 44 }],
    tail: 50,
  });

  const uniform = device.createBuffer({ size: Scene.size, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  device.queue.writeBuffer(uniform, 0, writer.buffer, writer.byteOffset, writer.byteLength);
  const bytes = READS.length * 4;
  const storage = device.createBuffer({ size: bytes, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC });
  const pipeline = device.createComputePipeline({ layout: "auto", compute: { module, entryPoint: "main" } });
  const bind = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: { buffer: uniform } },
      { binding: 1, resource: { buffer: storage } },
    ],
  });
  const encoder = device.createCommandEncoder();
  const pass = encoder.beginComputePass({ ...timer.pass("copy") });
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bind);
  pass.dispatchWorkgroups(1);
  pass.end();
  timer.resolve(encoder);
  device.queue.submit([encoder.finish()]);
  const times = await timer.collect();
  const failed = await device.popErrorScope();
  if (failed) messages.push(`validation: ${failed.message}`);
  const read = [...new Float32Array(await readBuffer(device, storage))];

  // Three texels to a row: a copy pads each row to 256 bytes, and the padding must not come back.
  const texture = device.createTexture({
    size: [3, 2],
    format: "rgba8unorm",
    usage: GPUTextureUsage.COPY_SRC | GPUTextureUsage.COPY_DST,
  });
  const texels = Uint8Array.from({ length: 24 }, (_, i) => i + 1);
  device.queue.writeTexture({ texture }, texels, { bytesPerRow: 12 }, [3, 2]);
  const got = await readTexture(device, texture);
  return {
    read,
    size: Scene.size,
    messages,
    timed: timer.enabled === timestamps && times ? times.passes.map((p) => p.label) : timestamps ? [] : null,
    pixels: [...got.data],
    row: got.bytesPerRow,
  };
}
