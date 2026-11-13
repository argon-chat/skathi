// The PE side of `vn build`: a pure-TS reader for the header fields and resource-tree facts the
// stamper needs, plus bun:ffi bindings to the Win32 resource-update API that does the writing.
//
// Win32 rather than a PE writer of our own: `UpdateResourceW` rebuilds `.rsrc`, moves `.reloc` behind
// it and fixes every header field that moves. A pure-TS writer is the road to stamping on another OS.
// The reader is file-backed and page-cached: everything asked of it is in the first few KB or in
// `.rsrc`'s directory tables.

import { dlopen, FFIType, ptr, toArrayBuffer } from "bun:ffi";
import { closeSync, fstatSync, openSync, readSync, writeSync } from "node:fs";

const CHUNK = 64 * 1024;

class FileReader {
  readonly fd: number;
  readonly size: number;
  #cache = new Map<number, Uint8Array>();

  constructor(path: string) {
    this.fd = openSync(path, "r");
    this.size = fstatSync(this.fd).size;
  }

  #chunk(index: number): Uint8Array {
    let c = this.#cache.get(index);
    if (!c) {
      const start = index * CHUNK;
      const len = Math.max(0, Math.min(CHUNK, this.size - start));
      c = new Uint8Array(len);
      if (len) readSync(this.fd, c, 0, len, start);
      this.#cache.set(index, c);
    }
    return c;
  }

  bytes(off: number, len: number): Uint8Array {
    if (off < 0 || off + len > this.size) {
      throw new Error(`PE: read [${off}, +${len}) outside the ${this.size}-byte file`);
    }
    const out = new Uint8Array(len);
    let done = 0;
    while (done < len) {
      const pos = off + done;
      const c = this.#chunk(Math.floor(pos / CHUNK));
      const inChunk = pos % CHUNK;
      const n = Math.min(len - done, c.length - inChunk);
      out.set(c.subarray(inChunk, inChunk + n), done);
      done += n;
    }
    return out;
  }

  u16(off: number): number {
    const b = this.bytes(off, 2);
    return b[0]! | (b[1]! << 8);
  }

  u32(off: number): number {
    const b = this.bytes(off, 4);
    return (b[0]! | (b[1]! << 8) | (b[2]! << 16) | (b[3]! << 24)) >>> 0;
  }

  close() {
    closeSync(this.fd);
  }
}

export const RT = {
  ICON: 3,
  RCDATA: 10,
  GROUP_ICON: 14,
  VERSION: 16,
  MANIFEST: 24,
} as const;

export const SUBSYSTEM = { GUI: 2, CONSOLE: 3 } as const;

export interface Section {
  name: string;
  va: number;
  vsize: number;
  raw: number;
  rawSize: number;
}

export interface ResourceEntry {
  type: number | string;
  name: number | string;
  lang: number;
  rva: number;
  size: number;
  fileOffset: number;
}

export interface PeInfo {
  is64: boolean;
  machine: number;
  subsystem: number;
  subsystemOffset: number;
  timeDateStamp: number;
  sizeOfImage: number;
  sections: Section[];
  /** The certificate table. Its "RVA" is a FILE offset: the one data directory the loader never maps, so a signature can sit after the image. */
  security: { offset: number; size: number };
  securityEntryOffset: number;
  resources: ResourceEntry[];
  overlay: number;
}

