/** The JavaScriptCore archive of the aot-nojit workload (Windows x64): the pinned archive with every
 * reference to a JIT entry point redirected to redjs_removed_jit_entry, so the linker leaves the JIT out.
 * JSC lists those entry points itself, in its .bun_removable_entry_points records; jit-entries.json is
 * what they were at the reviewed pin, and every mode fails when the pinned archive says otherwise.
 *
 *   bun scripts/aot/no-jit.ts --out <dir>   <dir>/JavaScriptCoreNoJIT.lib and the receipt redjs's no-jit feature reads
 *   bun scripts/aot/no-jit.ts --check       fails unless jit-entries.json matches the pinned archive
 *   bun scripts/aot/no-jit.ts --write       rewrites jit-entries.json from the pinned archive, to review after a pin change
 */
import { createHash } from "node:crypto";
import { closeSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, writeFileSync, writeSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const SDK = resolve(import.meta.dir, "../..");
const INVENTORY = join(import.meta.dir, "jit-entries.json");
export const ARCHIVE = "JavaScriptCoreNoJIT.lib";
export const RECEIPT = ".redjs-no-jit";
export const REDIRECT = "redjs_removed_jit_entry";

const GUARD_SECTION = ".bun_removable_entry_points";
const RECORD_BYTES = 24;
const FEATURE_JIT = 0;
const STUB_ABORT = 0;
const ORIGIN_NATIVE_CODE = 0;
const ORIGIN_DEFINITION = 2;
const REL_ADDR64 = 1;
const REL_REL32 = 4;
const SCN_LNK_NRELOC_OVFL = 0x01000000;
const STORAGE_EXTERNAL = 2;
const STORAGE_STATIC = 3;
const REFERENCING_SECTIONS = new Set([".text", ".data", ".rdata"]);
const BIGOBJ_CLASS = Uint8Array.from([
  0xc7, 0xa1, 0xba, 0xd1, 0xee, 0xba, 0xa9, 0x4b, 0xaf, 0x20, 0xfa, 0xf6, 0x6a, 0xa4, 0xdc, 0xb8,
]);

export interface Inventory {
  schema: "skathi.no-jit-inventory";
  version: 1;
  engine: string;
  archiveSha256: string;
  feature: { name: "JIT"; id: number };
  references: { definition: number; callSite: number };
  entries: Record<string, { definition: number; callSite: number }>;
}

const latin1 = new TextDecoder("latin1");
const text = (bytes: Uint8Array, start: number, end: number) => latin1.decode(bytes.subarray(start, end));
const view = (bytes: Uint8Array) => new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
const buffer = (bytes: Uint8Array) => Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);

type MemberKind = "first-linker" | "second-linker" | "longnames" | "object";
interface Member {
  kind: MemberKind;
  header: number;
  start: number;
  size: number;
  name: string;
}

function members(archive: Uint8Array): Member[] {
  if (text(archive, 0, 8) !== "!<arch>\n") throw Error("expected a COFF archive");
  const list: Member[] = [];
  let longnames: Uint8Array | null = null;
  for (let offset = 8; offset < archive.length;) {
    if (archive[offset + 58] !== 0x60 || archive[offset + 59] !== 0x0a) throw Error(`bad archive member at ${offset}`);
    const field = text(archive, offset, offset + 16).trimEnd();
    const size = Number.parseInt(text(archive, offset + 48, offset + 58).trim(), 10);
    const start = offset + 60;
    let kind: MemberKind = "object";
    let name = field;
    if (field === "/") kind = list.some((m) => m.kind === "first-linker") ? "second-linker" : "first-linker";
    else if (field === "//") {
      kind = "longnames";
      longnames = archive.subarray(start, start + size);
    } else if (field.startsWith("/") && /^\/\d+$/.test(field)) {
      if (!longnames) throw Error(`member ${field} before the long-name table`);
      const at = Number(field.slice(1));
      let end = at;
      while (end < longnames.length && longnames[end] !== 0 && longnames[end] !== 0x0a) end++;
      name = text(longnames, at, end).replace(/\/$/, "");
    } else if (field.startsWith("/")) throw Error(`unsupported archive member ${field}`);
    else name = field.replace(/\/$/, "");
    list.push({ kind, header: offset, start, size, name });
    offset = start + size + (size % 2);
  }
  return list;
}

