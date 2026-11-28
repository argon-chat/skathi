import { createHash } from "node:crypto";
import { posix } from "node:path";

const MAGIC = Buffer.from("HIVE.BANK");
export const SKBANK_FORMAT = 3;
const TOC_MAGIC = 0x48495645544f4333n;
const END_MAGIC = 0x544f435f454e4421n;
const HEADER_OPEN = 0xaa;
const HEADER_CLOSE = 0xdd;
const ID_BYTES = 16;
const SIGNATURE_BYTES = 256;
const SIGNED = 4n;
const SUPPORTED_FLAGS = 2n | SIGNED | 8n; // compression capability, RSA signature, root world
const STORE = 0;
const DEDUP_REFERENCE = 2;
const FOOTER_SIZE = 16; // TOC offset and end magic
const TOC_HEADER_SIZE = 16; // TOC magic and entry count
const STORE_RECORD_TAIL = 3 * 8 + ID_BYTES + 2 + 4; // offset, length, CRC, ID, flags, codec, empty dedup target
const MIN_RECORD_SIZE = 4 + STORE_RECORD_TAIL; // empty path
const MAX_ENTRIES = 10_000_000;
const MAX_STRING = 1024 * 1024;
const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

export function isSkBank(bytes: Uint8Array): boolean {
  return bytes.length >= MAGIC.length && MAGIC.every((byte, i) => bytes[i] === byte);
}

// CRC-64/ECMA-182. Slice n is the contribution of one byte followed by n zero bytes.
const crcHigh = new Uint32Array(8 * 256);
const crcLow = new Uint32Array(8 * 256);
for (let i = 0; i < 256; i++) {
  let c = BigInt(i) << 56n;
  for (let bit = 0; bit < 8; bit++) {
    c = BigInt.asUintN(64, (c << 1n) ^ ((c >> 63n) * 0x42f0e1eba9ea3693n));
  }
  crcHigh[i] = Number(c >> 32n);
  crcLow[i] = Number(c & 0xffff_ffffn);
}
for (let i = 256; i < crcHigh.length; i++) {
  const high = crcHigh[i - 256]!;
  const low = crcLow[i - 256]!;
  crcHigh[i] = (high << 8) ^ (low >>> 24) ^ crcHigh[high >>> 24]!;
  crcLow[i] = (low << 8) ^ crcLow[high >>> 24]!;
}

function foldEightBytes(table: Uint32Array, first: number, last: number): number {
  return (
    table[7 * 256 + (first >>> 24)]! ^
    table[6 * 256 + ((first >>> 16) & 0xff)]! ^
    table[5 * 256 + ((first >>> 8) & 0xff)]! ^
    table[4 * 256 + (first & 0xff)]! ^
    table[3 * 256 + (last >>> 24)]! ^
    table[2 * 256 + ((last >>> 16) & 0xff)]! ^
    table[256 + ((last >>> 8) & 0xff)]! ^
    table[last & 0xff]!
  );
}

export function crc64(bytes: Uint8Array): bigint {
  let high = 0;
  let low = 0;
  let i = 0;
  if (bytes.length >= 8) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    for (; i <= bytes.length - 8; i += 8) {
      // XOR uses signed words. A uint32 load can box high-bit binary data as doubles.
      const first = high ^ view.getInt32(i);
      const last = low ^ view.getInt32(i + 4);
      high = foldEightBytes(crcHigh, first, last);
      low = foldEightBytes(crcLow, first, last);
    }
  }
  for (; i < bytes.length; i++) {
    const index = (high >>> 24) ^ bytes[i]!;
    high = (high << 8) ^ (low >>> 24) ^ crcHigh[index]!;
    low = (low << 8) ^ crcLow[index]!;
  }
  return (BigInt(high >>> 0) << 32n) | BigInt(low >>> 0);
}

function u32(n: number): Buffer {
  const buf = Buffer.allocUnsafe(4);
  buf.writeUInt32LE(n);
  return buf;
}

function u64(n: number | bigint): Buffer {
  const buf = Buffer.allocUnsafe(8);
  buf.writeBigUInt64LE(BigInt(n));
  return buf;
}

function string(value: string): Buffer {
  const length = Buffer.byteLength(value);
  if (length > MAX_STRING) throw new Error("SkBank: string exceeds the bank's 1 MiB limit");
  const field = Buffer.allocUnsafe(4 + length);
  field.writeUInt32LE(length);
  field.write(value, 4);
  return field;
}

export interface StoreFile {
  path: string;
  data: Uint8Array;
  alignment: number;
}

