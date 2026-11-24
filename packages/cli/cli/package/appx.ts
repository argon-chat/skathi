// The MSIX container, written without the Windows SDK: a ZIP64 archive whose every entry has a data
// descriptor, payload files first, then AppxManifest.xml, AppxBlockMap.xml and [Content_Types].xml, laid out as
// makeappx lays them out. The block map holds a SHA-256 of every 64 KiB of each file; a deflated file is
// compressed block by block (a full flush after each), so each block's compressed size can be listed too.

import { createHash } from "node:crypto";
import { closeSync, openSync, writeSync } from "node:fs";
import { constants, deflateRawSync } from "node:zlib";

export const BLOCK = 65536;

export interface AppxFile {
  /** The path inside the package, with `/` separators. */
  path: string;
  data: Uint8Array;
}

export interface BlockMapFile {
  path: string;
  size: number;
  lfhSize: number;
  blocks: { hash: string; size: number | null }[];
  /** The whole file's SHA-256, listed for a file of more than one block. */
  fileHash: string | null;
}

/** Already compressed: stored, as makeappx stores them. */
const STORED = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "avif",
  "ico",
  "zip",
  "7z",
  "gz",
  "br",
  "zst",
  "xz",
  "cab",
  "mp3",
  "mp4",
  "m4a",
  "aac",
  "ogg",
  "opus",
  "webm",
  "woff",
  "woff2",
]);

const RESERVED = new Set(["appxmanifest.xml", "appxblockmap.xml", "[content_types].xml", "appxsignature.p7x"]);

export function reservedPath(path: string): boolean {
  const p = path.toLowerCase();
  return RESERVED.has(p) || p.startsWith("appxmetadata/");
}

const CONTENT_TYPES = "[Content_Types].xml";

const UNRESERVED = /[A-Za-z0-9\-._~]/;

/** The ZIP name of a package path: every byte outside RFC 3986's unreserved set percent-encoded, as makeappx writes it. */
export function zipName(path: string): string {
  return path
    .split("/")
    .map((segment) =>
      [...segment]
        .map((ch) =>
          UNRESERVED.test(ch)
            ? ch
            : [...new TextEncoder().encode(ch)]
                .map((b) => `%${b.toString(16).toUpperCase().padStart(2, "0")}`)
                .join(""),
        )
        .join(""),
    )
    .join("/");
}

const sha256 = (data: Uint8Array) => createHash("sha256").update(data).digest("base64");

/** The bytes written for a file and what the block map says about them. */
export function encodeFile(file: AppxFile, compress: boolean): { body: Uint8Array; entry: BlockMapFile } {
  const { data } = file;
  const blocks: BlockMapFile["blocks"] = [];
  const parts: Uint8Array[] = [];
  const count = Math.ceil(data.length / BLOCK);
  for (let i = 0; i < count; i++) {
    const block = data.subarray(i * BLOCK, Math.min(data.length, (i + 1) * BLOCK));
    if (compress) {
      const last = i === count - 1;
      const packed = new Uint8Array(
        deflateRawSync(block, { finishFlush: last ? constants.Z_FINISH : constants.Z_FULL_FLUSH }),
      );
      parts.push(packed);
      blocks.push({ hash: sha256(block), size: packed.length });
    } else {
      blocks.push({ hash: sha256(block), size: null });
    }
  }
  if (compress && count === 0) parts.push(new Uint8Array(deflateRawSync(new Uint8Array(0))));
  const body = compress ? Buffer.concat(parts) : data;
  const lfhSize = 30 + new TextEncoder().encode(zipName(file.path)).length;
  return {
    body,
    entry: {
      path: file.path,
      size: data.length,
      lfhSize,
      blocks,
      fileHash: count > 1 ? sha256(data) : null,
    },
  };
}

