// bun benches/skbank/io.ts <source.skpk> <scratch-directory> — warm filesystem, median milliseconds.
import assert from "node:assert/strict";
import {
  closeSync,
  existsSync,
  fsyncSync,
  mkdtempSync,
  openSync,
  readFileSync,
  rmdirSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { readPayload } from "../../packages/cli/cli/payload";
import { readSkBank, writeSkBank } from "../../packages/cli/cli/skbank";

const [sourcePath, scratchPath] = Bun.argv.slice(2);
if (!sourcePath || !scratchPath) throw new Error("usage: bun benches/skbank/io.ts <source.skpk> <scratch-directory>");

const source = readPayload(readFileSync(sourcePath));
const bank = writeSkBank(source.entries.map((entry) => ({ ...entry, data: source.bytes(entry) })));
const samples: Record<"writeMs" | "flushMs" | "readMs" | "parseMs", number[]> = {
  writeMs: [],
  flushMs: [],
  readMs: [],
  parseMs: [],
};
const scratch = mkdtempSync(join(resolve(scratchPath), "skbank-bench-"));
const output = join(scratch, "bank.skpk");
try {
  for (let i = 0; i < 10; i++) {
    const file = openSync(output, "w");
    try {
      const start = performance.now();
      writeFileSync(file, bank);
      const written = performance.now();
      fsyncSync(file);
      const flushed = performance.now();
      if (i >= 3) {
        samples.writeMs.push(written - start);
        samples.flushMs.push(flushed - written);
      }
    } finally {
      closeSync(file);
    }

    const start = performance.now();
    const bytes = readFileSync(output);
    const read = performance.now();
    const parsed = readSkBank(bytes);
    const opened = performance.now();
    assert.equal(parsed.entries.length, source.entries.length);
    assert.ok(bytes.equals(bank));
    if (i >= 3) {
      samples.readMs.push(read - start);
      samples.parseMs.push(opened - read);
    }
  }
} finally {
  if (existsSync(output)) unlinkSync(output);
  rmdirSync(scratch);
}
const medians = Object.fromEntries(
  Object.entries(samples).map(([name, times]) => [name, times.sort((a, b) => a - b)[Math.floor(times.length / 2)]]),
);
console.log(
  JSON.stringify({ sourcePath, scratchPath, bytes: bank.length, samples: samples.writeMs.length, ...medians }),
);
