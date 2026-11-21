import { createHash } from "node:crypto";
import { closeSync, openSync, readFileSync, renameSync, rmSync, writeSync } from "node:fs";
import { join } from "node:path";
import type { Inventory, RemovedFeature } from "./no-jit";

export const MACHO_NO_JIT_ARCHIVE = "libJavaScriptCoreNoJIT.a";

const GUARD_SECTION = "__bun_entries";
const RECORD_BYTES = 24;
const WASM_INSTALLER = "__ZN3JSC5LLIntL29installWebAssemblyInterpreterEPPvS2_S2_";
const HOST_FUNCTION = /^__ZN3JSC(?:L)?\d+\w+EPNS_14JSGlobalObjectEPNS_9CallFrameE$/;
const REFERENCING_METADATA = "__llvm_addrsig";
const EXTERNAL_SYMBOL = 0x08000000;
const SYMBOL_INDEX = 0x00ffffff;
const INSTRUCTION_ADDENDS = new Map([
  [2, 0x03ffffff], // ARM64_RELOC_BRANCH26
  [3, 0x60ffffe0], // ARM64_RELOC_PAGE21
  [4, 0x003ffc00], // ARM64_RELOC_PAGEOFF12
  [5, 0x60ffffe0], // ARM64_RELOC_GOT_LOAD_PAGE21
  [6, 0x003ffc00], // ARM64_RELOC_GOT_LOAD_PAGEOFF12
]);

interface Member {
  header: number;
  prefix: number;
  start: number;
  end: number;
  name: string;
}

function members(archive: Buffer): Member[] {
  if (archive.toString("ascii", 0, 8) !== "!<arch>\n") throw Error("expected a BSD archive");
  const list: Member[] = [];
  for (let header = 8; header < archive.length;) {
    const prefix = header + 60;
    const nameBytes = Number(archive.toString("ascii", header + 3, header + 16).trim());
    const size = Number(archive.toString("ascii", header + 48, header + 58).trim());
    if (
      archive.toString("ascii", header, header + 3) !== "#1/" ||
      archive.toString("ascii", header + 58, prefix) !== "`\n" ||
      !Number.isInteger(nameBytes) ||
      nameBytes <= 0 ||
      !Number.isInteger(size) ||
      size < nameBytes ||
      prefix + size > archive.length
    )
      throw Error(`invalid BSD archive member at ${header}`);
    const start = prefix + nameBytes;
    const end = prefix + size;
    const name = archive.toString("utf8", prefix, start).replace(/\0+$/, "");
    list.push({ header, prefix, start, end, name });
    header = end + (size % 2);
  }
  if (list[0]?.name !== "__.SYMDEF SORTED" && list[0]?.name !== "__.SYMDEF")
    throw Error("BSD archive is missing its 32-bit symbol index");
  return list;
}

interface Section {
  index: number;
  name: string;
  segment: string;
  address: number;
  size: number;
  offset: number;
  relocations: number;
  count: number;
}

interface Symbol {
  name: string;
  section: number;
  value: number;
}

interface Relocation {
  at: number;
  offset: number;
  info: number;
  index: number;
  external: boolean;
  type: number;
  width: number;
}

class MachOObject {
  readonly sections: Section[] = [];
  readonly symbols: Symbol[] = [];
  readonly atoms: Map<number, string[]>[] = [];
  readonly starts: number[][] = [];
  readonly table: number;
  readonly dynamic: number;
  readonly entries: number;
  readonly strings: number;
  readonly stringsSize: number;

