/** The JavaScriptCore archive of the aot-nojit workload (Windows x64): the pinned archive with every
 * reference to an entry point of the JIT and of WebAssembly redirected to a stub redjs defines, so the
 * linker leaves both out. JSC lists those entry points itself, in its .bun_removable_entry_points records;
 * jit-entries.json and webassembly-entries.json are what they were at the reviewed pin, and every mode
 * fails when the pinned archive says otherwise.
 *
 * An entry point the engine marks Abort goes to redjs_removed_<feature>_entry, which aborts. A built-in
 * the engine marks Throw (WebAssembly's constructors and functions) goes to a host function that throws;
 * the built-ins a static table holds carry no record, and AOTBuiltinFeatures.h names them instead. A void
 * function the engine skips once the feature is removed (ReturnDefault) goes to a stub that returns: the
 * engine still calls it directly while nothing is cut out by address range (isTrimmed is 0).
 *
 *   bun scripts/aot/no-jit.ts --out <dir>   <dir>/JavaScriptCoreNoJIT.lib and the receipt redjs's no-jit feature reads
 *   bun scripts/aot/no-jit.ts --check       fails unless the inventories match the pinned archive
 *   bun scripts/aot/no-jit.ts --write       rewrites the inventories from the pinned archive, to review after a pin change
 *   --remove JIT                            only the JIT (default: JIT,WebAssembly)
 *   --remove JIT,WebAssembly,Interpreter    qualification archive for the experimental no-interpreter Cargo feature
 */
import { createHash } from "node:crypto";
import { closeSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, writeFileSync, writeSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const SDK = resolve(import.meta.dir, "../..");
export const ARCHIVE = "JavaScriptCoreNoJIT.lib";
export const RECEIPT = ".redjs-no-jit";

export interface RemovedFeature {
  name: string;
  id: number;
  /** What every Abort entry point is redirected to. */
  abort: string;
  /** What every Throw built-in is redirected to: a host function that throws. */
  throws?: string;
  /** What every void ReturnDefault entry point is redirected to: a function that returns. */
  skips?: string;
  inventory: string;
}

/** What the aot-nojit workload removes; the ids are AOTBuiltinFeatures.h's. */
export const REMOVED: readonly RemovedFeature[] = [
  { name: "JIT", id: 0, abort: "redjs_removed_jit_entry", inventory: "jit-entries.json" },
  {
    name: "WebAssembly",
    id: 4,
    abort: "redjs_removed_wasm_entry",
    throws: "redjs_removed_wasm_host_function",
    skips: "redjs_removed_wasm_skip",
    inventory: "webassembly-entries.json",
  },
];

export const TRIMMABLE: readonly RemovedFeature[] = [
  ...REMOVED,
  {
    name: "Interpreter",
    id: 1,
    abort: "redjs_removed_interpreter_entry",
    throws: "redjs_removed_interpreter_host_function",
    skips: "redjs_removed_interpreter_skip",
    inventory: "interpreter-entries.json",
  },
];

const GUARD_SECTION = ".bun_removable_entry_points";
const RECORD_BYTES = 24;
const STUB_ABORT = 0;
const STUB_THROW = 1;
const STUB_RETURN_DEFAULT = 3;
const ORIGIN_NATIVE_CODE = 0;
const ORIGIN_DEFINITION = 2;
const ORIGIN_BUILTIN_NAMED_BY_ENGINE = 3;
/** `EncodedJSValue (JSGlobalObject*, CallFrame*)` in namespace JSC: the shape of the throwing stub. */
const HOST_FUNCTION = /^\?\w+@JSC@@YA_JPEAVJSGlobalObject@1@PEAVCallFrame@1@@Z$/;
/** A free function returning void, which a bare return stands in for whatever its arguments. */
const VOID_FUNCTION = /^\?\w+(@\w+)*@@YAX/;
// MSVC spells the static void RemovableBody::run for VM::createBytecodeIntrinsicRegistry with its lambda type.
const VOID_REGISTRY_BODY =
  "?run@?$RemovableBody@$00$02XV<lambda_19>@?0??createBytecodeIntrinsicRegistry@VM@JSC@@AEAAXXZ@@JSC@@SAXAEAV<lambda_19>@?0??createBytecodeIntrinsicRegistry@VM@2@AEAAXXZ@@Z";
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
  feature: { name: string; id: number };
  references: References;
  entries: Record<string, References>;
  /** The entries that throw instead of aborting. */
  throwing?: string[];
  /** The entries that return without doing anything instead of aborting. */
  skipped?: string[];
}

