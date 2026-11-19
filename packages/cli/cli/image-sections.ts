// Native AOT images as PE sections, written without a linker. `sk build --aot` appends the app's image (and, with
// devtools, the inspector's) to a copy of the workload's empty `skimage.dll` beside the exe, or with
// "publishSingleFile" to the exe itself. The image stays uncompressed and keeps one contiguous, page-aligned range
// in the file and in memory, so the host maps it where it lies: the loader maps the exe's sections, and the engine
// maps a container's range from its file (crates/sk-app/src/native_aot/container.rs).
//
// Three sections per image, so the loader protects each part as the engine would: the header and tables read-only,
// the code read-and-execute, the rest read-only. Writing them strips a certificate table (sign afterwards) and
// rewrites the section table, SizeOfImage, SizeOfCode, SizeOfInitializedData and CheckSum; when the section table
// has no room, the headers grow and every file offset behind them moves (sections, the debug directory).

import type { Platform } from "./platform";

export const PAGE = 4096;

export interface ImageSectionNames {
  header: string;
  code: string;
  data: string;
}
/** The host finds an image by its first section's name. */
export const APP_SECTIONS: ImageSectionNames = { header: ".skapp", code: ".skappx", data: ".skappd" };
export const INSPECTOR_SECTIONS: ImageSectionNames = { header: ".skins", code: ".skinsx", data: ".skinsd" };

/** The empty container a workload carries beside its host. */
export const TEMPLATE_NAME = "skimage.dll";

/** A container's file name beside the app: the platform's shared-library form. */
export function containerName(kind: "app" | "inspector", platform: Platform): string {
  const extension = platform === "windows" ? "dll" : platform === "macos" ? "dylib" : "so";
  return `${kind}.image.${extension}`;
}

export interface ImageLayout {
  size: number;
  codeOffset: number;
  executableSize: number;
}

/** The members of the image header the sections are cut by (JSC's `ImageHeader`, at fixed offsets). */
export function imageLayout(image: Uint8Array): ImageLayout {
  if (image.length < 72) throw new Error("the native image is too short to have a header");
  const view = new DataView(image.buffer, image.byteOffset, image.byteLength);
  const u64 = (at: number) => Number(view.getBigUint64(at, true));
  const layout = { size: u64(32), codeOffset: u64(40), executableSize: u64(64) };
  const { size, codeOffset, executableSize } = layout;
  if (
    size !== image.length ||
    size % PAGE ||
    codeOffset % PAGE ||
    executableSize % PAGE ||
    !codeOffset ||
    !executableSize ||
    codeOffset + executableSize > size
  ) {
    throw new Error(
      `the native image's header does not describe it (size ${size} of ${image.length} bytes, code at ${codeOffset}, ` +
        `${executableSize} executable): rebuild it with the sk-aot that matches the workload`,
    );
  }
  return layout;
}

const SCN_CODE = 0x00000020;
const SCN_INITIALIZED_DATA = 0x00000040;
const SCN_EXECUTE = 0x20000000;
const SCN_READ = 0x40000000;
const SECTION_HEADER = 40;
const DIRECTORY_SECURITY = 4;
const DIRECTORY_DEBUG = 6;

interface SectionHeader {
  at: number;
  name: string;
  virtualSize: number;
  virtualAddress: number;
  rawSize: number;
  rawOffset: number;
}

