// The AppImage format: the squashfs read back by an independent reader, the runtime's sections, and a
// whole image from a fake runtime and build. Runs on any OS.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { inflateSync } from "node:zlib";
import type { AppConfig } from "../cli/config";
import { appImageName, checkRuntime, distributionsFor, format, glibcFloor } from "../cli/package/appimage";
import { desktopEntry } from "../cli/package/freedesktop";
import { dir, put, SQUASHFS_MAGIC, writeSquashfs, type Compression } from "../cli/package/squashfs";
import type { PackageConfig, PackageContext, Problem } from "../cli/package/types";

const root = mkdtempSync(join(tmpdir(), "sk-appimage-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

type Read =
  | { type: "dir"; mode: number; entries: Map<string, Read> }
  | { type: "file"; mode: number; bytes: Uint8Array }
  | { type: "symlink"; target: string };

/** A squashfs 4.0 reader written from the format description, independent of the writer. */
function readSquashfs(image: Uint8Array): { root: Read; inodes: number } {
  const v = new DataView(image.buffer, image.byteOffset, image.byteLength);
  expect(v.getUint32(0, true)).toBe(SQUASHFS_MAGIC);
  const inodes = v.getUint32(4, true);
  const blockSize = v.getUint32(12, true);
  const compressor = v.getUint16(20, true);
  const decompress = (b: Uint8Array) => (compressor === 6 ? Bun.zstdDecompressSync(b) : inflateSync(b));
  const u64 = (at: number) => Number(v.getBigUint64(at, true));
  const rootRef = v.getBigUint64(32, true);
  const inodeStart = u64(64);
  const dirStart = u64(72);
  const fragStart = u64(80);
  expect(u64(40)).toBeLessThanOrEqual(image.length);

  const table = (start: number, end: number) => {
    const out: number[] = [];
    const at = new Map<number, number>();
    for (let p = start; p < end;) {
      const h = v.getUint16(p, true);
      const len = h & 0x7fff;
      const raw = image.subarray(p + 2, p + 2 + len);
      at.set(p - start, out.length);
      out.push(...(h & 0x8000 ? raw : decompress(raw)));
      p += 2 + len;
    }
    return { bytes: Uint8Array.from(out), at };
  };
  const it = table(inodeStart, dirStart);
  const dt = table(dirStart, fragStart);
  const iv = new DataView(it.bytes.buffer);
  const dv = new DataView(dt.bytes.buffer);

  const inode = (block: number, offset: number): Read => {
    const p = it.at.get(block)! + offset;
    const type = iv.getUint16(p, true);
    const mode = iv.getUint16(p + 2, true);
    if (type === 2) {
      const start = iv.getUint32(p + 16, true);
      expect(iv.getUint32(p + 20, true)).toBe(0xffffffff);
      const size = iv.getUint32(p + 28, true);
      const bytes = new Uint8Array(size);
      let at = start;
      for (let i = 0, filled = 0; filled < size; i++) {
        const word = iv.getUint32(p + 32 + 4 * i, true);
        const len = word & 0xffffff;
        const raw = image.subarray(at, at + len);
        const block = word & (1 << 24) ? raw : decompress(raw);
        expect(block.length).toBe(Math.min(blockSize, size - filled));
        bytes.set(block, filled);
        filled += block.length;
        at += len;
      }
      return { type: "file", mode, bytes };
    }
    if (type === 3) {
      const len = iv.getUint32(p + 20, true);
      return { type: "symlink", target: new TextDecoder().decode(it.bytes.subarray(p + 24, p + 24 + len)) };
    }
    let listBlock: number, size: number, listOffset: number;
    if (type === 1) {
      listBlock = iv.getUint32(p + 16, true);
      size = iv.getUint16(p + 24, true);
      listOffset = iv.getUint16(p + 26, true);
    } else {
      expect(type).toBe(8);
      size = iv.getUint32(p + 20, true);
      listBlock = iv.getUint32(p + 24, true);
      listOffset = iv.getUint16(p + 34, true);
    }
    const entries = new Map<string, Read>();
    let q = dt.at.get(listBlock)! + listOffset;
    const end = q + size - 3;
    while (q < end) {
      const count = dv.getUint32(q, true) + 1;
      const start = dv.getUint32(q + 4, true);
      q += 12;
      for (let i = 0; i < count; i++) {
        const offset = dv.getUint16(q, true);
        const nameLen = dv.getUint16(q + 6, true) + 1;
        const name = new TextDecoder().decode(dt.bytes.subarray(q + 8, q + 8 + nameLen));
        entries.set(name, inode(start, offset));
        q += 8 + nameLen;
      }
    }
    return { type: "dir", mode, entries };
  };
  return { root: inode(Number(rootRef >> 16n), Number(rootRef & 0xffffn)), inodes };
}

const at = (node: Read, path: string): Read => {
  let n = node;
  for (const part of path.split("/")) {
    if (n.type !== "dir") throw new Error(`${part}: parent is not a directory`);
    const next = n.entries.get(part);
    if (!next) throw new Error(`no ${path}`);
    n = next;
  }
  return n;
};
const fileAt = (node: Read, path: string) => {
  const n = at(node, path);
  if (n.type !== "file") throw new Error(`${path} is a ${n.type}`);
  return n;
};
const concat = (chunks: Uint8Array[]) => {
  const out = new Uint8Array(chunks.reduce((s, c) => s + c.length, 0));
  let o = 0;
  for (const c of chunks) {
    out.set(c, o);
    o += c.length;
  }
  return out;
};

function noise(n: number, seed: number): Uint8Array {
  const out = new Uint8Array(n);
  let x = seed;
  for (let i = 0; i < n; i++) {
    x = (x * 1103515245 + 12345) >>> 0;
    out[i] = x >>> 24;
  }
  return out;
}

describe("squashfs", () => {
  for (const compression of ["zstd", "gzip"] as Compression[]) {
    test(`${compression}: every file, mode and link reads back`, () => {
      const blockSize = 4096;
      const tree = dir();
      const files = new Map<string, Uint8Array>([
        ["empty", new Uint8Array(0)],
        ["exact", noise(blockSize * 2, 1)],
        ["partial", noise(blockSize * 3 + 17, 2)],
        ["zeros", new Uint8Array(blockSize + 5)],
        ["a/b/c/deep.txt", new TextEncoder().encode("deep")],
      ]);
      for (const [path, bytes] of files) put(tree, path, { type: "file", source: bytes, executable: path === "exact" });
      // More names than one directory header holds, and more inodes than one 8 KiB metadata block.
      for (let i = 0; i < 700; i++)
        put(tree, `many/entry-${String(i).padStart(4, "0")}`, {
          type: "file",
          source: noise(i % 7, i),
          executable: false,
        });
      put(tree, "AppRun", { type: "symlink", target: "a/b/c/deep.txt" });

      const image = concat(writeSquashfs(tree, { compression, mtime: 1_700_000_000, blockSize }));
      expect(image.length % 4096).toBe(0);
      const { root: back, inodes } = readSquashfs(image);
      expect(inodes).toBe(files.size + 700 + 1 + 5);
      for (const [path, bytes] of files) expect(fileAt(back, path).bytes).toEqual(bytes);
      expect(fileAt(back, "exact").mode).toBe(0o755);
      expect(fileAt(back, "partial").mode).toBe(0o644);
      expect(at(back, "AppRun")).toEqual({ type: "symlink", target: "a/b/c/deep.txt" });
      const many = at(back, "many");
      if (many.type !== "dir") throw new Error("many is not a directory");
      expect([...many.entries.keys()]).toEqual([...Array(700)].map((_, i) => `entry-${String(i).padStart(4, "0")}`));
      expect(fileAt(back, "many/entry-0699").bytes).toEqual(noise(699 % 7, 699));
    });
  }

  test("a directory listing over 64 KiB takes the extended inode", () => {
    const tree = dir();
    for (let i = 0; i < 2000; i++)
      put(tree, `big/${"n".repeat(40)}-${i}`, { type: "file", source: new Uint8Array([i & 0xff]), executable: false });
    const { root: back } = readSquashfs(concat(writeSquashfs(tree, { compression: "gzip", mtime: 0 })));
    const big = at(back, "big");
    expect(big.type === "dir" && big.entries.size).toBe(2000);
    expect(fileAt(back, `big/${"n".repeat(40)}-1999`).bytes).toEqual(new Uint8Array([1999 & 0xff]));
  });
});

/** A 64-bit ELF with the given sections, the section headers at its end as in the AppImage runtime. */
function fakeElf(sections: [string, Uint8Array][], runtimeMagic: boolean): Uint8Array<ArrayBuffer> {
  const names = ["", ...sections.map(([n]) => n), ".shstrtab"];
  const shstr: number[] = [];
  const nameAt = names.map((n) => {
    const o = shstr.length;
    shstr.push(...new TextEncoder().encode(n), 0);
    return o;
  });
  const blobs = [...sections.map(([, d]) => d), Uint8Array.from(shstr)];
  const offsets: number[] = [];
  let p = 64;
  for (const b of blobs) {
    offsets.push(p);
    p += b.length;
  }
  const shoff = p;
  const out = new Uint8Array(shoff + names.length * 64);
  out.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1]);
  if (runtimeMagic) out.set([0x41, 0x49, 0x02], 8);
  const v = new DataView(out.buffer);
  v.setBigUint64(0x28, BigInt(shoff), true);
  v.setUint16(0x3a, 64, true);
  v.setUint16(0x3c, names.length, true);
  v.setUint16(0x3e, names.length - 1, true);
  blobs.forEach((b, i) => {
    out.set(b, offsets[i]!);
    const h = shoff + (i + 1) * 64;
    v.setUint32(h, nameAt[i + 1]!, true);
    v.setUint32(h + 4, 1, true);
    v.setBigUint64(h + 24, BigInt(offsets[i]!), true);
    v.setBigUint64(h + 32, BigInt(b.length), true);
  });
  return out;
}