interface Section {
  number: number;
  name: string;
  size: number;
  raw: number;
  relocations: number;
  relocationCount: number;
  flags: number;
}

class CoffObject {
  readonly data: DataView;
  readonly big: boolean;
  readonly symbolsField: number;
  readonly symbols: number;
  readonly symbolCount: number;
  readonly symbolSize: number;
  readonly strings: number;
  readonly sections: Section[] = [];

  constructor(readonly bytes: Uint8Array) {
    const data = (this.data = view(bytes));
    this.big =
      data.getUint16(0, true) === 0 &&
      data.getUint16(2, true) === 0xffff &&
      buffer(bytes.subarray(12, 28)).equals(BIGOBJ_CLASS);
    const machine = data.getUint16(this.big ? 6 : 0, true);
    if (machine !== 0x8664) throw Error(`expected an AMD64 COFF object, got machine ${machine.toString(16)}`);
    this.symbolsField = this.big ? 48 : 8;
    this.symbols = data.getUint32(this.symbolsField, true);
    this.symbolCount = data.getUint32(this.symbolsField + 4, true);
    this.symbolSize = this.big ? 20 : 18;
    this.strings = this.symbols + this.symbolCount * this.symbolSize;
    const count = this.big ? data.getUint32(44, true) : data.getUint16(2, true);
    const table = this.big ? 56 : 20 + data.getUint16(16, true);
    for (let index = 0; index < count; index++) {
      const at = table + 40 * index;
      let name = text(bytes, at, at + 8).replace(/\0+$/, "");
      if (name.startsWith("/")) {
        if (!/^\/\d+$/.test(name)) throw Error(`unsupported section name ${name}`);
        name = this.string(Number(name.slice(1)));
      }
      const flags = data.getUint32(at + 36, true);
      const relocations = data.getUint32(at + 24, true);
      let relocationCount = data.getUint16(at + 32, true);
      if (flags & SCN_LNK_NRELOC_OVFL) relocationCount = data.getUint32(relocations, true);
      this.sections.push({
        number: index + 1,
        name,
        size: data.getUint32(at + 16, true),
        raw: data.getUint32(at + 20, true),
        relocations,
        relocationCount,
        flags,
      });
    }
  }

  string(offset: number): string {
    const start = this.strings + offset;
    const end = this.bytes.indexOf(0, start);
    return text(this.bytes, start, end);
  }

  symbolName(index: number): string {
    const at = this.symbols + index * this.symbolSize;
    if (this.data.getUint32(at, true) === 0) return this.string(this.data.getUint32(at + 4, true));
    return text(this.bytes, at, at + 8).replace(/\0+$/, "");
  }

  symbolSection(index: number): number {
    const at = this.symbols + index * this.symbolSize + 12;
    return this.big ? this.data.getInt32(at, true) : this.data.getInt16(at, true);
  }

  *relocations(section: Section): Generator<{ at: number; address: number; symbol: number; type: number }> {
    const first = section.flags & SCN_LNK_NRELOC_OVFL ? 1 : 0;
    for (let index = first; index < section.relocationCount; index++) {
      const at = section.relocations + 10 * index;
      yield {
        at,
        address: this.data.getUint32(at, true),
        symbol: this.data.getUint32(at + 4, true),
        type: this.data.getUint16(at + 8, true),
      };
    }
  }
}

interface EntryRecord {
  feature: number;
  origin: number;
  symbol: string;
  guardSection: number;
}