export function readPe(path: string): PeInfo {
  const r = new FileReader(path);
  try {
    if (r.u16(0) !== 0x5a4d) throw new Error(`${path}: not an executable (no MZ header)`);
    const pe = r.u32(0x3c);
    if (r.u32(pe) !== 0x4550) throw new Error(`${path}: not a PE image`);
    const coff = pe + 4;
    const machine = r.u16(coff);
    const nSections = r.u16(coff + 2);
    const timeDateStamp = r.u32(coff + 4);
    const optSize = r.u16(coff + 16);
    const opt = coff + 20;
    const magic = r.u16(opt);
    if (magic !== 0x20b && magic !== 0x10b)
      throw new Error(`${path}: unknown optional-header magic 0x${magic.toString(16)}`);
    const is64 = magic === 0x20b;
    const sizeOfImage = r.u32(opt + 56);
    const subsystemOffset = opt + 68;
    const subsystem = r.u16(subsystemOffset);
    const ddCount = r.u32(opt + (is64 ? 108 : 92));
    const ddOff = opt + (is64 ? 112 : 96);
    const dd = (i: number) =>
      i < ddCount ? { rva: r.u32(ddOff + i * 8), size: r.u32(ddOff + i * 8 + 4) } : { rva: 0, size: 0 };

    const sections: Section[] = [];
    const secOff = opt + optSize;
    let rawEnd = 0;
    for (let i = 0; i < nSections; i++) {
      const o = secOff + i * 40;
      const nameBytes = r.bytes(o, 8);
      const name = new TextDecoder("latin1").decode(nameBytes).replace(/\0+$/, "");
      const s = { name, vsize: r.u32(o + 8), va: r.u32(o + 12), rawSize: r.u32(o + 16), raw: r.u32(o + 20) };
      sections.push(s);
      rawEnd = Math.max(rawEnd, s.raw + s.rawSize);
    }
    const rvaToOff = (rva: number): number => {
      for (const s of sections) {
        if (rva >= s.va && rva < s.va + Math.max(s.vsize, s.rawSize)) return rva - s.va + s.raw;
      }
      throw new Error(`${path}: RVA 0x${rva.toString(16)} is in no section`);
    };

    const security = dd(4);
    const resources: ResourceEntry[] = [];
    const res = dd(2);
    if (res.size) {
      const base = rvaToOff(res.rva);
      const nameAt = (off: number) => {
        const len = r.u16(base + off);
        return new TextDecoder("utf-16le").decode(r.bytes(base + off + 2, len * 2));
      };
      const walk = (dirOff: number, trail: (number | string)[]) => {
        const nNamed = r.u16(base + dirOff + 12);
        const nId = r.u16(base + dirOff + 14);
        for (let i = 0; i < nNamed + nId; i++) {
          const e = base + dirOff + 16 + i * 8;
          const nm = r.u32(e);
          const off = r.u32(e + 4);
          const label: number | string = nm & 0x80000000 ? nameAt(nm & 0x7fffffff) : nm;
          if (off & 0x80000000) {
            if (trail.length >= 2) throw new Error(`${path}: resource tree deeper than type/name/lang`);
            walk(off & 0x7fffffff, [...trail, label]);
          } else {
            const rva = r.u32(base + off);
            const size = r.u32(base + off + 4);
            resources.push({
              type: trail[0]!,
              name: trail[1]!,
              lang: typeof label === "number" ? label : 0,
              rva,
              size,
              fileOffset: rvaToOff(rva),
            });
          }
        }
      };
      walk(0, []);
    }

    return {
      is64,
      machine,
      subsystem,
      subsystemOffset,
      timeDateStamp,
      sizeOfImage,
      sections,
      security: { offset: security.rva, size: security.size },
      securityEntryOffset: ddOff + 4 * 8,
      resources,
      overlay: r.size - rawEnd,
    };
  } finally {
    r.close();
  }
}

export function readFileRange(path: string, offset: number, len: number): Uint8Array {
  const r = new FileReader(path);
  try {
    return r.bytes(offset, len);
  } finally {
    r.close();
  }
}

export function patchFile(path: string, offset: number, bytes: Uint8Array) {
  const fd = openSync(path, "r+");
  try {
    writeSync(fd, bytes, 0, bytes.length, offset);
  } finally {
    closeSync(fd);
  }
}

export function setSubsystem(path: string, subsystem: number) {
  const pe = readPe(path);
  patchFile(path, pe.subsystemOffset, new Uint8Array([subsystem & 0xff, subsystem >> 8]));
}

// Every pointer is a u64, every string a NUL-terminated UTF-16LE buffer, and an integer resource id is the id as a pointer.

const u64 = FFIType.u64;
const i32 = FFIType.i32;
const u32 = FFIType.u32;