/** `table`: from a static table of built-ins, counted only when there are any. */
interface References {
  definition: number;
  callSite: number;
  table?: number;
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
  stub: number;
  origin: number;
  symbol: string;
  guardSection: number;
}

function removableEntries(object: CoffObject, removed: ReadonlyMap<number, RemovedFeature>): EntryRecord[] {
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
        stub: object.bytes[at + 18]!,
        origin: object.bytes[at + 19]!,
        symbol: object.symbolName(entry),
        guardSection: object.symbolSection(guard),
      };
      const feature = removed.get(record.feature);
      if (feature) {
        const { stub, origin, symbol } = record;
        const aborts = stub === STUB_ABORT && (origin === ORIGIN_NATIVE_CODE || origin === ORIGIN_DEFINITION);
        const skips =
          stub === STUB_RETURN_DEFAULT &&
          origin === ORIGIN_NATIVE_CODE &&
          feature.skips !== undefined &&
          (VOID_FUNCTION.test(symbol) || (feature.name === "Interpreter" && symbol === VOID_REGISTRY_BODY));
        const throws =
          stub === STUB_THROW &&
          origin === ORIGIN_BUILTIN_NAMED_BY_ENGINE &&
          feature.throws !== undefined &&
          HOST_FUNCTION.test(symbol);
        if (!aborts && !skips && !throws)
          throw Error(
            `${symbol}: a ${feature.name} entry with stub ${stub} and origin ${origin}, which is not handled`,
          );
        if (origin !== ORIGIN_DEFINITION && record.guardSection < 1)
          throw Error(`${symbol}: the guard is not in a section`);
        if (object.data.getBigUint64(at + 8, true) !== 0n)
          throw Error(`${symbol}: interior entry points are unsupported`);
      }
      records.push(record);
    }
  }
  return records;
}

type Kind = "definition" | "callSite" | "table";

/** `object` with each guarded reference to an entry pointing at a new undefined symbol, its redirect; null when it has none. */
function redirect(
  object: CoffObject,
  redirects: ReadonlyMap<string, string>,
  definitions: Set<string>,
  tables: Set<string>,
  guards: Map<number, Set<string>> | undefined,
  count: (name: string, kind: Kind) => void,
): Uint8Array | null {
  const targets = new Map<number, string>();
  for (let index = 0; index < object.symbolCount;) {
    const at = object.symbols + index * object.symbolSize;
    const storage = object.bytes[at + object.symbolSize - 2];
    if (storage === STORAGE_EXTERNAL || storage === STORAGE_STATIC) {
      const name = object.symbolName(index);
      if (redirects.has(name)) targets.set(index, name);
    }
    index += 1 + object.bytes[at + object.symbolSize - 1]!;
  }
  if (!targets.size) return null;
  const patches: { at: number; to: string }[] = [];
  for (const section of object.sections) {
    if (!REFERENCING_SECTIONS.has(section.name.split("$", 1)[0]!)) continue;
    for (const relocation of object.relocations(section)) {
      const name = targets.get(relocation.symbol);
      if (name === undefined) continue;
      let kind: Kind;
      if (definitions.has(name)) kind = "definition";
      else if (guards?.get(section.number)?.has(name)) kind = "callSite";
      else if (tables.has(name) && !section.name.startsWith(".text")) kind = "table";
      else throw Error(`an unguarded reference to the removed entry ${name}`);
      if (relocation.type !== REL_ADDR64 && relocation.type !== REL_REL32)
        throw Error(`an unexpected relocation (type ${relocation.type}) to ${name}`);
      const width = relocation.type === REL_ADDR64 ? 8 : 4;
      const site = object.bytes.subarray(section.raw + relocation.address, section.raw + relocation.address + width);
      if (site.some((b) => b !== 0)) throw Error(`an interior reference to ${name}`);
      patches.push({ at: relocation.at, to: redirects.get(name)! });
      count(name, kind);
    }
  }
  if (!patches.length) return null;

  // The symbols and their names go at the end of the symbol and string tables: nothing may follow them.
  const stringsSize = object.data.getUint32(object.strings, true);
  for (const section of object.sections) {
    if (
      section.raw + section.size > object.symbols ||
      section.relocations + 10 * section.relocationCount > object.symbols
    )
      throw Error("a section lies after the symbol table");
  }
  if (object.strings + stringsSize !== object.bytes.length) throw Error("data after the string table");
  const added = [...new Set(patches.map((p) => p.to))];
  const names = added.map((name) => new TextEncoder().encode(`${name}\0`));
  const namesSize = names.reduce((n, name) => n + name.length, 0);
  const symbolsSize = added.length * object.symbolSize;
  const out = new Uint8Array(object.bytes.length + symbolsSize + namesSize);
  out.set(object.bytes.subarray(0, object.strings));
  let nameAt = stringsSize;
  for (const [index, name] of names.entries()) {
    const start = object.strings + index * object.symbolSize;
    const symbol = out.subarray(start, start + object.symbolSize);
    const record = view(symbol);
    record.setUint32(4, nameAt, true);
    record.setUint16(object.big ? 16 : 14, 0x20, true);
    symbol[object.symbolSize - 2] = STORAGE_EXTERNAL;
    nameAt += name.length;
  }
  const strings = object.strings + symbolsSize;
  out.set(object.bytes.subarray(object.strings), strings);
  let at = strings + stringsSize;
  for (const name of names) {
    out.set(name, at);
    at += name.length;
  }
  const data = view(out);
  data.setUint32(strings, stringsSize + namesSize, true);
  data.setUint32(object.symbolsField + 4, object.symbolCount + added.length, true);
  for (const patch of patches) data.setUint32(patch.at + 4, object.symbolCount + added.indexOf(patch.to), true);
  return out;
}