function removableEntries(object: CoffObject): EntryRecord[] {
  const records: EntryRecord[] = [];
  for (const section of object.sections) {
    if (section.name !== GUARD_SECTION) continue;
    if (section.size % RECORD_BYTES) throw Error("unknown removable-entry record format");
    const targets = new Map<number, number>();
    for (const relocation of object.relocations(section)) targets.set(relocation.address, relocation.symbol);
    for (let offset = 0; offset < section.size; offset += RECORD_BYTES) {
      const at = section.raw + offset;
      const guard = targets.get(offset);
      const entry = targets.get(offset + 8);
      if (guard === undefined || entry === undefined) throw Error("a removable-entry record without its relocations");
      const record = {
        feature: object.data.getUint16(at + 16, true),
        origin: object.bytes[at + 19]!,
        symbol: object.symbolName(entry),
        guardSection: object.symbolSection(guard),
      };
      if (record.feature === FEATURE_JIT) {
        const stub = object.bytes[at + 18]!;
        if (stub !== STUB_ABORT || ![ORIGIN_NATIVE_CODE, ORIGIN_DEFINITION].includes(record.origin))
          throw Error(`${record.symbol}: expected an Abort entry recorded at native code or its definition`);
        if (record.guardSection < 1) throw Error(`${record.symbol}: the guard is not in a section`);
        if (object.data.getBigUint64(at + 8, true) !== 0n)
          throw Error(`${record.symbol}: interior entry points are unsupported`);
      }
      records.push(record);
    }
  }
  return records;
}

type Kind = "definition" | "callSite";

/** `object` with each guarded reference to an entry pointing at a new undefined `REDIRECT` symbol; null when it has none. */
function redirect(
  object: CoffObject,
  entries: Set<string>,
  definitions: Set<string>,
  guards: Map<number, Set<string>> | undefined,
  count: (name: string, kind: Kind) => void,
): Uint8Array | null {
  const targets = new Map<number, string>();
  for (let index = 0; index < object.symbolCount;) {
    const at = object.symbols + index * object.symbolSize;
    const storage = object.bytes[at + object.symbolSize - 2];
    if (storage === STORAGE_EXTERNAL || storage === STORAGE_STATIC) {
      const name = object.symbolName(index);
      if (entries.has(name)) targets.set(index, name);
    }
    index += 1 + object.bytes[at + object.symbolSize - 1]!;
  }
  if (!targets.size) return null;
  const patches: number[] = [];
  for (const section of object.sections) {
    if (!REFERENCING_SECTIONS.has(section.name.split("$", 1)[0]!)) continue;
    for (const relocation of object.relocations(section)) {
      const name = targets.get(relocation.symbol);
      if (name === undefined) continue;
      let kind: Kind;
      if (definitions.has(name)) kind = "definition";
      else if (guards?.get(section.number)?.has(name)) kind = "callSite";
      else throw Error(`an unguarded reference to the JIT entry ${name}`);
      if (relocation.type !== REL_ADDR64 && relocation.type !== REL_REL32)
        throw Error(`an unexpected relocation (type ${relocation.type}) to ${name}`);
      const width = relocation.type === REL_ADDR64 ? 8 : 4;
      const site = object.bytes.subarray(section.raw + relocation.address, section.raw + relocation.address + width);
      if (site.some((b) => b !== 0)) throw Error(`an interior reference to ${name}`);
      patches.push(relocation.at);
      count(name, kind);
    }
  }
  if (!patches.length) return null;

  // The symbol and its name go at the end of the symbol and string tables: nothing may follow them.
  const stringsSize = object.data.getUint32(object.strings, true);
  for (const section of object.sections) {
    if (
      section.raw + section.size > object.symbols ||
      section.relocations + 10 * section.relocationCount > object.symbols
    )
      throw Error("a section lies after the symbol table");
  }
  if (object.strings + stringsSize !== object.bytes.length) throw Error("data after the string table");
  const name = new TextEncoder().encode(`${REDIRECT}\0`);
  const out = new Uint8Array(object.bytes.length + object.symbolSize + name.length);
  out.set(object.bytes.subarray(0, object.strings));
  const symbol = out.subarray(object.strings, object.strings + object.symbolSize);
  const record = view(symbol);
  record.setUint32(4, stringsSize, true);
  record.setUint16(object.big ? 16 : 14, 0x20, true);
  symbol[object.symbolSize - 2] = STORAGE_EXTERNAL;
  const strings = object.strings + object.symbolSize;
  out.set(object.bytes.subarray(object.strings), strings);
  out.set(name, strings + stringsSize);
  const data = view(out);
  data.setUint32(strings, stringsSize + name.length, true);
  data.setUint32(object.symbolsField + 4, object.symbolCount + 1, true);
  for (const at of patches) data.setUint32(at + 4, object.symbolCount, true);
  return out;
}