const xmlAttr = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export function blockMapXml(files: readonly BlockMapFile[]): string {
  const out = [
    '<?xml version="1.0" encoding="UTF-8" standalone="no"?>',
    '<BlockMap xmlns="http://schemas.microsoft.com/appx/2010/blockmap" xmlns:b4="http://schemas.microsoft.com/appx/2021/blockmap" IgnorableNamespaces="b4" HashMethod="http://www.w3.org/2001/04/xmlenc#sha256">',
  ];
  for (const f of files) {
    const head = `<File Name="${xmlAttr(f.path.replace(/\//g, "\\"))}" Size="${f.size}" LfhSize="${f.lfhSize}"`;
    if (!f.blocks.length) {
      out.push(`${head}/>`);
      continue;
    }
    const blocks = f.blocks.map((b) => `<Block Hash="${b.hash}"${b.size === null ? "" : ` Size="${b.size}"`}/>`);
    const hash = f.fileHash ? `<b4:FileHash Hash="${f.fileHash}"/>` : "";
    out.push(`${head}>${blocks.join("")}${hash}</File>`);
  }
  out.push("</BlockMap>");
  return out.join("");
}

const MIME: Readonly<Record<string, string>> = {
  exe: "application/x-msdownload",
  dll: "application/x-msdownload",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  ico: "image/vnd.microsoft.icon",
  json: "application/json",
  txt: "text/plain",
  xml: "text/xml",
  html: "text/html",
  css: "text/css",
  js: "application/javascript",
  svg: "image/svg+xml",
  wasm: "application/wasm",
  woff2: "font/woff2",
  ttf: "font/ttf",
};

const MANIFEST_TYPE = "application/vnd.ms-appx.manifest+xml";

export function contentTypesXml(payload: readonly string[]): string {
  const defaults: string[] = [];
  const overrides: string[] = [];
  const seen = new Set<string>();
  for (const path of payload) {
    const name = path.split("/").at(-1) ?? path;
    const dot = name.lastIndexOf(".");
    if (dot <= 0 || dot === name.length - 1) {
      overrides.push(`<Override PartName="/${zipName(path)}" ContentType="application/octet-stream" />`);
      continue;
    }
    const ext = name.slice(dot + 1);
    if (seen.has(ext.toLowerCase())) continue;
    seen.add(ext.toLowerCase());
    defaults.push(
      `<Default Extension="${xmlAttr(zipName(ext))}" ContentType="${MIME[ext.toLowerCase()] ?? "application/octet-stream"}" />`,
    );
  }
  if (seen.has("xml")) overrides.push(`<Override PartName="/AppxManifest.xml" ContentType="${MANIFEST_TYPE}" />`);
  else defaults.push(`<Default Extension="xml" ContentType="${MANIFEST_TYPE}" />`);
  overrides.push('<Override PartName="/AppxBlockMap.xml" ContentType="application/vnd.ms-appx.blockmap+xml" />');
  return `<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">${[...defaults, ...overrides].join("")}</Types>`;
}

const compressible = (path: string) => !STORED.has(path.split(".").at(-1)?.toLowerCase() ?? "");

class ZipWriter {
  private readonly fd: number;
  offset = 0;
  private readonly central: Uint8Array[] = [];
  private count = 0;

  constructor(path: string) {
    this.fd = openSync(path, "w");
  }

  private write(bytes: Uint8Array): void {
    let done = 0;
    while (done < bytes.length) done += writeSync(this.fd, bytes, done, bytes.length - done);
    this.offset += bytes.length;
  }

