// VNPK v1 — the one blob a packaged exe carries (research/reports/m25/packaging-design.md §4.2).
//
// The WRITER lives here; the READER that matters lives in Rust
// (`crates/vn-payload`), and the two are held to one golden fixture,
// `packages/vue-native/test/fixtures/golden.vnpk`, from both sides. The reader
// below exists for `vn build`'s own verify step and for the tests.
//
//   header (64 bytes, little-endian, read BYTE-WISE: the blob is only 4-byte
//   aligned inside the image)
//     0   [4]  "VNPK"
//     4   u16  format = 1            (a host refuses > 1: "built by a newer vn")
//     6   u16  flags                 (bit 0: has BYTECODE)
//     8   u32  entry_count
//     12  u32  toc_off               (= 64)
//     16  u32  names_off
//     20  u32  names_len
//     24  u32  skew                  zero bytes between the names and the data,
//                                    chosen by the stamper so data_off lands on
//                                    a page boundary IN THE MAPPED IMAGE
//     28  u32  reserved (0)
//     32  u64  data_off              = names_off + names_len + skew
//     40  u64  blob_len
//     48  [16] reserved (0)
//   toc: entry_count × 32 bytes, sorted by (kind, name bytes)
//     u32 name_off (rel. names_off), u32 name_len, u64 off (rel. data_off),
//     u64 len, u16 kind, u8 align_log2, u8 reserved, u32 crc32
//   names: UTF-8, no terminators
//   data: each entry aligned to 2^align_log2 relative to data_off
//
// No timestamps anywhere, so two builds of the same inputs are byte-identical.
// No compression: demand paging matters more than size, and PNG/MP4/TGS are
// already compressed.

export const MAGIC = "VNPK";
export const FORMAT = 1;
export const HEADER_SIZE = 64;
export const TOC_ENTRY_SIZE = 32;
export const PAGE = 4096;

/** Entry kinds. Only `ASSET` is ever reachable from app code. */
export const KIND = {
  META: 1,
  WINDOW: 2,
  SCRIPT: 3,
  /** Reserved for phase 3 (a JSC bytecode cache); page-aligned when present. */
  BYTECODE: 4,
  ASSET: 16,
} as const;

/** The names the host looks the internal entries up by. */
export const NAMES = {
  META: "vn/app.json",
  WINDOW: "vn/window.json",
  SCRIPT: "vn/app.js",
  BYTECODE: "vn/app.bc",
} as const;

export const FLAG_HAS_BYTECODE = 1;

export interface VnpkInput {
  name: string;
  kind: number;
  data: Uint8Array;
  /** Default: 12 (a page) for BYTECODE, 4 (16 bytes) for everything else. */
  alignLog2?: number;
}

export interface VnpkEntry {
  name: string;
  kind: number;
  /** Offset of the bytes from the start of the BLOB (not from data_off). */
  start: number;
  len: number;
  alignLog2: number;
  crc32: number;
}

export interface VnpkHeader {
  format: number;
  flags: number;
  entryCount: number;
  tocOff: number;
  namesOff: number;
  namesLen: number;
  skew: number;
  dataOff: number;
  blobLen: number;
}

// ---------------------------------------------------------------------------
// crc32 (IEEE) — for `vn verify` and the tests; the host never checks it at
// boot, because hashing the payload would fault in every page (design V7).
// ---------------------------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(bytes: Uint8Array, seed = 0): number {
  let c = (seed ^ 0xffffffff) >>> 0;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]!) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// ---------------------------------------------------------------------------
// writer
// ---------------------------------------------------------------------------

const utf8 = new TextEncoder();

/** Lexicographic byte order — what Rust's `str` Ord is, and what the host binary-searches. */
export function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i]! - b[i]!;
  }
  return a.length - b.length;
}

const alignUp = (n: number, to: number) => Math.ceil(n / to) * to;

function defaultAlign(kind: number): number {
  return kind === KIND.BYTECODE ? 12 : 4;
}

/**
 * Lays the entries out and returns the blob. `skew` is the stamper's business
 * (see `skewFor`); a blob written with `skew: 0` is a valid payload on its own,
 * just not a page-aligned one.
 */