function encodeHeader(files: readonly StoreFile[]): Buffer {
  const bankId = createHash("sha256").update("SkBank/v3\0");
  for (const { path } of files) bankId.update(path).update("\0");
  const setId = createHash("sha256").update("SkBank/v3").digest().subarray(0, ID_BYTES);
  const version = Buffer.alloc(2);
  version.writeUInt16LE(SKBANK_FORMAT);
  return Buffer.concat([
    MAGIC,
    version,
    Buffer.from([HEADER_OPEN]),
    u64(0),
    bankId.digest().subarray(0, ID_BYTES),
    setId,
    u64(files.length),
    u64(0),
    u64(0), // file, component and archetype counts
    u64(0),
    Buffer.from([HEADER_CLOSE]), // per-file alignment is carried in entity tags
  ]);
}

// A regular DeHive tag keeps sequential readers in sync and aligns the data relative to its mapped address.
function alignmentTags(unpaddedDataAddress: number, alignment: number): Buffer {
  if (unpaddedDataAddress % alignment === 0) return u32(0);
  const prefix = "sk.pad";
  const tagAddress = unpaddedDataAddress + 4 + prefix.length;
  const fill = (alignment - (tagAddress % alignment)) % alignment;
  return Buffer.concat([u32(1), string(prefix + ".".repeat(fill))]);
}

const STORE_ENTITY_END = Buffer.concat([Buffer.from([STORE, 0x0a]), u64(0xaabbccddeen), string(".e_file")]);

function encodeStoreEntry(file: StoreFile, offset: number, baseOffset: number) {
  const path = string(file.path);
  const checksum = u64(crc64(file.data));
  const id = createHash("sha256").update(file.path).update(checksum).digest().subarray(0, ID_BYTES);
  const length = BigInt(file.data.length);
  const name = posix.basename(file.path);
  const extension = posix.extname(name);
  const names = [string(name.slice(0, name.length - extension.length)), string(name), path, string(extension)];
  const nameBytes = names.reduce((sum, field) => sum + field.length, 0);
  const fixedSize = 8 + ID_BYTES + nameBytes + checksum.length + STORE_ENTITY_END.length;
  const tags = alignmentTags(baseOffset + offset + fixedSize + 4, file.alignment);

  const header = Buffer.allocUnsafe(fixedSize + tags.length);
  let at = header.writeBigUInt64LE(length);
  at += id.copy(header, at);
  for (const field of names) at += field.copy(header, at);
  at += tags.copy(header, at);
  at += checksum.copy(header, at);
  STORE_ENTITY_END.copy(header, at);

  const record = Buffer.allocUnsafe(path.length + STORE_RECORD_TAIL);
  at = path.copy(record);
  at = record.writeBigUInt64LE(BigInt(offset + header.length), at);
  at = record.writeBigUInt64LE(length, at);
  at += checksum.copy(record, at);
  at += id.copy(record, at);
  at = record.writeUInt16LE(0, at); // no flags, Store codec
  record.writeUInt32LE(0, at); // empty dedup target
  return { header, record };
}

export function writeStoreBank(files: readonly StoreFile[], baseOffset: number): Uint8Array {
  if (files.length > MAX_ENTRIES) throw new Error("SkBank: invalid entry count");
  const header = encodeHeader(files);
  const body: Uint8Array[] = [header];
  const index: Buffer[] = [u64(TOC_MAGIC), u64(files.length)];
  let offset = header.length;
  for (const file of files) {
    const entry = encodeStoreEntry(file, offset, baseOffset);
    body.push(entry.header, file.data);
    index.push(entry.record);
    offset += entry.header.length + file.data.length;
  }
  index.push(u64(offset), u64(END_MAGIC));
  const directory = Buffer.concat(index);
  if (offset + directory.length > 0xffff_ffff) throw new Error("SkBank: payload exceeds the PE resource size limit");
  return Buffer.concat([...body, directory], offset + directory.length);
}

class Cursor {
  position = 0;
  constructor(readonly buffer: Buffer) {}
  take(length: number): Buffer {
    if (length > this.buffer.length - this.position) throw new Error("SkBank: truncated bank");
    const start = this.position;
    this.position += length;
    return this.buffer.subarray(start, this.position);
  }
  byte(): number {
    return this.take(1)[0]!;
  }
  u64(): bigint {
    return this.take(8).readBigUInt64LE();
  }
  string(): string {
    const length = this.take(4).readUInt32LE();
    if (length > MAX_STRING) throw new Error("SkBank: invalid string length");
    return utf8.decode(this.take(length));
  }
}

interface Header {
  size: number;
  fileCount: bigint;
  alignment: bigint;
}

function readHeader(buffer: Buffer): Header {
  const cursor = new Cursor(buffer);
  if (!cursor.take(MAGIC.length).equals(MAGIC)) throw new Error("SkBank: invalid magic");
  if (cursor.take(2).readUInt16LE() !== SKBANK_FORMAT) throw new Error("SkBank: unsupported bank version");
  if (cursor.byte() !== HEADER_OPEN) throw new Error("SkBank: invalid header marker");
  const flags = cursor.u64();
  cursor.take(2 * ID_BYTES); // bank and set IDs
  const fileCount = cursor.u64();
  const componentCount = cursor.u64();
  const archetypeCount = cursor.u64();
  if ((flags & ~SUPPORTED_FLAGS) !== 0n || componentCount !== 0n || archetypeCount !== 0n) {
    throw new Error("SkBank: expected a directly mapped data bank");
  }
  // Opening the bank only reads the signature slot. Authenticating it requires a trusted public key.
  if (flags & SIGNED) cursor.take(SIGNATURE_BYTES);
  const alignment = cursor.u64();
  if (cursor.byte() !== HEADER_CLOSE) throw new Error("SkBank: invalid header marker");
  return { size: cursor.position, fileCount, alignment };
}