function pinnedEngine(): string {
  const pins = readFileSync(join(SDK, "vendor/redjs/redjs/pins.txt"), "utf8");
  const engine = /^webkit\.version\s*=\s*(\S+)/m.exec(pins)?.[1];
  if (!engine) throw Error("vendor/redjs/redjs/pins.txt has no webkit.version");
  return engine;
}

function checkPrebuilt(webkit: string, engine: string, removed: readonly RemovedFeature[]): string {
  const configuration = readFileSync(join(webkit, "include/cmakeconfig.h"), "utf8");
  if (!configuration.includes(`#define BUN_WEBKIT_VERSION "${engine}"`))
    throw Error(`${webkit} is not the pinned prebuilt ${engine}; run vendor/redjs/setup.ps1`);
  const features = readFileSync(join(webkit, "include/JavaScriptCore/AOTBuiltinFeatures.h"), "utf8");
  for (const { name, id } of removed) {
    if (!features.includes(`macro(${name}, ${id})`))
      throw Error("the engine numbers its removable features differently");
  }
  return features;
}

/** The host functions AOTBuiltinFeatures.h names as entry points of a removed feature, by mangled name. */
function guardedBuiltins(header: string, removed: readonly RemovedFeature[]): Map<string, RemovedFeature> {
  const start = header.indexOf("#define FOR_EACH_GUARDED_ENGINE_SYMBOL(macro)");
  if (start < 0) throw Error("AOTBuiltinFeatures.h lists no guarded engine symbols");
  const end = header.indexOf("\n\n", start);
  const list = header.slice(start, end < 0 ? undefined : end);
  const found = new Map<string, RemovedFeature>();
  for (const [, symbol, name, stub] of list.matchAll(/macro\((\w+), (\w+), (\w+), \d+\)/g)) {
    const feature = removed.find((f) => f.name === name);
    if (!feature) continue;
    if (stub !== "Throw" || !feature.throws)
      throw Error(`${symbol}: a guarded ${name} built-in with stub ${stub}, which is not handled`);
    found.set(`?${symbol}@JSC@@YA_JPEAVJSGlobalObject@1@PEAVCallFrame@1@@Z`, feature);
  }
  return found;
}

interface Scan {
  inventories: Inventory[];
  archive: Uint8Array;
  members: Member[];
  replaced: Map<number, Uint8Array>;
  redirects: Map<string, string>;
}