function pinnedEngine(): string {
  const pins = readFileSync(join(SDK, "vendor/redjs/redjs/pins.txt"), "utf8");
  const engine = /^webkit\.version\s*=\s*(\S+)/m.exec(pins)?.[1];
  if (!engine) throw Error("vendor/redjs/redjs/pins.txt has no webkit.version");
  return engine;
}

function checkPrebuilt(webkit: string, engine: string): void {
  const configuration = readFileSync(join(webkit, "include/cmakeconfig.h"), "utf8");
  if (!configuration.includes(`#define BUN_WEBKIT_VERSION "${engine}"`))
    throw Error(`${webkit} is not the pinned prebuilt ${engine}; run vendor/redjs/setup.ps1`);
  const features = readFileSync(join(webkit, "include/JavaScriptCore/AOTBuiltinFeatures.h"), "utf8");
  if (!features.includes(`macro(JIT, ${FEATURE_JIT})`))
    throw Error("the engine numbers its removable features differently");
}

interface Scan {
  inventory: Inventory;
  archive: Uint8Array;
  members: Member[];
  replaced: Map<number, Uint8Array>;
}

function scan(webkit: string): Scan {
  const engine = pinnedEngine();
  checkPrebuilt(webkit, engine);
  const archive: Uint8Array = readFileSync(join(webkit, "lib/JavaScriptCore.lib"));
  const archiveSha256 = createHash("sha256").update(archive).digest("hex");
  const list = members(archive);
  const objects = list.map((m, index) => ({ index, member: m })).filter(({ member }) => member.kind === "object");

  const entries = new Set<string>();
  const definitions = new Set<string>();
  const others = new Set<string>();
  const guards = new Map<number, Map<number, Set<string>>>();
  for (const { index, member } of objects) {
    const bytes = archive.subarray(member.start, member.start + member.size);
    if (buffer(bytes).indexOf(GUARD_SECTION) < 0) continue;
    for (const record of removableEntries(new CoffObject(bytes))) {
      if (record.feature !== FEATURE_JIT) {
        others.add(record.symbol);
        continue;
      }
      entries.add(record.symbol);
      if (record.origin === ORIGIN_DEFINITION) definitions.add(record.symbol);
      else {
        const sections = guards.get(index) ?? new Map<number, Set<string>>();
        guards.set(index, sections);
        const names = sections.get(record.guardSection) ?? new Set<string>();
        sections.set(record.guardSection, names);
        names.add(record.symbol);
      }
    }
  }
  if (!entries.size) throw Error("the archive records no JIT entry points");
  for (const name of entries) {
    if (others.has(name)) throw Error(`${name} is an entry point of the JIT and of another removable feature`);
    if (name.length <= 8) throw Error(`${name}: short symbol names are not searched for`);
  }

  const counts = new Map<string, { definition: number; callSite: number }>();
  for (const name of entries) counts.set(name, { definition: 0, callSite: 0 });
  const needles = [...entries].map((name) => Buffer.from(`${name}\0`, "latin1"));
  const replaced = new Map<number, Uint8Array>();
  for (const { index, member } of objects) {
    const bytes = archive.subarray(member.start, member.start + member.size);
    const object = new CoffObject(bytes);
    const strings = buffer(bytes.subarray(object.strings));
    if (!needles.some((needle) => strings.indexOf(needle) >= 0)) continue;
    const out = redirect(object, entries, definitions, guards.get(index), (name, kind) => counts.get(name)![kind]++);
    if (out) replaced.set(index, out);
  }
  const sorted = [...counts].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  const total = (kind: Kind) => sorted.reduce((n, [, c]) => n + c[kind], 0);
  return {
    inventory: {
      schema: "skathi.no-jit-inventory",
      version: 1,
      engine,
      archiveSha256,
      feature: { name: "JIT", id: FEATURE_JIT },
      references: { definition: total("definition"), callSite: total("callSite") },
      entries: Object.fromEntries(sorted),
    },
    archive,
    members: list,
    replaced,
  };
}