  constructor(readonly bytes: Buffer) {
    if (bytes.readUInt32LE(0) !== 0xfeedfacf || bytes.readUInt32LE(4) !== 0x0100000c || bytes.readUInt32LE(12) !== 1)
      throw Error("expected an ARM64 Mach-O object");
    let table = 0,
      dynamic = 0;
    for (let n = 0, at = 32; n < bytes.readUInt32LE(16); n++) {
      const command = bytes.readUInt32LE(at);
      if (command === 2) table = at;
      if (command === 0xb) dynamic = at;
      if (command === 0x19) {
        const count = bytes.readUInt32LE(at + 64);
        for (let index = 0; index < count; index++) {
          const field = at + 72 + index * 80;
          this.sections.push({
            index: this.sections.length + 1,
            name: bytes.toString("ascii", field, field + 16).replace(/\0+$/, ""),
            segment: bytes.toString("ascii", field + 16, field + 32).replace(/\0+$/, ""),
            address: this.number(field + 32),
            size: this.number(field + 40),
            offset: bytes.readUInt32LE(field + 48),
            relocations: bytes.readUInt32LE(field + 56),
            count: bytes.readUInt32LE(field + 60),
          });
        }
      }
      at += bytes.readUInt32LE(at + 4);
    }
    this.table = table;
    this.dynamic = dynamic;
    this.entries = table ? bytes.readUInt32LE(table + 8) : 0;
    this.strings = table ? bytes.readUInt32LE(table + 16) : 0;
    this.stringsSize = table ? bytes.readUInt32LE(table + 20) : 0;
    for (const section of this.sections) this.atoms[section.index] = new Map();
    const count = table ? bytes.readUInt32LE(table + 12) : 0;
    for (let index = 0; index < count; index++) {
      const at = this.entries + index * 16;
      const start = this.strings + bytes.readUInt32LE(at);
      const end = bytes.indexOf(0, start);
      if (end < 0 || end >= this.strings + this.stringsSize) throw Error("unterminated Mach-O symbol name");
      const name = bytes.toString("utf8", start, end);
      const type = bytes[at + 4]!;
      const section = bytes[at + 5]!;
      const value = this.number(at + 8);
      this.symbols.push({ name, section, value });
      if ((type & 0xee) !== 0xe || bytes.readUInt16LE(at + 6) & 0x200 || /^ltmp\d+$/.test(name)) continue;
      const atoms = this.atoms[section]!;
      const names = atoms.get(value) ?? [];
      names.push(name);
      atoms.set(value, names);
    }
    for (const section of this.sections)
      this.starts[section.index] = [...this.atoms[section.index]!.keys()].sort((a, b) => a - b);
  }

  number(at: number): number {
    const value = Number(this.bytes.readBigUInt64LE(at));
    if (!Number.isSafeInteger(value)) throw Error("Mach-O address exceeds the integer range");
    return value;
  }

  atomAt(section: number, address: number): string[] {
    const starts = this.starts[section]!;
    let lo = 0,
      hi = starts.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (starts[mid]! <= address) lo = mid + 1;
      else hi = mid;
    }
    return this.atoms[section]!.get(starts[lo - 1]!) ?? [];
  }

  *relocations(section: Section): Generator<Relocation> {
    for (let index = 0; index < section.count; index++) {
      const at = section.relocations + 8 * index;
      const info = this.bytes.readUInt32LE(at + 4);
      yield {
        at: at + 4,
        offset: this.bytes.readInt32LE(at),
        info,
        index: info & SYMBOL_INDEX,
        external: !!(info & EXTERNAL_SYMBOL),
        type: info >>> 28,
        width: 1 << ((info >>> 25) & 3),
      };
    }
  }

  redirect(patches: ReadonlyMap<Relocation, string>): Buffer {
    const { bytes, table, dynamic, entries, strings, stringsSize } = this;
    const count = this.symbols.length;
    if (
      !table ||
      entries + count * 16 !== strings ||
      strings + stringsSize !== bytes.length ||
      this.sections.some((s) => s.relocations + s.count * 8 > entries) ||
      (dynamic &&
        (bytes.readUInt32LE(dynamic + 24) + bytes.readUInt32LE(dynamic + 28) !== count ||
          bytes.subarray(dynamic + 32, dynamic + 80).some((value) => value !== 0)))
    )
      throw Error("unexpected Mach-O symbol table layout");
    const stubs = [...new Set(patches.values())];
    const names = stubs.map((stub) => Buffer.from(`${stub}\0`, "ascii"));
    const namesSize = names.reduce((total, name) => total + name.length, 0);
    const newStrings = strings + stubs.length * 16;
    const newStringsSize = (stringsSize + namesSize + 7) & ~7;
    const output = Buffer.alloc(newStrings + newStringsSize);
    bytes.copy(output, 0, 0, strings);
    bytes.copy(output, newStrings, strings);
    let nameAt = stringsSize;
    for (const [index, name] of names.entries()) {
      const at = strings + index * 16;
      output.writeUInt32LE(nameAt, at);
      output[at + 4] = 1; // N_UNDF | N_EXT
      name.copy(output, newStrings + nameAt);
      nameAt += name.length;
    }
    output.writeUInt32LE(count + stubs.length, table + 12);
    output.writeUInt32LE(newStrings, table + 16);
    output.writeUInt32LE(newStringsSize, table + 20);
    if (dynamic) output.writeUInt32LE(bytes.readUInt32LE(dynamic + 28) + stubs.length, dynamic + 28);
    for (const [rel, stub] of patches) {
      const index = count + stubs.indexOf(stub);
      if (index > SYMBOL_INDEX) throw Error("Mach-O relocation symbol index overflow");
      output.writeUInt32LE(((rel.info & ~SYMBOL_INDEX) | EXTERNAL_SYMBOL | index) >>> 0, rel.at);
    }
    return output;
  }
}