export function writeVnpk(inputs: VnpkInput[], options: { skew?: number } = {}): Uint8Array {
  const skew = options.skew ?? 0;
  if (!Number.isInteger(skew) || skew < 0 || skew >= PAGE) {
    throw new Error(`vnpk: skew must be an integer in [0, ${PAGE}), got ${skew}`);
  }
  const items = inputs.map((e) => ({
    ...e,
    nameBytes: utf8.encode(e.name),
    alignLog2: e.alignLog2 ?? defaultAlign(e.kind),
  }));
  for (const e of items) {
    if (!e.name) throw new Error("vnpk: an entry has an empty name");
    if (!Number.isInteger(e.kind) || e.kind < 1 || e.kind > 0xffff) {
      throw new Error(`vnpk: ${e.name}: kind ${e.kind} is not a u16`);
    }
    if (e.alignLog2 < 0 || e.alignLog2 > 16) throw new Error(`vnpk: ${e.name}: align 2^${e.alignLog2}`);
  }
  items.sort((a, b) => a.kind - b.kind || compareBytes(a.nameBytes, b.nameBytes));
  for (let i = 1; i < items.length; i++) {
    const a = items[i - 1]!;
    const b = items[i]!;
    if (a.kind === b.kind && compareBytes(a.nameBytes, b.nameBytes) === 0) {
      throw new Error(`vnpk: duplicate entry ${JSON.stringify(b.name)} (kind ${b.kind})`);
    }
  }

  const tocOff = HEADER_SIZE;
  const namesOff = tocOff + items.length * TOC_ENTRY_SIZE;
  const namesLen = items.reduce((n, e) => n + e.nameBytes.length, 0);
  const dataOff = namesOff + namesLen + skew;

  // Data layout, relative to data_off.
  let cursor = 0;
  const placed = items.map((e) => {
    const off = alignUp(cursor, 1 << e.alignLog2);
    cursor = off + e.data.length;
    return off;
  });
  const blobLen = dataOff + cursor;
  if (blobLen > 0xffff_ffff) {
    throw new Error(`vnpk: ${blobLen} bytes — a PE resource is limited to 4 GiB`);
  }

  const out = new Uint8Array(blobLen);
  const dv = new DataView(out.buffer);
  out.set(utf8.encode(MAGIC), 0);
  dv.setUint16(4, FORMAT, true);
  let flags = 0;
  if (items.some((e) => e.kind === KIND.BYTECODE)) flags |= FLAG_HAS_BYTECODE;
  dv.setUint16(6, flags, true);
  dv.setUint32(8, items.length, true);
  dv.setUint32(12, tocOff, true);
  dv.setUint32(16, namesOff, true);
  dv.setUint32(20, namesLen, true);
  dv.setUint32(24, skew, true);
  dv.setUint32(28, 0, true);
  dv.setBigUint64(32, BigInt(dataOff), true);
  dv.setBigUint64(40, BigInt(blobLen), true);

  let nameCursor = 0;
  items.forEach((e, i) => {
    const t = tocOff + i * TOC_ENTRY_SIZE;
    dv.setUint32(t + 0, nameCursor, true);
    dv.setUint32(t + 4, e.nameBytes.length, true);
    dv.setBigUint64(t + 8, BigInt(placed[i]!), true);
    dv.setBigUint64(t + 16, BigInt(e.data.length), true);
    dv.setUint16(t + 24, e.kind, true);
    out[t + 26] = e.alignLog2;
    out[t + 27] = 0;
    dv.setUint32(t + 28, crc32(e.data), true);
    out.set(e.nameBytes, namesOff + nameCursor);
    nameCursor += e.nameBytes.length;
    out.set(e.data, dataOff + placed[i]!);
  });
  return out;
}

/**
 * The skew that puts `data_off` on a page boundary once the blob sits at image
 * RVA `rva`. Pure: the RVA does not move between stamping passes, because the
 * only resources laid out ahead of the payload are the `RT_ICON`s (measured,
 * design §2), and changing the skew changes nothing in front of the data.
 */