function loadKernel32() {
  return dlopen("kernel32.dll", {
    BeginUpdateResourceW: { args: [u64, i32], returns: u64 },
    UpdateResourceW: { args: [u64, u64, u64, FFIType.u16, u64, u32], returns: i32 },
    EndUpdateResourceW: { args: [u64, i32], returns: i32 },
    GetLastError: { args: [], returns: u32 },
    LoadLibraryExW: { args: [u64, u64, u32], returns: u64 },
    FreeLibrary: { args: [u64], returns: i32 },
    FindResourceW: { args: [u64, u64, u64], returns: u64 },
    LoadResource: { args: [u64, u64], returns: u64 },
    LockResource: { args: [u64], returns: u64 },
    SizeofResource: { args: [u64, u64], returns: u32 },
  }).symbols;
}
function loadShell32() {
  return dlopen("shell32.dll", {
    ExtractIconExW: { args: [u64, i32, u64, u64, u32], returns: u32 },
  }).symbols;
}
function loadVersion() {
  return dlopen("version.dll", {
    GetFileVersionInfoSizeW: { args: [u64, u64], returns: u32 },
    GetFileVersionInfoW: { args: [u64, u32, u32, u64], returns: i32 },
    VerQueryValueW: { args: [u64, u64, u64, u64], returns: i32 },
  }).symbols;
}

let k32: ReturnType<typeof loadKernel32> | null = null;
let sh32: ReturnType<typeof loadShell32> | null = null;
let verdll: ReturnType<typeof loadVersion> | null = null;
const kernel32 = () => (k32 ??= loadKernel32());
const shell32 = () => (sh32 ??= loadShell32());
const version = () => (verdll ??= loadVersion());

export function requireWindows(what: string) {
  if (process.platform !== "win32") {
    throw new Error(
      `${what} uses the Win32 resource API and runs on Windows only; stamping on another OS is not supported.`,
    );
  }
}

/** NUL-terminated UTF-16LE. The buffer must outlive the call — keep a reference. */
export const wide = (s: string): Uint8Array => new Uint8Array(Buffer.from(s + "\0", "utf16le"));
const P = (b: Uint8Array) => BigInt(ptr(b));

export const ERROR_SHARING_VIOLATION = 32;
export const ERROR_LOCK_VIOLATION = 33;
export const ERROR_ACCESS_DENIED = 5;

export class Win32Error extends Error {
  constructor(
    readonly fn: string,
    readonly code: number,
  ) {
    super(
      `${fn} failed with Win32 error ${code}${code === ERROR_SHARING_VIOLATION ? " (sharing violation — another process has the file open)" : ""}`,
    );
  }
}

export type ResId = number | string;

/** One `BeginUpdateResourceW` ... `EndUpdateResourceW` transaction. String buffers handed to Win32 stay alive on `this` until `commit`: `UpdateResourceW` makes no promise about when it reads the name. */
export class ResourceUpdate {
  #h: bigint;
  #keep: Uint8Array[] = [];
  #done = false;

  constructor(readonly path: string) {
    requireWindows("stamping an exe");
    const p = wide(path);
    this.#keep.push(p);
    const h = kernel32().BeginUpdateResourceW(P(p), 0);
    if (!h) throw new Win32Error("BeginUpdateResourceW", kernel32().GetLastError());
    this.#h = BigInt(h);
  }

