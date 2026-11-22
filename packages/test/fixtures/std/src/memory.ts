import { BinaryReader, BinaryWriter, CRC32 } from "skathi:binary";
import { dlopen, NativeCallback, NativePointer } from "skathi:ffi";
import { struct, wgsl } from "skathi:gpu";
import { Arena, MemoryView, NativeBuffer } from "skathi:memory";

const attempt = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

const name = (run: () => unknown) => attempt(run).split(":")[0];

function buffers() {
  const fresh = NativeBuffer.alloc(16);
  const view = new Uint8Array(fresh.buffer);
  const zeroed = view.every((b) => b === 0);
  view.set([1, 2, 3, 4]);
  const before = [fresh.byteLength, fresh.buffer.byteLength, fresh.freed];
  fresh.free();
  const after = [fresh.byteLength, fresh.buffer.byteLength, fresh.freed, view.length];
  const transferred = NativeBuffer.alloc(8);
  const moved = transferred.buffer.transfer();
  return {
    zeroed,
    before,
    after,
    again: attempt(() => fresh.free()),
    aligned: NativePointer.of(NativeBuffer.alloc(8, { alignment: 4096 })).address % 4096n,
    empty: NativeBuffer.alloc(0).byteLength,
    transferred: [transferred.freed, moved.byteLength],
    errors: [
      name(() => NativeBuffer.alloc(-1)),
      name(() => NativeBuffer.alloc(8, { alignment: 3 })),
      name(() => new (NativeBuffer as unknown as new () => object)()),
    ],
  };
}

function binary() {
  const source = NativeBuffer.alloc(8);
  new DataView(source.buffer).setUint32(0, 0x01020304, true);
  new DataView(source.buffer).setUint16(4, 7, true);
  const reader = new BinaryReader(source);
  const header = reader.readStruct({ magic: "u32", count: "u16" });
  const copied = new BinaryWriter().bytes(source).finish();
  const crc = [CRC32.compute(source), CRC32.compute(new Uint8Array(source.buffer))];
  source.free();
  return {
    header,
    copied: [...copied],
    crc: crc[0] === crc[1],
    readFreed: name(() => reader.readArray(Uint8Array, 1)),
    newFreed: name(() => new BinaryReader(source)),
    crcFreed: name(() => CRC32.compute(source)),
  };
}

function views() {
  // Three vertices of { pos: vec3f, uv: vec2f }: the `u` and the `v` of each.
  const vertices = NativeBuffer.alloc(60);
  const u = new MemoryView(vertices, { type: "f32", byteOffset: 12, stride: 20, length: 3 });
  const v = new MemoryView(vertices, { type: "f32", byteOffset: 16, stride: 20, length: 3 });
  u.copyFrom([0.5, 0.25, 1]);
  v.copyFrom([1, 0.75]);
  v.set(2, 2);
  const floats = new Float32Array(vertices.buffer);
  const bytes = new MemoryView(new ArrayBuffer(4), { type: "u8", length: 4 });
  bytes.set(0, 257);
  bytes.copyFrom([-1, 256.7], 1);
  const doomed = NativeBuffer.alloc(8);
  const doomedView = new MemoryView(doomed, { type: "u32", length: 2 });
  const freeing = { valueOf: () => (doomed.free(), 5) };
  const midway = name(() => doomedView.copyFrom([1, freeing as unknown as number]));
  return {
    uv: [u.get(0), v.get(0), u.get(1), v.get(1), u.get(2), v.get(2)],
    interleaved: [...floats.subarray(3, 5), ...floats.subarray(8, 10), ...floats.subarray(13, 15)],
    untouched: [floats[0], floats[5], floats[10]],
    bytes: [bytes.get(0), bytes.get(1), bytes.get(2), bytes.get(3)],
    errors: [
      name(() => u.get(3)),
      name(() => u.copyFrom([1, 2], 2)),
      name(() => new MemoryView(vertices, { type: "f32", byteOffset: 12, stride: 20, length: 4 })),
      name(() => new MemoryView(vertices, { type: "f64", stride: 4, length: 2 })),
    ],
    midway,
    afterFree: [name(() => doomedView.get(0)), name(() => doomedView.set(0, 1)), name(() => doomedView.copyFrom([1]))],
    onFreed: name(() => new MemoryView(doomed, { type: "u8", length: 1 })),
  };
}