function difference(committed: Inventory, found: Inventory): string[] {
  const lines: string[] = [];
  for (const key of ["engine", "archiveSha256"] as const) {
    if (committed[key] !== found[key]) lines.push(`${key}: ${committed[key]} -> ${found[key]}`);
  }
  const names = new Set([...Object.keys(committed.entries), ...Object.keys(found.entries)]);
  for (const name of [...names].sort()) {
    const was = JSON.stringify(committed.entries[name] ?? null);
    const is = JSON.stringify(found.entries[name] ?? null);
    if (was !== is) lines.push(`${name}: ${was} -> ${is}`);
  }
  if (JSON.stringify(committed.references) !== JSON.stringify(found.references))
    lines.push(`references: ${JSON.stringify(committed.references)} -> ${JSON.stringify(found.references)}`);
  return lines;
}

function checkInventory(found: Inventory): void {
  const committed = JSON.parse(readFileSync(INVENTORY, "utf8")) as Inventory;
  const lines = difference(committed, found);
  if (lines.length)
    throw Error(
      `the pinned JavaScriptCore.lib does not match scripts/aot/jit-entries.json:\n  ${lines.join("\n  ")}\n` +
        "Review the JIT entry points of the new engine, then run bun scripts/aot/no-jit.ts --write.",
    );
}

const sizeField = (size: number) => Buffer.from(String(size).padEnd(10, " "), "latin1");

function writeArchive({ archive, members: list, replaced }: Scan, path: string): string {
  const moved = new Map<number, number>();
  let offset = 8;
  for (const [index, member] of list.entries()) {
    moved.set(member.header, offset);
    const size = replaced.get(index)?.length ?? member.size;
    offset += 60 + size + (size % 2);
  }
  const remap = (old: number) => {
    const to = moved.get(old);
    if (to === undefined) throw Error(`a linker member names offset ${old}, which is no member`);
    return to;
  };
  const hash = createHash("sha256");
  const partial = `${path}.partial`;
  const fd = openSync(partial, "w");
  const put = (bytes: Uint8Array) => {
    hash.update(bytes);
    for (let done = 0; done < bytes.length;) done += writeSync(fd, bytes, done, bytes.length - done);
  };
  try {
    put(archive.subarray(0, 8));
    for (const [index, member] of list.entries()) {
      let body: Uint8Array = replaced.get(index) ?? archive.subarray(member.start, member.start + member.size);
      if (member.kind === "first-linker") {
        body = Uint8Array.from(body);
        const data = view(body);
        const count = data.getUint32(0, false);
        for (let i = 0; i < count; i++) data.setUint32(4 + 4 * i, remap(data.getUint32(4 + 4 * i, false)), false);
      } else if (member.kind === "second-linker") {
        body = Uint8Array.from(body);
        const data = view(body);
        const count = data.getUint32(0, true);
        for (let i = 0; i < count; i++) data.setUint32(4 + 4 * i, remap(data.getUint32(4 + 4 * i, true)), true);
      }
      const header = Uint8Array.from(archive.subarray(member.header, member.start));
      header.set(sizeField(body.length), 48);
      put(header);
      put(body);
      if (body.length % 2) put(Uint8Array.of(0x0a));
    }
  } finally {
    closeSync(fd);
  }
  renameSync(partial, path);
  return hash.digest("hex");
}