function scan(webkit: string, removed: readonly RemovedFeature[]): Scan {
  const engine = pinnedEngine();
  const header = checkPrebuilt(webkit, engine, removed);
  const archive: Uint8Array = readFileSync(join(webkit, "lib/JavaScriptCore.lib"));
  const archiveSha256 = createHash("sha256").update(archive).digest("hex");
  const list = members(archive);
  const objects = list.map((m, index) => ({ index, member: m })).filter(({ member }) => member.kind === "object");
  const byId = new Map(removed.map((f) => [f.id, f]));

  const featureOf = new Map<string, RemovedFeature>();
  const throwing = new Set<string>();
  const aborting = new Set<string>();
  const skipping = new Set<string>();
  const definitions = new Set<string>();
  const others = new Set<string>();
  const guards = new Map<number, Map<number, Set<string>>>();
  for (const { index, member } of objects) {
    const bytes = archive.subarray(member.start, member.start + member.size);
    if (buffer(bytes).indexOf(GUARD_SECTION) < 0) continue;
    for (const record of removableEntries(new CoffObject(bytes), byId)) {
      const feature = byId.get(record.feature);
      if (!feature) {
        others.add(record.symbol);
        continue;
      }
      const owner = featureOf.get(record.symbol);
      if (owner && owner !== feature)
        throw Error(`${record.symbol} is an entry point of ${owner.name} and of ${feature.name}`);
      featureOf.set(record.symbol, feature);
      (record.stub === STUB_THROW ? throwing : record.stub === STUB_RETURN_DEFAULT ? skipping : aborting).add(
        record.symbol,
      );
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
  const tables = new Set<string>();
  for (const [name, feature] of guardedBuiltins(header, removed)) {
    const owner = featureOf.get(name);
    if (owner && owner !== feature) throw Error(`${name} is an entry point of ${owner.name} and of ${feature.name}`);
    featureOf.set(name, feature);
    throwing.add(name);
    tables.add(name);
  }
  for (const feature of removed) {
    if (![...featureOf.values()].includes(feature)) throw Error(`the archive records no ${feature.name} entry points`);
  }
  for (const name of featureOf.keys()) {
    if (others.has(name))
      throw Error(`${name} is an entry point of ${featureOf.get(name)!.name} and of a feature that stays`);
    if (throwing.has(name) && aborting.has(name)) throw Error(`${name} is recorded both to throw and to abort`);
    if (throwing.has(name) && definitions.has(name))
      throw Error(`${name}: a throwing entry recorded at its definition`);
    if (skipping.has(name) && (throwing.has(name) || definitions.has(name)))
      throw Error(`${name}: a skipped entry that also throws or is recorded at its definition`);
    if (name.length <= 8) throw Error(`${name}: short symbol names are not searched for`);
  }
  const redirects = new Map<string, string>();
  for (const [name, feature] of featureOf)
    redirects.set(name, throwing.has(name) ? feature.throws! : skipping.has(name) ? feature.skips! : feature.abort);

  const counts = new Map<string, References>();
  for (const name of featureOf.keys()) counts.set(name, { definition: 0, callSite: 0 });
  const needles = [...featureOf.keys()].map((name) => Buffer.from(`${name}\0`, "latin1"));
  const replaced = new Map<number, Uint8Array>();
  for (const { index, member } of objects) {
    const bytes = archive.subarray(member.start, member.start + member.size);
    const object = new CoffObject(bytes);
    const strings = buffer(bytes.subarray(object.strings));
    if (!needles.some((needle) => strings.indexOf(needle) >= 0)) continue;
    const out = redirect(object, redirects, definitions, tables, guards.get(index), (name, kind) => {
      const c = counts.get(name)!;
      c[kind] = (c[kind] ?? 0) + 1;
    });
    if (out) replaced.set(index, out);
  }
  for (const [name, c] of counts) {
    if (!c.definition && !c.callSite && !c.table)
      throw Error(`no reference to ${name} was found: the engine names an entry point the archive lacks`);
  }
  const inventories = removed.map((feature): Inventory => {
    const sorted = [...counts]
      .filter(([name]) => featureOf.get(name) === feature)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    const total = (kind: Kind) => sorted.reduce((n, [, c]) => n + (c[kind] ?? 0), 0);
    const table = total("table");
    const thrown = sorted.map(([name]) => name).filter((name) => throwing.has(name));
    const skipped = sorted.map(([name]) => name).filter((name) => skipping.has(name));
    return {
      schema: "skathi.no-jit-inventory",
      version: 1,
      engine,
      archiveSha256,
      feature: { name: feature.name, id: feature.id },
      references: { definition: total("definition"), callSite: total("callSite"), ...(table ? { table } : {}) },
      entries: Object.fromEntries(sorted),
      ...(thrown.length ? { throwing: thrown } : {}),
      ...(skipped.length ? { skipped } : {}),
    };
  });
  return { inventories, archive, members: list, replaced, redirects };
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
  for (const key of ["references", "throwing", "skipped"] as const) {
    if (JSON.stringify(committed[key]) !== JSON.stringify(found[key]))
      lines.push(`${key}: ${JSON.stringify(committed[key])} -> ${JSON.stringify(found[key])}`);
  }
  return lines;
}

export const inventoryPath = (feature: RemovedFeature) => join(import.meta.dir, feature.inventory);

function checkInventory(feature: RemovedFeature, found: Inventory): void {
  const committed = JSON.parse(readFileSync(inventoryPath(feature), "utf8")) as Inventory;
  const lines = difference(committed, found);
  if (lines.length)
    throw Error(
      `the pinned JavaScriptCore.lib does not match scripts/aot/${feature.inventory}:\n  ${lines.join("\n  ")}\n` +
        `Review the ${feature.name} entry points of the new engine, then run bun scripts/aot/no-jit.ts --write.`,
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

/** Reads the written archive back: the same members in the same order, linker members pointing at them, and only redirects left. */
function verifyArchive(path: string, scan: Scan): void {
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
  const stubs = new Set(scan.redirects.values());
  for (const index of scan.replaced.keys()) {
    const member = list[index]!;
    const object = new CoffObject(archive.subarray(member.start, member.start + member.size));
    for (const section of object.sections) {
      if (!REFERENCING_SECTIONS.has(section.name.split("$", 1)[0]!)) continue;
      for (const relocation of object.relocations(section)) {
        const name = object.symbolName(relocation.symbol);
        if (scan.redirects.has(name)) throw Error(`${member.name}: a reference to ${name} survived`);
        if (stubs.has(name) && object.symbolSection(relocation.symbol) !== 0)
          throw Error(`${member.name}: ${name} is defined in the engine archive`);
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
      remove: { type: "string" },
    },
  });
  if ([values.out !== undefined, values.check, values.write].filter(Boolean).length !== 1) {
    console.error(
      "no-jit.ts --out <dir> | --check | --write  [--remove JIT[,WebAssembly][,Interpreter]] [--webkit <bun-webkit prebuilt>]",
    );
    process.exit(2);
  }
  if (process.platform !== "win32" || process.arch !== "x64")
    throw Error("the aot-nojit archive is made on Windows x64, from the Windows prebuilt");
  const names = values.remove?.split(",").map((n) => n.trim()) ?? REMOVED.map((f) => f.name);
  const removed = TRIMMABLE.filter((f) => names.includes(f.name));
  for (const name of names)
    if (!TRIMMABLE.some((f) => f.name === name))
      throw Error(`--remove: ${name} is not one of ${TRIMMABLE.map((f) => f.name).join(", ")}`);
  if (!names.includes("JIT")) throw Error("--remove: the archive always leaves out the JIT");
  const webkit = resolve(values.webkit ?? join(SDK, "vendor/redjs/prebuilt/bun-webkit"));
  const t0 = performance.now();
  const result = scan(webkit, removed);
  const summaries = result.inventories.map(
    (i) =>
      `${i.feature.name}: ${Object.keys(i.entries).length} entry points (${i.throwing?.length ?? 0} throw, ${i.skipped?.length ?? 0} skipped), ` +
      `${i.references.definition} definition, ${i.references.callSite} call-site and ${i.references.table ?? 0} table references`,
  );
  if (values.write) {
    for (const [index, feature] of removed.entries()) {
      writeFileSync(inventoryPath(feature), JSON.stringify(result.inventories[index], null, 2) + "\n");
      console.log(`[no-jit] ${inventoryPath(feature)}: ${summaries[index]}`);
    }
    process.exit(0);
  }
  for (const [index, feature] of removed.entries()) checkInventory(feature, result.inventories[index]!);
  if (values.check) {
    console.log(`[no-jit] the inventories match ${result.inventories[0]!.engine}:\n  ${summaries.join("\n  ")}`);
    process.exit(0);
  }
  const out = resolve(values.out!);
  mkdirSync(out, { recursive: true });
  rmSync(join(out, RECEIPT), { force: true });
  const path = join(out, ARCHIVE);
  const sha256 = writeArchive(result, path);
  verifyArchive(path, result);
  const lines = [
    "redjs.no-jit.v2",
    `webkit=${result.inventories[0]!.engine}`,
    `source=${result.inventories[0]!.archiveSha256}`,
    `sha256=${sha256}`,
    `removed=${removed.map((f) => f.name).join(",")}`,
    ...removed.map((f) => `entry.${f.name}=${f.abort}`),
    ...removed.flatMap((f) => (f.throws ? [`host-function.${f.name}=${f.throws}`] : [])),
    ...removed.flatMap((f) => (f.skips ? [`skip.${f.name}=${f.skips}`] : [])),
  ];
  writeFileSync(join(out, RECEIPT), `${lines.join("\n")}\n`);
  console.log(
    `[no-jit] ${path}: ${summaries.join("; ")}; redirected in ${result.replaced.size} objects, ${((performance.now() - t0) / 1000).toFixed(0)} s`,
  );
}