function gpu() {
  const Light = struct("Light", { position: wgsl.vec3(wgsl.f32), power: wgsl.f32 });
  const target = NativeBuffer.alloc(Light.size);
  const writer = Light.writer(target).set("position", [1, 2, 3]).set("power", 0.5);
  const written = [...new Float32Array(target.buffer)];
  target.free();
  return {
    written,
    afterFree: [name(() => writer.set("power", 1)), target.buffer.byteLength],
    onFreed: name(() => Light.writer(target)),
  };
}

function arenas() {
  const arena = new Arena({ chunkSize: 1024 });
  const first = arena.alloc(100);
  new Uint8Array(first.buffer).set([1, 2, 3, 4]);
  const second = arena.alloc(8, 256);
  const used = arena.used;
  const aligned = NativePointer.of(second).address % 256n;
  const held = NativePointer.of(first);
  arena.reset();
  const freed = [first.freed, second.freed, arena.used];
  const next = arena.alloc(100);
  return {
    used,
    aligned,
    freed,
    notReused: NativePointer.of(next).address !== held.address,
    kept: [...held.read(4)],
    zeroed: new Uint8Array(next.buffer).every((b) => b === 0),
  };
}

function ffi() {
  const crt = dlopen("crt", {
    memcpy: { args: ["buffer", "buffer", "usize"], returns: "pointer" },
    qsort: { args: ["buffer", "usize", "usize", "pointer"] },
  });
  const target = NativeBuffer.alloc(4);
  crt.symbols.memcpy(target, new Uint8Array([5, 6, 7, 8]), 4);
  const back = new Uint8Array(4);
  crt.symbols.memcpy(back, target, 4);

  const sorted = NativeBuffer.alloc(16);
  new Int32Array(sorted.buffer).set([3, 1, 4, 2]);
  const kept = NativePointer.of(sorted);
  const int32 = (p: NativePointer) => new Int32Array(p.read(4).buffer)[0]!;
  const descending = new NativeCallback(
    (a, b) => {
      sorted.free();
      return int32(b) - int32(a);
    },
    { args: ["pointer", "pointer"], returns: "i32" },
  );
  crt.symbols.qsort(sorted, 4, 4, descending.pointer);
  descending.close();
  const freed = NativeBuffer.alloc(4);
  freed.free();
  return {
    copied: [...new Uint8Array(target.buffer)],
    back: [...back],
    freedDuringCall: sorted.freed,
    sortedInPlace: [...new Int32Array(kept.read(16).buffer)],
    freedArgument: name(() => crt.symbols.memcpy(freed, back, 4)),
    freedPointer: name(() => NativePointer.of(freed)),
  };
}

const rejection = (promise: Promise<unknown>): Promise<string> =>
  promise.then(
    () => "resolved",
    (e: unknown) => (e as Error).name,
  );

/** A `NativeBuffer` handed to a call on another thread. */
export async function memoryWorkers() {
  const crt = dlopen("crt", {
    memcpy: { args: ["buffer", "buffer", "usize"], returns: "pointer", nonblocking: true },
  });
  const target = NativeBuffer.alloc(4);
  const earlier = new Uint8Array(target.buffer);
  const call = crt.symbols.memcpy(target, new Uint8Array([4, 3, 2, 1]), 4);
  const during = [target.buffer.byteLength, target.freed, earlier.length];
  const twice = await rejection(crt.symbols.memcpy(target, new Uint8Array(4), 4));
  const readDuring = name(() => new BinaryReader(target));
  await call;
  const dropped = NativeBuffer.alloc(4);
  const dropping = crt.symbols.memcpy(dropped, new Uint8Array([1, 1, 1, 1]), 4);
  dropped.free();
  await dropping;
  return {
    during,
    twice,
    readDuring,
    after: [...new Uint8Array(target.buffer)],
    earlier: earlier.length,
    droppedStaysFreed: [dropped.freed, dropped.buffer.byteLength],
  };
}

export function memory() {
  return { buffers: buffers(), binary: binary(), views: views(), gpu: gpu(), arenas: arenas(), ffi: ffi() };
}