const fakeRuntime = () =>
  fakeElf(
    [
      [".text", noise(300, 9)],
      [".upd_info", new Uint8Array(1024)],
      [".sha256_sig", new Uint8Array(1024)],
      [".sig_key", new Uint8Array(8192)],
    ],
    true,
  );

describe("runtime and host", () => {
  test("the image starts where the runtime's section headers end", () => {
    const runtime = fakeRuntime();
    const { end, sections } = checkRuntime(runtime);
    expect(end).toBe(runtime.length);
    expect(sections.get(".upd_info")!.size).toBe(1024);
  });

  test("an ELF without the AppImage magic is not taken for a runtime", () => {
    expect(() => checkRuntime(fakeElf([[".upd_info", new Uint8Array(4)]], false))).toThrow();
  });

  test("the glibc floor is the newest version the host's .dynstr asks for", () => {
    const dynstr = new TextEncoder().encode("\0libc.so.6\0GLIBC_2.2.5\0GLIBC_2.35\0GLIBC_2.4\0GLIBC_2.34\0");
    expect(glibcFloor(fakeElf([[".dynstr", dynstr]], false))).toEqual([2, 35]);
    expect(distributionsFor([2, 35])).toEqual(["Ubuntu 22.04", "Debian 12", "Fedora 36", "RHEL 10"]);
  });
});