interface Entry {
  feature: RemovedFeature;
  stub: string;
  definition: boolean;
  references: Inventory["references"];
}

function recordTarget(
  object: MachOObject,
  section: Section,
  relocation: Relocation | undefined,
  exact: boolean,
): string[] {
  if (!relocation || relocation.type !== 0 || relocation.width !== 8 || relocation.info & 0x01000000)
    throw Error("unexpected pointer relocation in a Mach-O guard record");
  const value = object.number(section.offset + relocation.offset);
  if (relocation.external) {
    const symbol = object.symbols[relocation.index]!;
    if (value !== 0) throw Error(`interior reference to ${symbol.name} in a Mach-O guard record`);
    return [symbol.name];
  }
  return (exact ? object.atoms[relocation.index]?.get(value) : object.atomAt(relocation.index, value)) ?? [];
}

export function scanMachO(webkit: string, engine: string, features: readonly RemovedFeature[], header: string) {
  const archive = readFileSync(join(webkit, "lib/libJavaScriptCore.a"));
  const list = members(archive);
  const entries = new Map<string, Entry>();
  const guards = new Map<number, Map<string, Set<string>>>();
  const byId = new Map(features.map((feature) => [feature.id, feature]));
  const builtins = new Map<string, RemovedFeature>();
  const declared = header.split("#define FOR_EACH_GUARDED_ENGINE_SYMBOL(macro)")[1]?.split(/\r?\n\r?\n/)[0];
  if (!declared) throw Error("AOTBuiltinFeatures.h lists no guarded engine symbols");
  for (const [, name, key, stub] of declared.matchAll(/macro\((\w+), (\w+), (\w+), \d+\)/g)) {
    const feature = features.find((f) => f.name === key);
    if (!feature) continue;
    if (stub !== "Throw" || !feature.throws) throw Error(`unexpected ${key} built-in ${name}`);
    for (const prefix of ["__ZN3JSC", "__ZN3JSCL"]) {
      builtins.set(`${prefix}${name!.length}${name}EPNS_14JSGlobalObjectEPNS_9CallFrameE`, feature);
    }
  }
  const reserved = new Set<string>();
  for (const member of list.slice(1)) {
    const data = archive.subarray(member.start, member.end);
    if (!data.includes(GUARD_SECTION, 0, "ascii")) continue;
    const object = new MachOObject(data);
    const section = object.sections.find((s) => s.segment === "__DWARF" && s.name === GUARD_SECTION);
    if (!section) continue;
    if (section.size % RECORD_BYTES) throw Error("unknown Mach-O removable-entry record format");
    const rel = new Map([...object.relocations(section)].map((r) => [r.offset, r]));
    for (let offset = 0; offset < section.size; offset += RECORD_BYTES) {
      const at = section.offset + offset;
      const feature = byId.get(data.readUInt16LE(at + 16));
      if (!feature) {
        for (const symbol of recordTarget(object, section, rel.get(offset + 8), true)) reserved.add(symbol);
        continue;
      }
      const symbols = recordTarget(object, section, rel.get(offset + 8), true);
      const callers = recordTarget(object, section, rel.get(offset), false);
      const stub = data[at + 18]!;
      const origin = data[at + 19]!;
      if (!symbols.length || !callers.length) throw Error(`${member.name}: unresolved Mach-O guard record`);
      for (const symbol of symbols) {
        let redirect: string | undefined;
        if (symbol === WASM_INSTALLER && feature.name === "WebAssembly" && origin === 0 && (stub === 0 || stub === 3)) {
          // LLInt::initialize calls it unconditionally while the engine's range table is absent.
          redirect = feature.skips;
        } else if (stub === 0 && (origin === 0 || origin === 2)) redirect = feature.abort;
        else if (stub === 1 && origin === 3 && HOST_FUNCTION.test(symbol)) redirect = feature.throws;
        if (!redirect) throw Error(`${symbol}: unhandled ${feature.name} guard with stub ${stub} and origin ${origin}`);
        const old = entries.get(symbol);
        if (old && (old.feature !== feature || old.stub !== redirect))
          throw Error(`${symbol}: conflicting Mach-O guards`);
        entries.set(symbol, {
          feature,
          stub: redirect,
          definition: origin === 2 || (old?.definition ?? false),
          references: { definition: 0, callSite: 0 },
        });
        if (origin !== 2) {
          const byCaller = guards.get(member.header) ?? new Map<string, Set<string>>();
          for (const caller of callers) {
            const targets = byCaller.get(caller) ?? new Set<string>();
            targets.add(symbol);
            byCaller.set(caller, targets);
          }
          guards.set(member.header, byCaller);
        }
      }
    }
  }
  for (const symbol of reserved) if (entries.has(symbol)) throw Error(`${symbol} belongs to a retained feature`);
  const replaced = new Map<number, Buffer>();
  for (const member of list.slice(1)) {
    const object = new MachOObject(archive.subarray(member.start, member.end));
    const targets = new Map<number, Entry>();
    const tables = new Set<string>();
    for (const [index, { name }] of object.symbols.entries()) {
      const feature = builtins.get(name);
      if (feature) {
        const original = entries.get(name);
        if (reserved.has(name)) throw Error(`${name} belongs to a retained feature`);
        if (original && original.feature !== feature) throw Error(`${name} belongs to two removable features`);
        tables.add(name);
        if (!original)
          entries.set(name, {
            feature,
            stub: feature.throws!,
            definition: false,
            references: { definition: 0, callSite: 0 },
          });
      }
      const entry = entries.get(name);
      if (entry) targets.set(index, entry);
    }
    if (!targets.size) continue;
    const patches = new Map<Relocation, string>();
    for (const section of object.sections) {
      if (section.segment === "__DWARF" || section.name === "__compact_unwind" || section.name === "__eh_frame")
        continue;
      let explicitAddend: number | undefined;
      for (const rel of object.relocations(section)) {
        if (rel.type === 10) {
          // ARM64_RELOC_ADDEND precedes its paired relocation.
          explicitAddend = rel.offset;
          continue;
        }
        const hasAddend = explicitAddend === rel.offset;
        explicitAddend = undefined;
        if (!rel.external) {
          if (rel.type === 0 && rel.width === 8) {
            const address = object.number(section.offset + rel.offset);
            if (object.atoms[rel.index]?.get(address)?.some((name) => entries.has(name)))
              throw Error(`${member.name}: unexpected local pointer to a removed entry`);
          }
          continue;
        }
        const entry = targets.get(rel.index);
        if (!entry) continue;
        const symbol = object.symbols[rel.index]!.name;
        if (hasAddend) throw Error(`${symbol}: explicit Mach-O relocation addend`);
        if (rel.type === 0 && rel.width === 8) {
          if (object.number(section.offset + rel.offset) !== 0) throw Error(`${symbol}: interior pointer reference`);
        } else {
          const mask = INSTRUCTION_ADDENDS.get(rel.type);
          if (mask === undefined || rel.width !== 4) throw Error(`${symbol}: unexpected Mach-O relocation ${rel.type}`);
          if (object.bytes.readUInt32LE(section.offset + rel.offset) & mask)
            throw Error(`${symbol}: interior instruction reference`);
        }
        if (section.name !== REFERENCING_METADATA) {
          const callers = object.atomAt(section.index, section.address + rel.offset);
          if (entry.definition) entry.references.definition++;
          else if (callers.some((caller) => guards.get(member.header)?.get(caller)?.has(symbol)))
            entry.references.callSite++;
          else if (tables.has(symbol) && section.segment !== "__TEXT")
            entry.references.table = (entry.references.table ?? 0) + 1;
          else throw Error(`${member.name}:${section.name}: unguarded Mach-O reference to ${symbol}`);
        }
        patches.set(rel, `_${entry.stub}`);
      }
    }
    if (patches.size) replaced.set(member.header, object.redirect(patches));
  }
  for (const [symbol, { references }] of entries) {
    if (!references.definition && !references.callSite && !references.table)
      throw Error(`${symbol}: no reference to the declared removable entry`);
  }
  const archiveSha256 = createHash("sha256").update(archive).digest("hex");
  const inventories = features.map((feature): Inventory => {
    const own = [...entries]
      .filter(([, entry]) => entry.feature === feature)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (!own.length) throw Error(`the Mach-O archive records no ${feature.name} entry points`);
    const total = (kind: keyof Inventory["references"]) =>
      own.reduce((count, [, entry]) => count + (entry.references[kind] ?? 0), 0);
    const table = total("table");
    const throwing = own.filter(([, entry]) => entry.stub === feature.throws).map(([name]) => name);
    const skipped = own.filter(([, entry]) => entry.stub === feature.skips).map(([name]) => name);
    return {
      schema: "skathi.no-jit-inventory",
      version: 1,
      engine,
      archiveSha256,
      feature: { name: feature.name, id: feature.id },
      references: {
        definition: total("definition"),
        callSite: total("callSite"),
        ...(table ? { table } : {}),
      },
      entries: Object.fromEntries(own.map(([symbol, entry]) => [symbol, entry.references])),
      ...(throwing.length ? { throwing } : {}),
      ...(skipped.length ? { skipped } : {}),
    };
  });
  return { archive, list, replaced, inventories };
}

