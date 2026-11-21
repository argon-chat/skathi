/// <reference types="@webgpu/types" />
// The parts of `skathi:gpu` that talk to a device: pass timing and reading back.

export interface GpuTimes {
  /** From the start of the first timed pass to the end of the last, in milliseconds. */
  total: number;
  /** The time of each label, the longest first. Passes with one label are summed. */
  passes: { label: string; ms: number }[];
}

interface Slot {
  buffer: GPUBuffer;
  busy: boolean;
  pairs: { label: string; begin: number }[];
}

/** How many frames may be on their way back at once; a frame past that is not timed. */
const IN_FLIGHT = 3;

/** Times render and compute passes on the GPU with timestamp queries. */
export class GpuTimer {
  /** False when the device was not requested with `timestamp-query`: the timer then does nothing. */
  readonly enabled: boolean;
  /** The newest frame that has come back, null until one has. */
  last: GpuTimes | null = null;
  readonly #queries: GPUQuerySet | null;
  readonly #resolved: GPUBuffer | null;
  readonly #slots: Slot[] = [];
  readonly #capacity: number;
  #pairs: { label: string; begin: number }[] = [];
  #pending: Slot | null = null;
  #dead = false;

  constructor(device: GPUDevice, options: { passes?: number } = {}) {
    const passes = options.passes ?? 64;
    if (!Number.isInteger(passes) || passes < 1 || passes > 2048) {
      throw new RangeError(`passes is a whole number from 1 to 2048, not ${passes}`);
    }
    this.#capacity = passes;
    this.enabled = device.features.has("timestamp-query");
    if (!this.enabled) {
      this.#queries = this.#resolved = null;
      return;
    }
    const size = passes * 16;
    this.#queries = device.createQuerySet({ type: "timestamp", count: passes * 2 });
    this.#resolved = device.createBuffer({ size, usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC });
    for (let i = 0; i < IN_FLIGHT; i++) {
      const buffer = device.createBuffer({ size, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
      this.#slots.push({ buffer, busy: false, pairs: [] });
    }
  }

  /** Spread into the descriptor of a pass to time it. Empty when disabled or the frame has used every pair. */
  pass(label: string): { timestampWrites?: GPURenderPassTimestampWrites } {
    const querySet = this.#queries;
    if (this.#dead || !querySet || this.#pairs.length >= this.#capacity) return {};
    const begin = this.#pairs.length * 2;
    this.#pairs.push({ label, begin });
    return { timestampWrites: { querySet, beginningOfPassWriteIndex: begin, endOfPassWriteIndex: begin + 1 } };
  }

  /** Ends the frame: records the readout into `encoder`, which must be submitted after it. */
  resolve(encoder: GPUCommandEncoder): void {
    const pairs = this.#pairs;
    this.#pairs = [];
    const queries = this.#queries;
    const resolved = this.#resolved;
    if (this.#dead || !queries || !resolved || pairs.length === 0) return;
    // A frame resolved but never collected gives its buffer back.
    if (this.#pending) this.#pending.busy = false;
    this.#pending = null;
    const slot = this.#slots.find((s) => !s.busy);
    if (!slot) return;
    encoder.resolveQuerySet(queries, 0, pairs.length * 2, resolved, 0);
    encoder.copyBufferToBuffer(resolved, 0, slot.buffer, 0, pairs.length * 16);
    slot.pairs = pairs;
    slot.busy = true;
    this.#pending = slot;
  }

  /** Reads the resolved frame back. Call after `queue.submit()`; resolves with the times, also left in `last`. */
  async collect(): Promise<GpuTimes | null> {
    const slot = this.#pending;
    this.#pending = null;
    if (!slot || this.#dead) return null;
    try {
      await slot.buffer.mapAsync(GPUMapMode.READ, 0, slot.pairs.length * 16);
      const stamps = new BigInt64Array(slot.buffer.getMappedRange(0, slot.pairs.length * 16).slice(0));
      slot.buffer.unmap();
      let first = -1n;
      let end = -1n;
      const sums = new Map<string, bigint>();
      for (const pair of slot.pairs) {
        const from = stamps[pair.begin] ?? 0n;
        const to = stamps[pair.begin + 1] ?? 0n;
        if (to <= from) continue;
        sums.set(pair.label, (sums.get(pair.label) ?? 0n) + (to - from));
        if (first < 0n || from < first) first = from;
        if (to > end) end = to;
      }
      const times: GpuTimes = {
        total: first >= 0n ? Number(end - first) / 1e6 : 0,
        passes: [...sums].map(([label, ns]) => ({ label, ms: Number(ns) / 1e6 })).sort((a, b) => b.ms - a.ms),
      };
      if (!this.#dead) this.last = times;
      return times;
    } finally {
      slot.busy = false;
    }
  }

  destroy(): void {
    if (this.#dead) return;
    this.#dead = true;
    this.#queries?.destroy();
    this.#resolved?.destroy();
    for (const slot of this.#slots) slot.buffer.destroy();
  }
}

/** The bytes one texel takes, for the formats a copy to a buffer can read. */
const TEXEL_BYTES: Partial<Record<string, number>> = {
  r8unorm: 1,
  r8snorm: 1,
  r8uint: 1,
  r8sint: 1,
  r16uint: 2,
  r16sint: 2,
  r16float: 2,
  rg8unorm: 2,
  rg8snorm: 2,
  rg8uint: 2,
  rg8sint: 2,
  r32uint: 4,
  r32sint: 4,
  r32float: 4,
  rg16uint: 4,
  rg16sint: 4,
  rg16float: 4,
  rgba8unorm: 4,
  "rgba8unorm-srgb": 4,
  rgba8snorm: 4,
  rgba8uint: 4,
  rgba8sint: 4,
  bgra8unorm: 4,
  "bgra8unorm-srgb": 4,
  rgb10a2unorm: 4,
  rgb10a2uint: 4,
  rg11b10ufloat: 4,
  rgb9e5ufloat: 4,
  rg32uint: 8,
  rg32sint: 8,
  rg32float: 8,
  rgba16uint: 8,
  rgba16sint: 8,
  rgba16float: 8,
  rgba32uint: 16,
  rgba32sint: 16,
  rgba32float: 16,
  depth32float: 4,
  depth16unorm: 2,
};

export interface ReadTextureOptions {
  mipLevel?: number;
  /** The first texel and the size of the region; the whole level by default. */
  origin?: { x?: number; y?: number; z?: number };
  size?: { width: number; height: number };
}

export interface TexturePixels {
  /** The rows one after another, without the padding a GPU copy puts between them. */
  data: Uint8Array;
  width: number;
  height: number;
  bytesPerRow: number;
  format: GPUTextureFormat;
}

/** Copies a region of a texture to memory. The texture needs the `COPY_SRC` usage. */
export async function readTexture(
  device: GPUDevice,
  texture: GPUTexture,
  options: ReadTextureOptions = {},
): Promise<TexturePixels> {
  const texel = TEXEL_BYTES[texture.format];
  if (texel === undefined) throw new TypeError(`readTexture() cannot read a ${texture.format} texture`);
  const mipLevel = options.mipLevel ?? 0;
  const x = options.origin?.x ?? 0;
  const y = options.origin?.y ?? 0;
  const width = options.size?.width ?? Math.max(1, texture.width >> mipLevel) - x;
  const height = options.size?.height ?? Math.max(1, texture.height >> mipLevel) - y;
  if (!(width > 0 && height > 0)) throw new RangeError(`The region to read is ${width} by ${height}`);
  const bytesPerRow = width * texel;
  // A copy to a buffer needs rows that start 256 bytes apart.
  const padded = Math.ceil(bytesPerRow / 256) * 256;
  const staging = device.createBuffer({
    size: padded * height,
    usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST,
  });
  try {
    const encoder = device.createCommandEncoder();
    encoder.copyTextureToBuffer(
      { texture, mipLevel, origin: { x, y, z: options.origin?.z ?? 0 } },
      { buffer: staging, bytesPerRow: padded, rowsPerImage: height },
      { width, height },
    );
    device.queue.submit([encoder.finish()]);
    await staging.mapAsync(GPUMapMode.READ);
    const rows = new Uint8Array(staging.getMappedRange());
    const data = new Uint8Array(bytesPerRow * height);
    for (let row = 0; row < height; row++) {
      data.set(rows.subarray(row * padded, row * padded + bytesPerRow), row * bytesPerRow);
    }
    staging.unmap();
    return { data, width, height, bytesPerRow, format: texture.format };
  } finally {
    staging.destroy();
  }
}

/** Copies a range of a buffer to memory. The buffer needs the `COPY_SRC` usage. */
export async function readBuffer(
  device: GPUDevice,
  buffer: GPUBuffer,
  byteOffset = 0,
  byteLength = buffer.size - byteOffset,
): Promise<ArrayBuffer> {
  if (byteOffset % 4 !== 0 || byteOffset < 0 || byteLength < 0 || byteOffset + byteLength > buffer.size) {
    throw new RangeError(
      `${byteLength} bytes at ${byteOffset} are not a range of a ${buffer.size}-byte buffer at a multiple of 4`,
    );
  }
  const size = Math.ceil(byteLength / 4) * 4;
  if (size === 0) return new ArrayBuffer(0);
  const staging = device.createBuffer({ size, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });
  try {
    const encoder = device.createCommandEncoder();
    encoder.copyBufferToBuffer(buffer, byteOffset, staging, 0, Math.min(size, buffer.size - byteOffset));
    device.queue.submit([encoder.finish()]);
    await staging.mapAsync(GPUMapMode.READ);
    const copy = staging.getMappedRange().slice(0, byteLength);
    staging.unmap();
    return copy;
  } finally {
    staging.destroy();
  }
}
