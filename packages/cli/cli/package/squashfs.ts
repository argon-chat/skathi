// A squashfs 4.0 image written from an in-memory tree, so an AppImage needs neither mksquashfs nor a
// Linux machine to build, and modes and symlinks do not depend on the file system the build runs on.
//
//   superblock | data blocks | inode table | directory table | id table | id index
//
// No fragments (a file's last block is just shorter), no xattrs, no export table, every inode owned
// by root. Offsets are relative to the image's start, which is how the AppImage runtime mounts it.

import { readFileSync } from "node:fs";
import { deflateSync } from "node:zlib";

export type SquashNode = SquashDir | SquashFile | SquashSymlink;

export interface SquashDir {
  type: "dir";
  entries: Map<string, SquashNode>;
}

export interface SquashFile {
  type: "file";
  /** A path read when the image is written, or the bytes. */
  source: string | Uint8Array;
  executable: boolean;
}

export interface SquashSymlink {
  type: "symlink";
  target: string;
}

export type Compression = "zstd" | "gzip";

export interface SquashOptions {
  compression: Compression;
  /** Seconds since the epoch, for the superblock and every inode. */
  mtime: number;
  blockSize?: number;
}

export const SQUASHFS_MAGIC = 0x73717368;
export const COMPRESSOR_ID: Record<Compression, number> = { gzip: 1, zstd: 6 };
const METADATA_BLOCK = 8192;
const NO_FRAGMENTS = 0x0010;
const NO_XATTRS = 0x0200;
const NONE64 = 0xffffffffffffffffn;
const NONE32 = 0xffffffff;
const UNCOMPRESSED_DATA = 1 << 24;
const UNCOMPRESSED_METADATA = 0x8000;
export const INODE = { dir: 1, file: 2, symlink: 3, extDir: 8 } as const;

export const dir = (entries: Record<string, SquashNode> = {}): SquashDir => ({
  type: "dir",
  entries: new Map(Object.entries(entries)),
});

/** The node at `path` (slash-separated), creating directories on the way. */
export function put(root: SquashDir, path: string, node: SquashNode): void {
  const parts = path.split("/").filter(Boolean);
  const name = parts.pop();
  if (!name) throw new Error(`squashfs: empty path`);
  let at = root;
  for (const part of parts) {
    const next = at.entries.get(part) ?? dir();
    if (next.type !== "dir") throw new Error(`squashfs: ${part} in ${path} is not a directory`);
    at.entries.set(part, next);
    at = next;
  }
  at.entries.set(name, node);
}

function compressor(compression: Compression): (bytes: Uint8Array) => Uint8Array {
  return compression === "zstd"
    ? (bytes) => Bun.zstdCompressSync(bytes, { level: 15 })
    : (bytes) => deflateSync(bytes, { level: 9 });
}

class Out {
  chunks: Uint8Array[] = [];
  size = 0;
  push(bytes: Uint8Array) {
    this.chunks.push(bytes);
    this.size += bytes.length;
  }
}

class Metadata {
  private blocks = new Out();
  private buf = new Uint8Array(METADATA_BLOCK);
  private len = 0;
  constructor(private compress: (bytes: Uint8Array) => Uint8Array) {}

  /** Where the next byte lands: the block's offset in the table and the offset inside the block. */
  get position(): { block: number; offset: number } {
    return { block: this.blocks.size, offset: this.len };
  }

  write(bytes: Uint8Array) {
    let at = 0;
    while (at < bytes.length) {
      const n = Math.min(bytes.length - at, METADATA_BLOCK - this.len);
      this.buf.set(bytes.subarray(at, at + n), this.len);
      this.len += n;
      at += n;
      if (this.len === METADATA_BLOCK) this.flush();
    }
  }

  private flush() {
    if (!this.len) return;
    const raw = this.buf.slice(0, this.len);
    const packed = this.compress(raw);
    const stored = packed.length < raw.length ? packed : raw;
    const header = new Uint8Array(2);
    new DataView(header.buffer).setUint16(0, stored.length | (stored === raw ? UNCOMPRESSED_METADATA : 0), true);
    this.blocks.push(header);
    this.blocks.push(stored);
    this.len = 0;
  }

