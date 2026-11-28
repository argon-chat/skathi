import { isUtf8 } from "node:buffer";
import { createHash } from "node:crypto";
import { compareBytes, KIND, PAGE, type SkpkInput } from "./skpk";
import { readBankIndex, writeStoreBank, type StoreFile } from "./skbank/format";

export { crc64, isSkBank, SKBANK_FORMAT } from "./skbank/format";

export interface SkBankEntry {
  name: string;
  kind: number;
  start: number;
  len: number;
  crc64: bigint;
}

export interface SkBank {
  entries: SkBankEntry[];
  index: Uint8Array;
  dataOffset: number;
  bytes(entry: SkBankEntry): Uint8Array;
  find(kind: number, name: string): SkBankEntry | undefined;
}

const textKinds = new Set<number>([KIND.META, KIND.WINDOW, KIND.SCRIPT, KIND.NATIVE_MANIFEST]);

function requireUtf8(name: string, data: Uint8Array): void {
  if (!isUtf8(data)) throw new Error(`SkBank: ${JSON.stringify(name)} is not valid UTF-8`);
}

function physicalPath(kind: number, name: string): string {
  if (!Number.isInteger(kind) || kind < 1 || kind > 0xffff) throw new Error("SkBank: invalid entry kind");
  const bytes = Buffer.from(name);
  if (
    bytes.toString("utf8") !== name ||
    /^[A-Za-z]:/.test(name) ||
    /[\\\0]/.test(name) ||
    name.split("/").some((part) => !part || part === "." || part === "..")
  )
    throw new Error("SkBank: expected a UTF-8 app-relative entry name");

  // The digest distinguishes case-only names even in DeHive's case-insensitive index.
  const digest = createHash("sha256").update(bytes).digest("hex");
  return `skathi/${kind.toString(16).padStart(4, "0")}/${digest}/${name}`;
}

function logicalKey(path: string): { kind: number; name: string } {
  const match = /^skathi\/([0-9a-f]{4})\/[0-9a-f]{64}\/(.+)$/s.exec(path);
  if (match) {
    const kind = Number.parseInt(match[1]!, 16);
    const name = match[2]!;
    if (physicalPath(kind, name) === path) return { kind, name };
  }
  throw new Error("SkBank: invalid Skathi key");
}

/** Write a v3 Store bank aligned for the file offset or resource RVA where it will be mapped. */
export function writeSkBank(inputs: readonly SkpkInput[], options: { baseOffset?: number } = {}): Uint8Array {
  const baseOffset = options.baseOffset ?? 0;
  if (!Number.isSafeInteger(baseOffset) || baseOffset < 0 || baseOffset > 0xffff_ffff) {
    throw new Error("SkBank: invalid base offset");
  }
  const sorted = inputs.map((input) => ({ input, nameBytes: Buffer.from(input.name) }));
  sorted.sort((a, b) => a.input.kind - b.input.kind || compareBytes(a.nameBytes, b.nameBytes));
  const files: StoreFile[] = sorted.map(({ input }, i) => {
    const { kind, name, data } = input;
    if (textKinds.has(kind)) requireUtf8(name, data);
    const previous = sorted[i - 1]?.input;
    if (previous?.kind === kind && previous.name === name) {
      throw new Error(`SkBank: duplicate entry ${name} (kind ${kind})`);
    }
    const log = input.alignLog2 ?? (kind === KIND.BYTECODE || kind === KIND.ICU_DATA ? 12 : 4);
    if (!Number.isInteger(log) || log < 0 || log > 16) throw new Error(`SkBank: invalid alignment for ${name}`);
    return { path: physicalPath(kind, name), data, alignment: Math.max(2 ** log, i === 0 ? PAGE : 1) };
  });
  return writeStoreBank(files, baseOffset);
}

/** Open the Skathi profile of a v3 bank and borrow its mapped Store entries. */
export function readSkBank(bytes: Uint8Array): SkBank {
  const directory = readBankIndex(bytes);
  const entries: SkBankEntry[] = [];
  const byKey = new Map<string, SkBankEntry>();
  const entryBytes = (entry: SkBankEntry) => bytes.subarray(entry.start, entry.start + entry.len);
  let dataOffset = directory.offset;
  for (const { path, start, len, crc64 } of directory.records) {
    const { kind, name } = logicalKey(path);
    const key = `${kind}/${name}`;
    if (byKey.has(key)) throw new Error("SkBank: duplicate Skathi key");

    const entry = { kind, name, start, len, crc64 };
    if (textKinds.has(kind)) requireUtf8(name, entryBytes(entry));
    entries.push(entry);
    byKey.set(key, entry);
    dataOffset = Math.min(dataOffset, start);
  }
  return {
    entries,
    dataOffset,
    index: bytes.subarray(directory.offset),
    bytes: entryBytes,
    find: (kind, name) => byKey.get(`${kind}/${name}`),
  };
}
