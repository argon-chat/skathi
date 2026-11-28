// bun benches/skbank/bench.ts [--pack <file.skpk>] — in-memory Store, median milliseconds per operation.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { cpus } from "node:os";
import { basename } from "node:path";
import { readPayload } from "../../packages/cli/cli/payload";
import { crc64, writeSkBank } from "../../packages/cli/cli/skbank";
import { crc32, KIND, writeSkpk, type SkpkInput } from "../../packages/cli/cli/skpk";

const samples = 7;
let sink = 0;

function timeBatch(op: () => number, iterations: number): number {
  const start = performance.now();
  for (let i = 0; i < iterations; i++) sink = (sink + op()) | 0;
  return performance.now() - start;
}

function measure(op: () => number): number {
  let iterations = 1;
  while (timeBatch(op, iterations) < 20 && iterations < 8192) iterations *= 2;
  const times: number[] = [];
  for (let i = 0; i < samples; i++) {
    Bun.gc(true);
    times.push(timeBatch(op, iterations) / iterations);
  }
  return times.sort((a, b) => a - b)[Math.floor(samples / 2)]!;
}

function makeFiles(count: number, size: number): SkpkInput[] {
  return Array.from({ length: count }, (_, index) => {
    // Both workload counts are powers of two; the odd multiplier permutes the names.
    const id = Math.imul(index, 2654435761) & (count - 1);
    const data = new Uint8Array(size);
    let state = id + 1;
    for (let i = 0; i < size; i++) {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      data[i] = state >>> 24;
    }
    return { name: `assets/item-${String(id).padStart(6, "0")}.bin`, data, kind: KIND.ASSET };
  });
}

function digest(parts: Iterable<Uint8Array>): string {
  const hash = createHash("sha256");
  for (const part of parts) hash.update(part);
  return hash.digest("hex");
}

function* workloads(): Generator<{ name: string; files: SkpkInput[] }> {
  const args = Bun.argv.slice(2);
  if (!args.length) {
    yield { name: "4096 x 1 KiB", files: makeFiles(4096, 1024) };
    yield { name: "4 x 8 MiB", files: makeFiles(4, 8 * 1024 * 1024) };
    return;
  }
  if (args.length !== 2 || args[0] !== "--pack")
    throw new Error("usage: bun benches/skbank/bench.ts [--pack <file.skpk>]");
  const path = args[1]!;
  const source = readPayload(readFileSync(path));
  const files = source.entries.map((entry) => ({
    name: entry.name,
    kind: entry.kind,
    data: source.bytes(entry),
    ...(entry.alignLog2 === undefined ? {} : { alignLog2: entry.alignLog2 }),
  }));
  yield { name: basename(path), files };
}

console.log(
  JSON.stringify({ bun: Bun.version, platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model, samples }),
);
for (const { name, files } of workloads()) {
  const count = files.length;
  const total = files.reduce((bytes, file) => bytes + file.data.length, 0);
  if (!total) throw new Error("benchmark requires at least one nonempty payload entry");
  const expected = digest(files.map((file) => file.data));
  for (const [format, write] of [
    ["SkBank", writeSkBank],
    ["SKPK v1", writeSkpk],
  ] as const) {
    const blob = write(files);
    const opened = readPayload(blob);
    const copy = new Uint8Array(total);
    assert.equal(opened.entries.length, count);
    // Read in file order independent of either writer's TOC order.
    const views = files.map((file) => opened.bytes(opened.find(file.kind, file.name)!));
    assert.equal(digest(views), expected);
    assert.ok(views.every((view) => view.buffer === blob.buffer));
    const writeMs = measure(() => write(files).length);
    const openMs = measure(() => readPayload(blob).entries.length);
    const lookupMs = measure(() => {
      let sum = 0;
      for (const file of files) sum += opened.bytes(opened.find(file.kind, file.name)!).byteOffset;
      return sum;
    });
    // Reuse the destination and pre-resolved views to isolate the cost of moving every data byte.
    const copyMs = measure(() => {
      let offset = 0;
      for (const view of views) {
        copy.set(view, offset);
        offset += view.length;
      }
      return copy[0]! + copy[offset - 1]!;
    });
    assert.equal(digest([copy]), expected);
    const crcMs = measure(
      format === "SkBank"
        ? () => {
            let combined = 0n;
            for (const file of files) combined ^= crc64(file.data);
            return Number(combined & 0xffffffffn);
          }
        : () => {
            let combined = 0;
            for (const file of files) combined ^= crc32(file.data);
            return combined;
          },
    );
    console.log(
      JSON.stringify({
        workload: name,
        format,
        count,
        inputBytes: total,
        archiveBytes: blob.length,
        writeMs,
        openMs,
        lookupMs,
        copyMs,
        crcMs,
      }),
    );
  }
}
console.log(JSON.stringify({ sink }));