  finish(): Out {
    this.flush();
    return this.blocks;
  }
}

class Bytes {
  private parts: number[] = [];
  u8(v: number) {
    this.parts.push(v & 0xff);
    return this;
  }
  u16(v: number) {
    return this.u8(v).u8(v >>> 8);
  }
  u32(v: number) {
    return this.u16(v & 0xffff).u16(v >>> 16);
  }
  i16(v: number) {
    return this.u16(v & 0xffff);
  }
  raw(bytes: Uint8Array) {
    for (const b of bytes) this.parts.push(b);
    return this;
  }
  get length() {
    return this.parts.length;
  }
  done() {
    return Uint8Array.from(this.parts);
  }
}

interface Placed {
  node: SquashNode;
  number: number;
  /** Inode reference: metadata block offset << 16 | offset in the block. */
  block: number;
  offset: number;
  type: number;
}

const encoder = new TextEncoder();
const sorted = (d: SquashDir) =>
  [...d.entries]
    .map(([name, node]) => ({ name: encoder.encode(name), node }))
    .sort((a, b) => compareBytes(a.name, b.name));

function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) if (a[i] !== b[i]) return a[i]! - b[i]!;
  return a.length - b.length;
}

/** The image, as chunks to write in order. */
export function writeSquashfs(root: SquashDir, options: SquashOptions): Uint8Array[] {
  const blockSize = options.blockSize ?? 128 * 1024;
  const blockLog = Math.log2(blockSize);
  if (!Number.isInteger(blockLog) || blockLog < 12 || blockLog > 20)
    throw new Error(`squashfs: bad block size ${blockSize}`);
  const compress = compressor(options.compression);
  const mtime = options.mtime >>> 0;

  const numbers = new Map<SquashNode, number>();
  let count = 0;
  const number = (node: SquashNode) => {
    if (node.type === "dir") for (const { node: child } of sorted(node)) number(child);
    if (numbers.has(node)) throw new Error("squashfs: a node appears twice in the tree");
    numbers.set(node, ++count);
  };
  number(root);

  const data = new Out();
  data.push(new Uint8Array(96));
  const inodes = new Metadata(compress);
  const dirs = new Metadata(compress);

  const header = (b: Bytes, type: number, mode: number, n: number) =>
    b.u16(type).u16(mode).u16(0).u16(0).u32(mtime).u32(n);

  const writeFile = (file: SquashFile, n: number): Placed => {
    const bytes = typeof file.source === "string" ? new Uint8Array(readFileSync(file.source)) : file.source;
    if (bytes.length > NONE32) throw new Error("squashfs: a file of 4 GiB or more needs an extended inode");
    const start = bytes.length ? data.size : 0;
    const sizes: number[] = [];
    for (let at = 0; at < bytes.length; at += blockSize) {
      const raw = bytes.subarray(at, at + blockSize);
      const packed = compress(raw);
      if (packed.length < raw.length) {
        data.push(packed);
        sizes.push(packed.length);
      } else {
        data.push(raw.slice());
        sizes.push(raw.length | UNCOMPRESSED_DATA);
      }
    }
    if (data.size > NONE32) throw new Error("squashfs: the image passed 4 GiB");
    const b = header(new Bytes(), INODE.file, file.executable ? 0o755 : 0o644, n)
      .u32(start)
      .u32(NONE32)
      .u32(0)
      .u32(bytes.length);
    for (const s of sizes) b.u32(s);
    return place(file, n, INODE.file, b.done());
  };

  const place = (node: SquashNode, n: number, type: number, inode: Uint8Array): Placed => {
    const { block, offset } = inodes.position;
    inodes.write(inode);
    return { node, number: n, block, offset, type };
  };

  const parentOf = new Map<SquashNode, number>();
  const visit = (node: SquashNode): Placed => {
    const n = numbers.get(node)!;
    if (node.type === "file") return writeFile(node, n);
    if (node.type === "symlink") {
      const target = encoder.encode(node.target);
      return place(
        node,
        n,
        INODE.symlink,
        header(new Bytes(), INODE.symlink, 0o777, n).u32(1).u32(target.length).raw(target).done(),
      );
    }
    const children = sorted(node).map(({ name, node: child }) => {
      parentOf.set(child, n);
      return { name, placed: visit(child) };
    });
    const listing = dirs.position;
    const table = new Bytes();
    for (let i = 0; i < children.length;) {
      const first = children[i]!.placed;
      let j = i;
      while (
        j < children.length &&
        j - i < 256 &&
        children[j]!.placed.block === first.block &&
        Math.abs(children[j]!.placed.number - first.number) < 0x8000
      )
        j++;
      table
        .u32(j - i - 1)
        .u32(first.block)
        .u32(first.number);
      for (const { name, placed } of children.slice(i, j)) {
        table
          .u16(placed.offset)
          .i16(placed.number - first.number)
          .u16(placed.type)
          .u16(name.length - 1)
          .raw(name);
      }
      i = j;
    }
    const size = table.length + 3;
    dirs.write(table.done());
    const links = 2 + children.filter((c) => c.placed.node.type === "dir").length;
    const parent = parentOf.get(node) ?? count + 1;
    const mode = 0o755;
    if (size <= 0xffff) {
      return place(
        node,
        n,
        INODE.dir,
        header(new Bytes(), INODE.dir, mode, n)
          .u32(listing.block)
          .u32(links)
          .u16(size)
          .u16(listing.offset)
          .u32(parent)
          .done(),
      );
    }
    return place(
      node,
      n,
      INODE.extDir,
      header(new Bytes(), INODE.extDir, mode, n)
        .u32(links)
        .u32(size)
        .u32(listing.block)
        .u32(parent)
        .u16(0)
        .u16(listing.offset)
        .u32(NONE32)
        .done(),
    );
  };
  const rootPlaced = visit(root);

  const inodeTable = inodes.finish();
  const dirTable = dirs.finish();
  const ids = new Metadata(compress);
  ids.write(new Uint8Array(4));
  const idTable = ids.finish();

  const inodeStart = data.size;
  const dirStart = inodeStart + inodeTable.size;
  const fragmentStart = dirStart + dirTable.size;
  const idBlockStart = fragmentStart;
  const idIndexStart = idBlockStart + idTable.size;
  const bytesUsed = idIndexStart + 8;

  const idIndex = new Uint8Array(8);
  new DataView(idIndex.buffer).setBigUint64(0, BigInt(idBlockStart), true);

  const sb = new DataView(data.chunks[0]!.buffer);
  sb.setUint32(0, SQUASHFS_MAGIC, true);
  sb.setUint32(4, count, true);
  sb.setUint32(8, mtime, true);
  sb.setUint32(12, blockSize, true);
  sb.setUint32(16, 0, true);
  sb.setUint16(20, COMPRESSOR_ID[options.compression], true);
  sb.setUint16(22, blockLog, true);
  sb.setUint16(24, NO_FRAGMENTS | NO_XATTRS, true);
  sb.setUint16(26, 1, true);
  sb.setUint16(28, 4, true);
  sb.setUint16(30, 0, true);
  sb.setBigUint64(32, (BigInt(rootPlaced.block) << 16n) | BigInt(rootPlaced.offset), true);
  sb.setBigUint64(40, BigInt(bytesUsed), true);
  sb.setBigUint64(48, BigInt(idIndexStart), true);
  sb.setBigUint64(56, NONE64, true);
  sb.setBigUint64(64, BigInt(inodeStart), true);
  sb.setBigUint64(72, BigInt(dirStart), true);
  sb.setBigUint64(80, BigInt(fragmentStart), true);
  sb.setBigUint64(88, NONE64, true);

  const pad = (4096 - (bytesUsed % 4096)) % 4096;
  return [
    ...data.chunks,
    ...inodeTable.chunks,
    ...dirTable.chunks,
    ...idTable.chunks,
    idIndex,
    ...(pad ? [new Uint8Array(pad)] : []),
  ];
}