class Pe {
  view: DataView;
  coff: number;
  optional: number;
  table: number;
  constructor(public bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (bytes.length < 0x40 || this.u16(0) !== 0x5a4d) throw new Error("not a PE file (no MZ header)");
    const pe = this.u32(0x3c);
    if (pe + 24 > bytes.length || this.u32(pe) !== 0x4550) throw new Error("not a PE file (no PE signature)");
    this.coff = pe + 4;
    this.optional = this.coff + 20;
    if (this.u16(this.optional) !== 0x20b) throw new Error("not a 64-bit PE file");
    this.table = this.optional + this.u16(this.coff + 16);
  }
  u16(at: number) {
    return this.view.getUint16(at, true);
  }
  u32(at: number) {
    return this.view.getUint32(at, true);
  }
  set16(at: number, value: number) {
    this.view.setUint16(at, value, true);
  }
  set32(at: number, value: number) {
    this.view.setUint32(at, value >>> 0, true);
  }
  get count() {
    return this.u16(this.coff + 2);
  }
  get sectionAlignment() {
    return this.u32(this.optional + 32);
  }
  get fileAlignment() {
    return this.u32(this.optional + 36);
  }
  get sizeOfHeaders() {
    return this.u32(this.optional + 60);
  }
  directory(index: number): { at: number; address: number; size: number } | null {
    if (index >= this.u32(this.optional + 108)) return null;
    const at = this.optional + 112 + index * 8;
    return { at, address: this.u32(at), size: this.u32(at + 4) };
  }
  sections(): SectionHeader[] {
    return Array.from({ length: this.count }, (_, index) => {
      const at = this.table + index * SECTION_HEADER;
      const raw = this.bytes.subarray(at, at + 8);
      const end = raw.indexOf(0);
      return {
        at,
        name: new TextDecoder().decode(end < 0 ? raw : raw.subarray(0, end)),
        virtualSize: this.u32(at + 8),
        virtualAddress: this.u32(at + 12),
        rawSize: this.u32(at + 16),
        rawOffset: this.u32(at + 20),
      };
    });
  }
}

const alignUp = (value: number, to: number) => Math.ceil(value / to) * to;

/** The standard PE checksum: 16-bit one's-complement sum of the file without its CheckSum field, plus its length. */
export function peChecksum(bytes: Uint8Array, checksumAt: number): number {
  let sum = 0;
  const words = Math.floor(bytes.length / 2);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let i = 0; i < words; i++) {
    const at = i * 2;
    if (at === checksumAt || at === checksumAt + 2) continue;
    sum += view.getUint16(at, true);
    if (sum > 0xffff) sum = (sum & 0xffff) + (sum >>> 16);
  }
  if (bytes.length % 2) {
    sum += bytes[bytes.length - 1]!;
    sum = (sum & 0xffff) + (sum >>> 16);
  }
  return ((sum & 0xffff) + bytes.length) >>> 0;
}

function sectionName(name: string): Uint8Array {
  const out = new Uint8Array(8);
  const encoded = new TextEncoder().encode(name);
  if (encoded.length > 8) throw new Error(`section name ${name} is longer than 8 bytes`);
  out.set(encoded);
  return out;
}

/** Room for `more` section headers, growing the headers when needed. Returns the (possibly new) file. */
function makeRoom(pe: Pe, more: number): Pe {
  const needed = pe.table + (pe.count + more) * SECTION_HEADER;
  const sections = pe.sections();
  const firstRaw = Math.min(...sections.filter((s) => s.rawSize).map((s) => s.rawOffset), pe.bytes.length);
  const firstAddress = Math.min(...sections.map((s) => s.virtualAddress));
  if (needed <= firstRaw && needed <= pe.sizeOfHeaders) return pe;
  const sizeOfHeaders = alignUp(needed, pe.fileAlignment);
  if (sizeOfHeaders > firstAddress) {
    throw new Error("the PE headers have no room for more sections below the first section's address");
  }
  // Whole pages once a section lies on one, so that images already in the file stay page-aligned in it.
  const pages = sections.some((s) => s.rawSize && s.rawOffset % PAGE === 0);
  const delta = Math.max(0, alignUp(needed - firstRaw, pages ? PAGE : pe.fileAlignment));
  const grown = new Uint8Array(pe.bytes.length + delta);
  grown.set(pe.bytes.subarray(0, firstRaw));
  grown.set(pe.bytes.subarray(firstRaw), firstRaw + delta);
  const out = new Pe(grown);
  out.set32(out.optional + 60, Math.max(pe.sizeOfHeaders, sizeOfHeaders));
  if (delta) {
    for (const s of out.sections()) {
      if (s.rawOffset) out.set32(s.at + 20, s.rawOffset + delta);
      for (const field of [24, 28]) if (out.u32(s.at + field)) out.set32(s.at + field, out.u32(s.at + field) + delta);
    }
    const symbols = out.u32(out.coff + 8);
    if (symbols) out.set32(out.coff + 8, symbols + delta);
    const debug = out.directory(DIRECTORY_DEBUG);
    if (debug?.size) {
      const holder = out
        .sections()
        .find((s) => debug.address >= s.virtualAddress && debug.address < s.virtualAddress + s.rawSize);
      if (!holder) throw new Error("the debug directory lies outside the file's sections");
      const at = holder.rawOffset + (debug.address - holder.virtualAddress);
      for (let entry = at; entry < at + debug.size; entry += 28) {
        const raw = out.u32(entry + 24);
        if (raw) out.set32(entry + 24, raw + delta);
      }
    }
  }
  return out;
}