/** Reads the written archive back: the same members in the same order, linker members pointing at them, and only the redirect left. */
function verifyArchive(path: string, scan: Scan, entries: Set<string>): void {
  const archive: Uint8Array = readFileSync(path);
  const list = members(archive);
  if (list.length !== scan.members.length || list.some((m, i) => m.name !== scan.members[i]!.name))
    throw Error("the written archive has other members than the pinned one");
  const headers = new Set(list.map((m) => m.header));
  for (const member of list.filter((m) => m.kind === "first-linker")) {
    const data = view(archive.subarray(member.start, member.start + member.size));
    for (let i = 0; i < data.getUint32(0, false); i++) {
      if (!headers.has(data.getUint32(4 + 4 * i, false)))
        throw Error("the written symbol table points between members");
    }
  }
  for (const index of scan.replaced.keys()) {
    const member = list[index]!;
    const object = new CoffObject(archive.subarray(member.start, member.start + member.size));
    const redirected = object.symbolCount - 1;
    if (object.symbolName(redirected) !== REDIRECT) throw Error(`${member.name}: the redirect symbol is missing`);
    for (const section of object.sections) {
      if (!REFERENCING_SECTIONS.has(section.name.split("$", 1)[0]!)) continue;
      for (const relocation of object.relocations(section)) {
        if (relocation.symbol !== redirected && entries.has(object.symbolName(relocation.symbol)))
          throw Error(`${member.name}: a reference to ${object.symbolName(relocation.symbol)} survived`);
      }
    }
  }
}

if (import.meta.main) {
  const { values } = parseArgs({
    options: {
      out: { type: "string" },
      check: { type: "boolean" },
      write: { type: "boolean" },
      webkit: { type: "string" },
    },
  });
  if ([values.out !== undefined, values.check, values.write].filter(Boolean).length !== 1) {
    console.error("no-jit.ts --out <dir> | --check | --write  [--webkit <bun-webkit prebuilt>]");
    process.exit(2);
  }
  if (process.platform !== "win32" || process.arch !== "x64")
    throw Error("the aot-nojit archive is made on Windows x64, from the Windows prebuilt");
  const webkit = resolve(values.webkit ?? join(SDK, "vendor/redjs/prebuilt/bun-webkit"));
  const t0 = performance.now();
  const result = scan(webkit);
  const { inventory } = result;
  const summary = `${Object.keys(inventory.entries).length} JIT entry points, ${inventory.references.definition} definition and ${inventory.references.callSite} call-site references`;
  if (values.write) {
    writeFileSync(INVENTORY, JSON.stringify(inventory, null, 2) + "\n");
    console.log(`[no-jit] ${INVENTORY}: ${summary}`);
    process.exit(0);
  }
  checkInventory(inventory);
  if (values.check) {
    console.log(`[no-jit] jit-entries.json matches ${inventory.engine}: ${summary}`);
    process.exit(0);
  }
  const out = resolve(values.out!);
  mkdirSync(out, { recursive: true });
  rmSync(join(out, RECEIPT), { force: true });
  const path = join(out, ARCHIVE);
  const sha256 = writeArchive(result, path);
  verifyArchive(path, result, new Set(Object.keys(inventory.entries)));
  writeFileSync(
    join(out, RECEIPT),
    `redjs.no-jit.v1\nwebkit=${inventory.engine}\nsource=${inventory.archiveSha256}\nsha256=${sha256}\nentry=${REDIRECT}\n`,
  );
  console.log(
    `[no-jit] ${path}: ${summary} redirected in ${result.replaced.size} objects, ${((performance.now() - t0) / 1000).toFixed(0)} s`,
  );
}