describe("desktop entry", () => {
  const base = {
    name: "Notes; and more",
    comment: "Take notes",
    exec: "Notes",
    icon: "com.example.notes",
    categories: ["Office"],
    wmClass: "notes",
  };
  const doc = { extensions: ["note"], name: "Note", mimeType: null, role: "editor" as const, icon: null };
  const exec = (entry: string) => entry.split("\n").find((l) => l.startsWith("Exec="));

  test("a link-only app is started through the protocol marker, so the URL cannot carry host flags", () => {
    const entry = desktopEntry({ ...base, fileAssociations: [], protocols: [{ scheme: "notes", name: "Notes" }] });
    expect(exec(entry)).toBe("Exec=Notes --sk-protocol-launch -- %u");
    expect(entry).toContain("MimeType=x-scheme-handler/notes;\n");
  });

  test("files are passed as paths", () => {
    const entry = desktopEntry({ ...base, fileAssociations: [doc], protocols: [] });
    expect(exec(entry)).toBe("Exec=Notes %F");
    expect(entry).toContain("MimeType=application/x-note;\n");
  });
});

function packageConfig(outDir: string, appimage: PackageConfig["linux"]["appimage"]): PackageConfig {
  return {
    identity: {
      displayName: "My Notes",
      publisher: "Example Ltd",
      id: "com.example.notes",
      version: "1.2.3",
      description: "Take notes",
      copyright: "© Example Ltd",
    },
    icons: [],
    fileAssociations: [],
    protocols: [{ scheme: "notes", name: "Notes link" }],
    outDir,
    windows: { formats: [], msix: {} as PackageConfig["windows"]["msix"] },
    macos: { formats: [], dmg: {} },
    linux: { formats: ["appimage"], appimage },
  };
}