/** Drops an Authenticode certificate table at the end of the file; its signature would no longer hold. */
function withoutCertificate(pe: Pe): Pe {
  const security = pe.directory(DIRECTORY_SECURITY);
  if (!security?.size) return pe;
  if (security.address + security.size < pe.bytes.length - 8) {
    throw new Error("the certificate table is not at the end of the file");
  }
  pe.set32(security.at, 0);
  pe.set32(security.at + 4, 0);
  return new Pe(pe.bytes.slice(0, security.address));
}

/** A copy of the PE file `bytes` with `image` appended as the three sections `names`. */
export function appendImageSections(bytes: Uint8Array, image: Uint8Array, names: ImageSectionNames): Uint8Array {
  const layout = imageLayout(image);
  let pe = withoutCertificate(new Pe(bytes.slice()));
  if (pe.sectionAlignment !== PAGE || pe.fileAlignment > PAGE) {
    throw new Error(
      `the PE file aligns sections to ${pe.sectionAlignment} bytes in memory and ${pe.fileAlignment} in the file; ` +
        `a native image needs ${PAGE} and at most ${PAGE}`,
    );
  }
  if (pe.sections().some((s) => s.name === names.header)) {
    throw new Error(`the PE file already has a ${names.header} section`);
  }
  const parts = [
    { name: names.header, start: 0, end: layout.codeOffset, flags: SCN_INITIALIZED_DATA | SCN_READ },
    {
      name: names.code,
      start: layout.codeOffset,
      end: layout.codeOffset + layout.executableSize,
      flags: SCN_CODE | SCN_EXECUTE | SCN_READ,
    },
    {
      name: names.data,
      start: layout.codeOffset + layout.executableSize,
      end: layout.size,
      flags: SCN_INITIALIZED_DATA | SCN_READ,
    },
  ].filter((p) => p.end > p.start);
  pe = makeRoom(pe, parts.length);
  const sections = pe.sections();
  const rawStart = alignUp(pe.bytes.length, PAGE);
  const addressStart = alignUp(
    Math.max(...sections.map((s) => s.virtualAddress + Math.max(s.virtualSize, s.rawSize))),
    PAGE,
  );
  const file = new Uint8Array(rawStart + layout.size);
  file.set(pe.bytes);
  file.set(image, rawStart);
  const out = new Pe(file);
  let code = 0;
  let data = 0;
  parts.forEach((part, index) => {
    const at = out.table + (out.count + index) * SECTION_HEADER;
    const size = part.end - part.start;
    out.bytes.set(sectionName(part.name), at);
    out.set32(at + 8, size);
    out.set32(at + 12, addressStart + part.start);
    out.set32(at + 16, size);
    out.set32(at + 20, rawStart + part.start);
    out.set32(at + 36, part.flags);
    if (part.flags & SCN_CODE) code += size;
    else data += size;
  });
  out.set16(out.coff + 2, out.count + parts.length);
  out.set32(out.optional + 4, out.u32(out.optional + 4) + code);
  out.set32(out.optional + 8, out.u32(out.optional + 8) + data);
  out.set32(out.optional + 56, addressStart + layout.size);
  out.set32(out.optional + 64, peChecksum(out.bytes, out.optional + 64));
  return out.bytes;
}

/** The image a container or an exe holds in the sections `names`, or null. */
export function readImageSections(bytes: Uint8Array, names: ImageSectionNames): Uint8Array | null {
  const pe = new Pe(bytes);
  const header = pe.sections().find((s) => s.name === names.header);
  if (!header) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset + header.rawOffset, 40);
  const size = Number(view.getBigUint64(32, true));
  if (header.rawOffset + size > bytes.length)
    throw new Error(`the ${names.header} image runs past the end of the file`);
  return bytes.slice(header.rawOffset, header.rawOffset + size);
}