interface IndexRecord {
  path: string;
  start: number;
  len: number;
  crc64: bigint;
  target: string;
}

function* readRecords(cursor: Cursor, header: Header, indexOffset: bigint, count: number): Generator<IndexRecord> {
  for (let i = 0; i < count; i++) {
    const path = cursor.string();
    const start = cursor.u64();
    const length = cursor.u64();
    const checksum = cursor.u64();
    cursor.take(ID_BYTES);
    const flags = cursor.byte();
    const codec = cursor.byte();
    const target = cursor.string();

    if (start < BigInt(header.size) || start + length > indexOffset) throw new Error("SkBank: data outside bank");
    if (header.alignment > 1n && start % header.alignment !== 0n) throw new Error("SkBank: invalid block alignment");
    const validStorage = target ? flags === DEDUP_REFERENCE && length === 0n : flags === 0;
    if (codec !== STORE || !validStorage)
      throw new Error("SkBank: expected a Store entry or a deduplication reference");
    yield { path, start: Number(start), len: Number(length), crc64: checksum, target };
  }
  if (cursor.position !== cursor.buffer.length) throw new Error("SkBank: unexpected trailing TOC data");
}

export function readBankIndex(bytes: Uint8Array): { offset: number; records: Iterable<IndexRecord> } {
  const buffer = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const header = readHeader(buffer);
  const footerStart = buffer.length - FOOTER_SIZE;
  if (footerStart < header.size + TOC_HEADER_SIZE) throw new Error("SkBank: truncated bank");
  const footer = new Cursor(buffer.subarray(footerStart));
  const offset = footer.u64();
  if (footer.u64() !== END_MAGIC) throw new Error("SkBank: missing TOC footer");
  if (offset < BigInt(header.size) || offset > BigInt(footerStart - TOC_HEADER_SIZE))
    throw new Error("SkBank: invalid TOC offset");

  const cursor = new Cursor(buffer.subarray(Number(offset), footerStart));
  if (cursor.u64() !== TOC_MAGIC) throw new Error("SkBank: expected a v3 TOC");
  const count = cursor.u64();
  const maxRecords = Math.floor((cursor.buffer.length - cursor.position) / MIN_RECORD_SIZE);
  if (count !== header.fileCount || count > MAX_ENTRIES || count > maxRecords)
    throw new Error("SkBank: invalid entry count");
  const rows = cursor.buffer.subarray(cursor.position);
  const records = () => readRecords(new Cursor(rows), header, offset, Number(count));
  return { offset: Number(offset), records: resolveReferences(records) };
}

function* resolveReferences(records: () => Iterable<IndexRecord>): Generator<IndexRecord> {
  let byPath: Map<string, IndexRecord> | undefined;
  for (const record of records()) {
    if (record.target) {
      // References can point forward. Build the physical path index only for banks that use them.
      if (!byPath) {
        byPath = new Map();
        for (const row of records()) byPath.set(bankPathKey(row.path), row);
      }
      let source = record;
      for (let depth = 0; source.target; depth++) {
        if (depth >= 16) throw new Error("SkBank: deduplication chain is cyclic or too long");
        const next = byPath.get(bankPathKey(source.target));
        if (!next) throw new Error("SkBank: missing deduplication target");
        source = next;
      }
      if (source.crc64 !== record.crc64) throw new Error("SkBank: deduplication target CRC64 differs");
      record.start = source.start;
      record.len = source.len;
    }
    yield record;
  }
}

// DeHive's HiveRelativePath uses normalized separators and .NET OrdinalIgnoreCase.
function bankPathKey(path: string): string {
  let key = "";
  for (const c of path.replaceAll("\\", "/").replace(/^\/+/, "")) {
    const code = c.codePointAt(0)!;
    // Greek prosgegrammeni has a simple uppercase distinct from the string's expanded uppercase.
    if (code >= 0x1f80 && code <= 0x1fa7 && (code & 15) < 8) {
      key += String.fromCodePoint(code + 8);
    } else if (code === 0x1fb3 || code === 0x1fc3 || code === 0x1ff3) {
      key += String.fromCodePoint(code + 9);
    } else {
      const upper = c.toUpperCase();
      const upperCode = upper.codePointAt(0)!;
      key += upper.length === (upperCode > 0xffff ? 2 : 1) && (code < 128 || upperCode >= 128) ? upper : c;
    }
  }
  return key;
}