export function writeMachO({ archive, list, replaced }: ReturnType<typeof scanMachO>, path: string): string {
  const offsets = new Map<number, number>();
  let position = 8;
  for (const member of list) {
    offsets.set(member.header, position);
    const size = member.start - member.prefix + (replaced.get(member.header)?.length ?? member.end - member.start);
    position += 60 + size + (size % 2);
  }
  const index = list[0]!;
  const symbols = Buffer.from(archive.subarray(index.start, index.end));
  const records = symbols.readUInt32LE(0);
  if (records % 8 || records + 8 > symbols.length) throw Error("invalid BSD archive symbol index");
  for (let at = 8; at < records + 4; at += 8) {
    const offset = offsets.get(symbols.readUInt32LE(at));
    if (offset === undefined || offset === index.header) throw Error("BSD archive symbol points outside an object");
    symbols.writeUInt32LE(offset, at);
  }
  const hash = createHash("sha256");
  const partial = `${path}.partial`;
  const fd = openSync(partial, "w");
  try {
    const put = (bytes: Buffer) => {
      for (let at = 0; at < bytes.length;) at += writeSync(fd, bytes, at, bytes.length - at);
      hash.update(bytes);
    };
    put(archive.subarray(0, 8));
    for (const member of list) {
      const body =
        member === index ? symbols : (replaced.get(member.header) ?? archive.subarray(member.start, member.end));
      const header = Buffer.from(archive.subarray(member.header, member.start));
      const size = member.start - member.prefix + body.length;
      header.write(String(size).padEnd(10, " "), 48, 10, "ascii");
      put(header);
      put(body);
      if (size % 2) put(Buffer.from("\n"));
    }
  } catch (error) {
    closeSync(fd);
    rmSync(partial, { force: true });
    throw error;
  }
  closeSync(fd);
  renameSync(partial, path);
  return hash.digest("hex");
}