export function skewFor(rva: number, header: Pick<VnpkHeader, "namesOff" | "namesLen">): number {
  const namesEnd = header.namesOff + header.namesLen;
  return (PAGE - ((rva + namesEnd) % PAGE)) % PAGE;
}

// ---------------------------------------------------------------------------
// reader (verify + tests; the host's reader is crates/vn-payload)
// ---------------------------------------------------------------------------

export class VnpkError extends Error {}

export function readHeader(blob: Uint8Array): VnpkHeader {
  if (blob.length < HEADER_SIZE) throw new VnpkError(`vnpk: ${blob.length} bytes is shorter than a header`);
  const magic = new TextDecoder().decode(blob.subarray(0, 4));
  if (magic !== MAGIC) throw new VnpkError(`vnpk: bad magic ${JSON.stringify(magic)}`);
  const dv = new DataView(blob.buffer, blob.byteOffset, blob.byteLength);
  const format = dv.getUint16(4, true);
  if (format > FORMAT) {
    throw new VnpkError(`vnpk: format ${format} — this payload was built by a newer vn`);
  }
  return {
    format,
    flags: dv.getUint16(6, true),
    entryCount: dv.getUint32(8, true),
    tocOff: dv.getUint32(12, true),
    namesOff: dv.getUint32(16, true),
    namesLen: dv.getUint32(20, true),
    skew: dv.getUint32(24, true),
    dataOff: Number(dv.getBigUint64(32, true)),
    blobLen: Number(dv.getBigUint64(40, true)),
  };
}

export interface Vnpk {
  header: VnpkHeader;
  entries: VnpkEntry[];
  bytes(e: VnpkEntry): Uint8Array;
  find(kind: number, name: string): VnpkEntry | undefined;
}

export function readVnpk(blob: Uint8Array): Vnpk {
  const header = readHeader(blob);
  const dv = new DataView(blob.buffer, blob.byteOffset, blob.byteLength);
  const within = (what: string, start: number, len: number) => {
    if (start < 0 || len < 0 || start + len > blob.length) {
      throw new VnpkError(`vnpk: ${what} [${start}, +${len}) is outside the ${blob.length}-byte blob`);
    }
  };
  within("toc", header.tocOff, header.entryCount * TOC_ENTRY_SIZE);
  within("names", header.namesOff, header.namesLen);
  within("data", header.dataOff, 0);
  if (header.blobLen > blob.length) within("blob_len", 0, header.blobLen);
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const entries: VnpkEntry[] = [];
  for (let i = 0; i < header.entryCount; i++) {
    const t = header.tocOff + i * TOC_ENTRY_SIZE;
    const nameOff = dv.getUint32(t, true);
    const nameLen = dv.getUint32(t + 4, true);
    within(`entry ${i} name`, header.namesOff + nameOff, nameLen);
    if (nameOff + nameLen > header.namesLen) throw new VnpkError(`vnpk: entry ${i} name overruns names`);
    const nameBytes = blob.subarray(header.namesOff + nameOff, header.namesOff + nameOff + nameLen);
    let name: string;
    try {
      name = decoder.decode(nameBytes);
    } catch {
      throw new VnpkError(`vnpk: entry ${i} name is not UTF-8`);
    }
    const off = Number(dv.getBigUint64(t + 8, true));
    const len = Number(dv.getBigUint64(t + 16, true));
    within(`entry ${i} (${name}) data`, header.dataOff + off, len);
    entries.push({
      name,
      kind: dv.getUint16(t + 24, true),
      alignLog2: blob[t + 26]!,
      start: header.dataOff + off,
      len,
      crc32: dv.getUint32(t + 28, true),
    });
  }
  const encoded = entries.map((e) => utf8.encode(e.name));
  for (let i = 1; i < entries.length; i++) {
    const a = entries[i - 1]!;
    const b = entries[i]!;
    const order = a.kind - b.kind || compareBytes(encoded[i - 1]!, encoded[i]!);
    if (order >= 0) throw new VnpkError(`vnpk: the toc is not sorted at entry ${i} (${b.name})`);
  }
  return {
    header,
    entries,
    bytes: (e) => blob.subarray(e.start, e.start + e.len),
    find: (kind, name) => entries.find((e) => e.kind === kind && e.name === name),
  };
}