describe("format", () => {
  const app = {
    appRoot: root,
    launch: { id: "notes", singleInstance: true, raiseOnSecondInstance: true, protocols: ["notes"] },
  } as AppConfig;

  test("options with the wrong shape are errors at their key", () => {
    const problems: Problem[] = [];
    format.parseOptions(
      { updateInformation: "https://example.com/x.zsync", compression: "xz", fileName: "out/x.AppImage" },
      { app, identity: packageConfig(root, {}).identity, at: "package.linux.appimage" },
      problems,
    );
    expect(problems.map((p) => [p.severity, p.key])).toEqual([
      ["error", "package.linux.appimage.compression"],
      ["error", "package.linux.appimage.updateInformation"],
      ["error", "package.linux.appimage.fileName"],
    ]);
  });

  test("writes the runtime, update information and an AppDir that runs the build's exe", async () => {
    const build = join(root, "build");
    const outDir = join(root, "package");
    mkdirSync(join(build, "lib"), { recursive: true });
    mkdirSync(join(outDir, ".sk-package"), { recursive: true });
    const exe = fakeElf([[".dynstr", new TextEncoder().encode("\0GLIBC_2.35\0")]], false);
    writeFileSync(join(build, "Notes"), exe);
    writeFileSync(join(build, "icu.skpk"), noise(5000, 3));
    writeFileSync(join(build, "lib", "libsqlite3.so"), fakeElf([], false));
    writeFileSync(join(build, ".hidden"), "no");
    const runtimePath = join(root, "runtime-x86_64");
    const runtime = fakeRuntime();
    writeFileSync(runtimePath, runtime);

    const update = "gh-releases-zsync|example|notes|latest|My_Notes-*-x86_64.AppImage.zsync";
    const problems: Problem[] = [];
    const options = format.parseOptions(
      { runtime: runtimePath, updateInformation: update, categories: ["Office"], compression: "gzip" },
      { app, identity: packageConfig(outDir, {}).identity, at: "package.linux.appimage" },
      problems,
    );
    expect(problems).toEqual([]);
    const lines: string[] = [];
    const icon = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2, 3]);
    const ctx: PackageContext = {
      appRoot: root,
      app,
      config: packageConfig(outDir, options),
      platform: "linux",
      arch: "x64",
      buildDir: build,
      exe: join(build, "Notes"),
      appBundle: null,
      outDir,
      workDir: join(outDir, ".sk-package"),
      log: { info: (l) => lines.push(l), warn: (l) => lines.push(l) },
      icon: (size) => (size <= 256 ? icon : null),
      sign: null,
    };
    expect(format.validate(ctx)).toEqual([]);
    const artifacts = await format.write(ctx);
    const path = join(outDir, appImageName(ctx, options));
    expect(artifacts[0]).toEqual({ path, kind: "installer", format: "appimage", signed: false });
    expect(lines.some((l) => l.includes("2.35"))).toBe(true);

    const image = new Uint8Array(readFileSync(path));
    const { end, sections } = checkRuntime(runtime);
    const upd = sections.get(".upd_info")!;
    expect(image.subarray(0, upd.offset)).toEqual(runtime.subarray(0, upd.offset));
    expect(new TextDecoder().decode(image.subarray(upd.offset, upd.offset + update.length))).toBe(update);
    expect(image[upd.offset + update.length]).toBe(0);

    const { root: appDir } = readSquashfs(image.subarray(end));
    expect(at(appDir, "AppRun")).toEqual({ type: "symlink", target: "usr/lib/Notes/Notes" });
    expect(at(appDir, "usr/bin/Notes")).toEqual({ type: "symlink", target: "../lib/Notes/Notes" });
    const program = fileAt(appDir, "usr/lib/Notes/Notes");
    expect(program.bytes).toEqual(exe);
    expect(program.mode).toBe(0o755);
    expect(fileAt(appDir, "usr/lib/Notes/icu.skpk").bytes).toEqual(noise(5000, 3));
    expect(fileAt(appDir, "usr/lib/Notes/icu.skpk").mode).toBe(0o644);
    expect(fileAt(appDir, "usr/lib/Notes/lib/libsqlite3.so").mode).toBe(0o755);
    expect(() => at(appDir, "usr/lib/Notes/.hidden")).toThrow();
    expect(at(appDir, ".DirIcon")).toEqual({ type: "symlink", target: "com.example.notes.png" });
    expect(fileAt(appDir, "com.example.notes.png").bytes).toEqual(icon);
    expect(fileAt(appDir, "usr/share/icons/hicolor/256x256/apps/com.example.notes.png").bytes).toEqual(icon);
    expect(() => at(appDir, "usr/share/icons/hicolor/512x512")).toThrow();
    const entry = new TextDecoder().decode(fileAt(appDir, "com.example.notes.desktop").bytes);
    expect(fileAt(appDir, "usr/share/applications/com.example.notes.desktop").bytes).toEqual(
      fileAt(appDir, "com.example.notes.desktop").bytes,
    );
    expect(entry).toContain("Exec=Notes --sk-protocol-launch -- %u\n");
    expect(entry).toContain("Icon=com.example.notes\n");
    expect(entry).toContain("StartupWMClass=notes\n");
    expect(
      new TextDecoder().decode(fileAt(appDir, "usr/share/metainfo/com.example.notes.metainfo.xml").bytes),
    ).toContain('<launchable type="desktop-id">com.example.notes.desktop</launchable>');
  });
});