  #id(id: ResId): bigint {
    if (typeof id === "number") return BigInt(id);
    const b = wide(id);
    this.#keep.push(b);
    return P(b);
  }

  set(type: ResId, name: ResId, lang: number, data: Uint8Array) {
    this.#keep.push(data);
    const ok = kernel32().UpdateResourceW(
      this.#h,
      this.#id(type),
      this.#id(name),
      lang,
      data.length ? P(data) : 0n,
      data.length,
    );
    if (!ok) throw new Win32Error(`UpdateResourceW(${type}/${name}/${lang})`, kernel32().GetLastError());
  }

  remove(type: ResId, name: ResId, lang: number) {
    const ok = kernel32().UpdateResourceW(this.#h, this.#id(type), this.#id(name), lang, 0n, 0);
    if (!ok) throw new Win32Error(`UpdateResourceW(delete ${type}/${name}/${lang})`, kernel32().GetLastError());
  }

  commit() {
    if (this.#done) return;
    this.#done = true;
    const ok = kernel32().EndUpdateResourceW(this.#h, 0);
    this.#keep = [];
    if (!ok) throw new Win32Error("EndUpdateResourceW", kernel32().GetLastError());
  }

  discard() {
    if (this.#done) return;
    this.#done = true;
    kernel32().EndUpdateResourceW(this.#h, 1);
    this.#keep = [];
  }
}

const LOAD_LIBRARY_AS_DATAFILE = 0x2;
const LOAD_LIBRARY_AS_IMAGE_RESOURCE = 0x20;

export function loadResourceCopy(path: string, type: ResId, name: ResId, maxBytes = Infinity): Uint8Array | null {
  requireWindows("reading an exe's resources");
  const k = kernel32();
  const p = wide(path);
  const mod = k.LoadLibraryExW(P(p), 0n, LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE);
  if (!mod) throw new Win32Error(`LoadLibraryExW(${path})`, k.GetLastError());
  try {
    const keep: Uint8Array[] = [];
    const id = (v: ResId) => {
      if (typeof v === "number") return BigInt(v);
      const b = wide(v);
      keep.push(b);
      return P(b);
    };
    const res = k.FindResourceW(mod, id(name), id(type));
    if (!res) return null;
    const hg = k.LoadResource(mod, res);
    if (!hg) throw new Win32Error("LoadResource", k.GetLastError());
    const at = k.LockResource(hg);
    const size = k.SizeofResource(mod, res);
    if (!at) throw new Win32Error("LockResource", k.GetLastError());
    const n = Math.min(size, maxBytes);
    return new Uint8Array(toArrayBuffer(Number(at), 0, n).slice(0));
  } finally {
    k.FreeLibrary(mod);
  }
}

export function shellIconCount(path: string): number {
  requireWindows("counting icons");
  const p = wide(path);
  return shell32().ExtractIconExW(P(p), -1, 0n, 0n, 0);
}

export function versionString(path: string, key: string, table = "040904b0"): string | null {
  requireWindows("reading VERSIONINFO");
  const v = version();
  const p = wide(path);
  const size = v.GetFileVersionInfoSizeW(P(p), 0n);
  if (!size) return null;
  const buf = new Uint8Array(size);
  if (!v.GetFileVersionInfoW(P(p), 0, size, P(buf))) return null;
  const outPtr = new Uint8Array(8);
  const outLen = new Uint8Array(4);
  const q = wide(`\\StringFileInfo\\${table}\\${key}`);
  if (!v.VerQueryValueW(P(buf), P(q), P(outPtr), P(outLen))) return null;
  const at = Number(new DataView(outPtr.buffer).getBigUint64(0, true));
  const chars = new DataView(outLen.buffer).getUint32(0, true);
  if (!at || !chars) return "";
  const raw = new Uint8Array(toArrayBuffer(at, 0, chars * 2).slice(0));
  return new TextDecoder("utf-16le").decode(raw).replace(/\0+$/, "");
}

export function fixedFileVersion(path: string): string | null {
  requireWindows("reading VERSIONINFO");
  const v = version();
  const p = wide(path);
  const size = v.GetFileVersionInfoSizeW(P(p), 0n);
  if (!size) return null;
  const buf = new Uint8Array(size);
  if (!v.GetFileVersionInfoW(P(p), 0, size, P(buf))) return null;
  const outPtr = new Uint8Array(8);
  const outLen = new Uint8Array(4);
  const q = wide("\\");
  if (!v.VerQueryValueW(P(buf), P(q), P(outPtr), P(outLen))) return null;
  const at = Number(new DataView(outPtr.buffer).getBigUint64(0, true));
  const ffi = new DataView(toArrayBuffer(at, 0, 52).slice(0));
  if (ffi.getUint32(0, true) !== 0xfeef04bd) return null;
  const ms = ffi.getUint32(8, true);
  const ls = ffi.getUint32(12, true);
  return `${ms >>> 16}.${ms & 0xffff}.${ls >>> 16}.${ls & 0xffff}`;
}