  add(path: string, data: Uint8Array, body: Uint8Array, deflated: boolean): void {
    // The content types part is not an OPC part name, so it keeps its brackets.
    const name = new TextEncoder().encode(path === CONTENT_TYPES ? path : zipName(path));
    const method = deflated ? 8 : 0;
    const crc = Bun.hash.crc32(data);
    const start = this.offset;
    const lfh = Buffer.alloc(30);
    lfh.writeUInt32LE(0x04034b50, 0);
    lfh.writeUInt16LE(45, 4);
    lfh.writeUInt16LE(8, 6);
    lfh.writeUInt16LE(method, 8);
    lfh.writeUInt16LE(0, 10);
    lfh.writeUInt16LE(0x21, 12);
    lfh.writeUInt16LE(name.length, 26);
    this.write(lfh);
    this.write(name);
    this.write(body);
    const dd = Buffer.alloc(24);
    dd.writeUInt32LE(0x08074b50, 0);
    dd.writeUInt32LE(crc, 4);
    dd.writeBigUInt64LE(BigInt(body.length), 8);
    dd.writeBigUInt64LE(BigInt(data.length), 16);
    this.write(dd);

    const cdh = Buffer.alloc(46 + name.length + 28);
    cdh.writeUInt32LE(0x02014b50, 0);
    cdh.writeUInt16LE(45, 4);
    cdh.writeUInt16LE(45, 6);
    cdh.writeUInt16LE(8, 8);
    cdh.writeUInt16LE(method, 10);
    cdh.writeUInt16LE(0, 12);
    cdh.writeUInt16LE(0x21, 14);
    cdh.writeUInt32LE(crc, 16);
    cdh.writeUInt32LE(0xffffffff, 20);
    cdh.writeUInt32LE(0xffffffff, 24);
    cdh.writeUInt16LE(name.length, 28);
    cdh.writeUInt16LE(28, 30);
    cdh.writeUInt32LE(0xffffffff, 42);
    cdh.set(name, 46);
    const x = 46 + name.length;
    cdh.writeUInt16LE(0x0001, x);
    cdh.writeUInt16LE(24, x + 2);
    cdh.writeBigUInt64LE(BigInt(data.length), x + 4);
    cdh.writeBigUInt64LE(BigInt(body.length), x + 12);
    cdh.writeBigUInt64LE(BigInt(start), x + 20);
    this.central.push(cdh);
    this.count++;
  }

  close(): void {
    const cdStart = this.offset;
    for (const c of this.central) this.write(c);
    const cdSize = this.offset - cdStart;
    const eocd64At = this.offset;
    const eocd64 = Buffer.alloc(56);
    eocd64.writeUInt32LE(0x06064b50, 0);
    eocd64.writeBigUInt64LE(44n, 4);
    eocd64.writeUInt16LE(45, 12);
    eocd64.writeUInt16LE(45, 14);
    eocd64.writeBigUInt64LE(BigInt(this.count), 24);
    eocd64.writeBigUInt64LE(BigInt(this.count), 32);
    eocd64.writeBigUInt64LE(BigInt(cdSize), 40);
    eocd64.writeBigUInt64LE(BigInt(cdStart), 48);
    this.write(eocd64);
    const locator = Buffer.alloc(20);
    locator.writeUInt32LE(0x07064b50, 0);
    locator.writeBigUInt64LE(BigInt(eocd64At), 8);
    locator.writeUInt32LE(1, 16);
    this.write(locator);
    const eocd = Buffer.alloc(22, 0xff);
    eocd.writeUInt32LE(0x06054b50, 0);
    eocd.writeUInt16LE(0, 20);
    this.write(eocd);
    closeSync(this.fd);
  }
}

/**
 * Writes an unsigned package. `read` loads a payload file when its turn comes, so only one file is in memory at
 * a time. Returns the block map, for inspection.
 */
export function writeAppx(
  out: string,
  payload: readonly string[],
  read: (path: string) => Uint8Array,
  manifest: string,
): { blockMap: string; bytes: number } {
  const zip = new ZipWriter(out);
  const entries: BlockMapFile[] = [];
  try {
    const add = (path: string, data: Uint8Array, mapped: boolean) => {
      const deflated = compressible(path);
      const { body, entry } = encodeFile({ path, data }, deflated);
      zip.add(path, data, body, deflated);
      if (mapped) entries.push(entry);
    };
    for (const path of payload) add(path, read(path), true);
    add("AppxManifest.xml", new TextEncoder().encode(manifest), true);
    const blockMap = blockMapXml(entries);
    add("AppxBlockMap.xml", new TextEncoder().encode(blockMap), false);
    add(CONTENT_TYPES, new TextEncoder().encode(contentTypesXml(payload)), false);
    zip.close();
    return { blockMap, bytes: zip.offset };
  } catch (e) {
    try {
      zip.close();
    } catch {}
    throw e;
  }
}
