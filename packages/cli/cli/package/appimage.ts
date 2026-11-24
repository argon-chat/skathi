// `sk package` → `<name>-<version>-<arch>.AppImage`: the type-2 runtime followed by a squashfs of the AppDir.
//
//   AppRun -> usr/lib/<exe>/<exe>        <id>.desktop  <id>.png  .DirIcon -> <id>.png
//   usr/lib/<exe>/                        the `sk build` folder: exe, packs, icu.skpk, the app's libraries
//   usr/bin/<exe> -> ../lib/<exe>/<exe>
//   usr/share/{applications,icons/hicolor,metainfo,mime/packages}/
//
// The squashfs is written here (squashfs.ts), so packaging needs no mksquashfs and runs on any OS. The
// runtime is AppImage/type2-runtime at a pinned release, checked by sha256 and cached; update information
// goes into its `.upd_info` section and a GnuPG signature into `.sha256_sig` / `.sig_key`, as appimagetool does.

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  readSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
  writeSync,
} from "node:fs";
import { basename, dirname, join, resolve, sep } from "node:path";
import { cacheRoot } from "../host";
import { sha256 } from "../host-fetch";
import { desktopEntry, MAIN_CATEGORIES, metainfo, mimeTypeOf, shareFiles } from "./freedesktop";
import { dir, put, writeSquashfs, type SquashDir } from "./squashfs";
import type {
  AppImageOptions,
  Artifact,
  OptionsContext,
  PackageCheckContext,
  PackageContext,
  PackageFormat,
  Problem,
} from "./types";

export const RUNTIME_RELEASE = "20251108";
export const RUNTIMES: Readonly<Record<string, { machine: string; sha256: string; size: number }>> = {
  x64: { machine: "x86_64", sha256: "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d", size: 944632 },
  arm64: {
    machine: "aarch64",
    sha256: "00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444",
    size: 936456,
  },
};
const RUNTIME_URL = "https://github.com/AppImage/type2-runtime/releases/download";

const UPDATE_PREFIXES = ["zsync|", "gh-releases-zsync|", "pling-v1-zsync|"];
const UPDATE_INFO_BYTES = 1024;
const CATEGORY = /^[A-Za-z0-9-]+$/;
const KEYS = [
  "categories",
  "compression",
  "runtime",
  "updateInformation",
  "signKey",
  "license",
  "homepage",
  "fileName",
] as const;

type Resolved = Required<AppImageOptions>;

function options(o: AppImageOptions): Resolved {
  return {
    categories: o.categories ?? ["Utility"],
    compression: o.compression ?? "zstd",
    runtime: o.runtime ?? null,
    updateInformation: o.updateInformation ?? null,
    signKey: o.signKey ?? null,
    license: o.license ?? null,
    homepage: o.homepage ?? null,
    fileName: o.fileName ?? null,
  };
}

function parseOptions(raw: Record<string, unknown>, ctx: OptionsContext, problems: Problem[]): AppImageOptions {
  const error = (key: string, message: string) =>
    problems.push({
      code: "SK8001",
      severity: "error",
      key: `${ctx.at}.${key}`,
      message: `"${ctx.at}.${key}" ${message}`,
    });
  const text = (key: string): string | null => {
    const v = raw[key];
    if (v === undefined || v === null) return null;
    if (typeof v === "string" && v.trim()) return v.trim();
    error(key, "must be a non-empty string");
    return null;
  };
  const out = options({});
  const categories = raw.categories;
  if (categories !== undefined) {
    if (
      !Array.isArray(categories) ||
      !categories.length ||
      categories.some((c) => typeof c !== "string" || !CATEGORY.test(c))
    ) {
      error("categories", 'must be a non-empty list of Desktop Menu categories, e.g. ["Office"]');
    } else out.categories = [...new Set(categories as string[])];
  }
  if (raw.compression !== undefined) {
    if (raw.compression === "zstd" || raw.compression === "gzip") out.compression = raw.compression;
    else error("compression", 'must be "zstd" or "gzip"');
  }
  const runtime = text("runtime");
  out.runtime = runtime && resolve(ctx.app.appRoot, runtime);
  out.updateInformation = text("updateInformation");
  if (out.updateInformation) {
    const u = out.updateInformation;
    if (!UPDATE_PREFIXES.some((p) => u.startsWith(p))) {
      error("updateInformation", `must start with ${UPDATE_PREFIXES.map((p) => `"${p}"`).join(", ")}`);
    } else if (new TextEncoder().encode(u).length > UPDATE_INFO_BYTES) {
      error("updateInformation", `is longer than the ${UPDATE_INFO_BYTES} bytes the runtime has room for`);
    }
  }
  out.signKey = text("signKey");
  out.license = text("license");
  out.homepage = text("homepage");
  if (out.homepage && !/^https?:\/\/\S+$/.test(out.homepage)) error("homepage", "must be an http or https URL");
  out.fileName = text("fileName");
  if (out.fileName && !/^[^/\\]+\.AppImage$/.test(out.fileName)) {
    error("fileName", 'must be a file name ending in ".AppImage", without a folder');
  }
  return out;
}

function validate(ctx: PackageCheckContext): Problem[] {
  const o = options(ctx.config.linux.appimage);
  const at = "package.linux.appimage";
  const problems: Problem[] = [];
  if (!o.categories.some((c) => MAIN_CATEGORIES.has(c))) {
    problems.push({
      code: "SK8002",
      severity: "warning",
      key: `${at}.categories`,
      message: `"${at}.categories" has no main category, so menus list the app under "Other"; add one of ${[...MAIN_CATEGORIES].join(", ")}`,
    });
  }
  if (o.runtime && !existsSync(o.runtime)) {
    problems.push({
      code: "SK8001",
      severity: "error",
      key: `${at}.runtime`,
      message: `"${at}.runtime": ${o.runtime} does not exist; point it at an AppImage type-2 runtime file, or leave it out to download the pinned one`,
    });
  }
  if (ctx.config.fileAssociations.length && ctx.config.protocols.length) {
    problems.push({
      code: "SK8002",
      severity: "warning",
      key: "package.protocols",
      message:
        "an AppImage has one desktop entry for files and links: a link to the app's schemes arrives as a plain argument " +
        "(launch.argv, not launch.url) because the app also opens files",
    });
  }
  return problems;
}

export interface ElfSection {
  offset: number;
  size: number;
}

/** A 64-bit little-endian ELF's sections by name, and where the file ends by its own headers. */
export function elfSections(bytes: Uint8Array): { end: number; sections: Map<string, ElfSection> } {
  if (bytes.length < 64 || bytes[0] !== 0x7f || bytes[1] !== 0x45 || bytes[2] !== 0x4c || bytes[3] !== 0x46) {
    throw new Error("not an ELF file");
  }
  if (bytes[4] !== 2 || bytes[5] !== 1) throw new Error("not a 64-bit little-endian ELF file");
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const shoff = Number(v.getBigUint64(0x28, true));
  const shentsize = v.getUint16(0x3a, true);
  const shnum = v.getUint16(0x3c, true);
  const shstrndx = v.getUint16(0x3e, true);
  const end = shoff + shentsize * shnum;
  if (end > bytes.length) throw new Error("the ELF section headers run past the end of the file");
  const header = (i: number) => {
    const at = shoff + i * shentsize;
    return {
      name: v.getUint32(at, true),
      offset: Number(v.getBigUint64(at + 24, true)),
      size: Number(v.getBigUint64(at + 32, true)),
    };
  };
  const strtab = header(shstrndx);
  const sections = new Map<string, ElfSection>();
  for (let i = 1; i < shnum; i++) {
    const h = header(i);
    let stop = strtab.offset + h.name;
    while (stop < bytes.length && bytes[stop] !== 0) stop++;
    const name = new TextDecoder().decode(bytes.subarray(strtab.offset + h.name, stop));
    sections.set(name, { offset: h.offset, size: h.size });
  }
  return { end, sections };
}

/** The runtime's checks: a type-2 AppImage runtime with the sections the image writes into. */
export function checkRuntime(bytes: Uint8Array): { end: number; sections: Map<string, ElfSection> } {
  const elf = elfSections(bytes);
  if (bytes[8] !== 0x41 || bytes[9] !== 0x49 || bytes[10] !== 0x02) throw new Error("not an AppImage type-2 runtime");
  for (const name of [".upd_info", ".sha256_sig", ".sig_key"]) {
    if (!elf.sections.has(name)) throw new Error(`the runtime has no ${name} section`);
  }
  return elf;
}

/** The newest `GLIBC_x.y` symbol version an executable asks for, from its `.dynstr`; null when none. */
export function glibcFloor(exe: Uint8Array): [number, number] | null {
  const dynstr = elfSections(exe).sections.get(".dynstr");
  if (!dynstr) return null;
  const text = new TextDecoder("latin1").decode(exe.subarray(dynstr.offset, dynstr.offset + dynstr.size));
  let best: [number, number] | null = null;
  for (const m of text.matchAll(/GLIBC_(\d+)\.(\d+)/g)) {
    const v: [number, number] = [Number(m[1]), Number(m[2])];
    if (!best || v[0] > best[0] || (v[0] === best[0] && v[1] > best[1])) best = v;
  }
  return best;
}

/** First release of each distribution that ships a glibc of at least the key. */
const DISTROS: readonly [string, readonly [number, string][]][] = [
  [
    "Ubuntu",
    [
      [31, "20.04"],
      [35, "22.04"],
      [39, "24.04"],
      [41, "25.04"],
      [42, "25.10"],
      [43, "26.04"],
    ],
  ],
  [
    "Debian",
    [
      [31, "11"],
      [36, "12"],
      [41, "13"],
    ],
  ],
  [
    "Fedora",
    [
      [33, "34"],
      [34, "35"],
      [35, "36"],
      [36, "37"],
      [37, "38"],
      [38, "39"],
      [39, "40"],
      [40, "41"],
      [41, "42"],
      [42, "43"],
    ],
  ],
  [
    "RHEL",
    [
      [28, "8"],
      [34, "9"],
      [39, "10"],
    ],
  ],
];

export function distributionsFor([major, minor]: [number, number]): string[] {
  if (major !== 2) return [];
  return DISTROS.flatMap(([name, releases]) => {
    const first = releases.find(([g]) => g >= minor);
    return first ? [`${name} ${first[1]}`] : [];
  });
}

async function runtimeBytes(o: Resolved, arch: string, log: PackageContext["log"]): Promise<Uint8Array> {
  if (o.runtime) return new Uint8Array(readFileSync(o.runtime));
  const pin = RUNTIMES[arch];
  if (!pin) throw new Error(`no pinned AppImage runtime for ${arch}; set package.linux.appimage.runtime to one`);
  const cached = join(dirname(cacheRoot()), "appimage-runtime", RUNTIME_RELEASE, `runtime-${pin.machine}`);
  if (existsSync(cached)) {
    const bytes = new Uint8Array(readFileSync(cached));
    if (sha256(bytes) === pin.sha256) return bytes;
  }
  if (process.env.SK_HOST_OFFLINE === "1") {
    throw new Error(
      `SK_HOST_OFFLINE=1 and no cached AppImage runtime: set package.linux.appimage.runtime to runtime-${pin.machine} ` +
        `from github.com/AppImage/type2-runtime release ${RUNTIME_RELEASE}`,
    );
  }
  const url = `${RUNTIME_URL}/${RUNTIME_RELEASE}/runtime-${pin.machine}`;
  log.info(`downloading the AppImage runtime ${RUNTIME_RELEASE} (${pin.machine})`);
  const res = await fetch(url, { headers: { "User-Agent": "skathi-cli" } });
  if (!res.ok) throw new Error(`downloading ${url}: HTTP ${res.status}`);
  const bytes = new Uint8Array(await res.arrayBuffer());
  if (bytes.length !== pin.size || sha256(bytes) !== pin.sha256) {
    throw new Error(`${url} does not match its pinned sha256; nothing was cached`);
  }
  mkdirSync(dirname(cached), { recursive: true });
  writeFileSync(`${cached}.partial`, bytes);
  rmSync(cached, { force: true });
  renameSync(`${cached}.partial`, cached);
  return bytes;
}

const isExecutable = (head: Uint8Array) =>
  (head[0] === 0x7f && head[1] === 0x45 && head[2] === 0x4c && head[3] === 0x46) ||
  (head[0] === 0x23 && head[1] === 0x21);

function headOf(path: string): Uint8Array {
  const fd = openSync(path, "r");
  try {
    const head = new Uint8Array(4);
    return head.subarray(0, readSync(fd, head, 0, 4, 0));
  } finally {
    closeSync(fd);
  }
}

/** The `sk build` folder as `usr/lib/<exe>/`, leaving out dotfiles and the package output. */
function addBuild(root: SquashDir, ctx: PackageContext, into: string): void {
  const skip = [ctx.outDir, ctx.workDir].map((p) => resolve(p));
  const inside = (path: string, d: string) => path === d || path.startsWith(d + sep);
  const walk = (from: string, to: string) => {
    for (const name of readdirSync(from).sort()) {
      if (name.startsWith(".")) continue;
      const path = resolve(from, name);
      if (skip.some((d) => inside(path, d))) continue;
      const st = statSync(path);
      if (st.isDirectory()) walk(path, `${to}/${name}`);
      else if (st.isFile() && !name.endsWith(".AppImage")) {
        put(root, `${to}/${name}`, { type: "file", source: path, executable: isExecutable(headOf(path)) });
      }
    }
  };
  walk(ctx.buildDir, into);
}

export function appImageName(ctx: Pick<PackageContext, "config" | "arch">, o: AppImageOptions): string {
  if (o.fileName) return o.fileName;
  const machine = RUNTIMES[ctx.arch]?.machine ?? ctx.arch;
  return `${ctx.config.identity.displayName.replace(/\s+/g, "_")}-${ctx.config.identity.version}-${machine}.AppImage`;
}

/** The AppDir, in memory. */
export function appDir(ctx: PackageContext, o: AppImageOptions, exeBytes?: Uint8Array): SquashDir {
  const opts = options(o);
  const { identity } = ctx.config;
  const exe = basename(ctx.exe);
  const lib = `usr/lib/${exe}`;
  const root = dir();
  addBuild(root, ctx, lib);
  put(root, `${lib}/${exe}`, { type: "file", source: exeBytes ?? ctx.exe, executable: true });
  put(root, "AppRun", { type: "symlink", target: `${lib}/${exe}` });
  put(root, `usr/bin/${exe}`, { type: "symlink", target: `../lib/${exe}/${exe}` });

  const icon = ctx.icon(256);
  const entry = desktopEntry({
    name: identity.displayName,
    comment: identity.description,
    exec: exe,
    icon: icon ? identity.id : null,
    categories: opts.categories,
    fileAssociations: ctx.config.fileAssociations,
    protocols: ctx.config.protocols,
    wmClass: ctx.app.launch.id,
    extra: { "X-AppImage-Version": identity.version },
  });
  const meta = metainfo({
    id: identity.id,
    name: identity.displayName,
    summary: identity.description,
    developer: identity.publisher,
    version: identity.version,
    license: opts.license,
    homepage: opts.homepage,
    binary: exe,
    mimeTypes: ctx.config.fileAssociations.map(mimeTypeOf),
    date: new Date(mtime() * 1000).toISOString().slice(0, 10),
  });
  for (const [path, bytes] of shareFiles(ctx, entry, meta)) {
    put(root, `usr/share/${path}`, { type: "file", source: bytes, executable: false });
  }
  put(root, `${identity.id}.desktop`, { type: "file", source: new TextEncoder().encode(entry), executable: false });
  if (icon) {
    put(root, `${identity.id}.png`, { type: "file", source: icon, executable: false });
    put(root, ".DirIcon", { type: "symlink", target: `${identity.id}.png` });
  }
  return root;
}

const mtime = () => {
  const epoch = Number(process.env.SOURCE_DATE_EPOCH);
  return Number.isFinite(epoch) && epoch > 0 ? Math.floor(epoch) : Math.floor(Date.now() / 1000);
};

function writeAt(path: string, offset: number, bytes: Uint8Array): void {
  const fd = openSync(path, "r+");
  try {
    writeSync(fd, bytes, 0, bytes.length, offset);
  } finally {
    closeSync(fd);
  }
}

function gpg(args: string[], input?: string): Uint8Array {
  const exe = Bun.which("gpg");
  if (!exe) {
    throw new Error(
      'package.linux.appimage.signKey needs GnuPG: put gpg on PATH, or leave "signKey" out for an unsigned image',
    );
  }
  const r = spawnSync(exe, ["--batch", ...args], { input, maxBuffer: 1 << 20 });
  if (r.error) throw new Error(`gpg: ${r.error.message}`);
  if (r.status !== 0) throw new Error(`gpg ${args.join(" ")} failed: ${r.stderr.toString().trim()}`);
  return new Uint8Array(r.stdout);
}

/** appimagetool's signature: a detached, armored GnuPG signature of the image's sha256 (hex) with both sections empty. */
function signImage(path: string, key: string, sections: Map<string, ElfSection>): void {
  const digest = sha256(new Uint8Array(readFileSync(path)));
  const signature = gpg(["--armor", "--textmode", "--detach-sign", "--local-user", key], digest);
  const publicKey = gpg(["--armor", "--export", key]);
  if (!publicKey.length) throw new Error(`gpg has no public key ${JSON.stringify(key)}`);
  for (const [name, bytes] of [
    [".sha256_sig", signature],
    [".sig_key", publicKey],
  ] as const) {
    const s = sections.get(name)!;
    if (bytes.length > s.size)
      throw new Error(`the ${name} section holds ${s.size} bytes, the data is ${bytes.length}`);
    writeAt(path, s.offset, bytes);
  }
}

async function write(ctx: PackageContext): Promise<Artifact[]> {
  const o = options(ctx.config.linux.appimage);
  const exeBytes = new Uint8Array(readFileSync(ctx.exe));
  const floor = glibcFloor(exeBytes);
  if (floor) {
    const distros = distributionsFor(floor);
    ctx.log.info(
      `the host needs glibc ${floor.join(".")} or newer` + (distros.length ? `: ${distros.join(", ")} or later` : ""),
    );
  }
  const runtime = await runtimeBytes(o, ctx.arch, ctx.log);
  const { end, sections } = checkRuntime(runtime);
  const image = new Uint8Array(runtime.subarray(0, end));
  if (o.updateInformation) {
    const upd = sections.get(".upd_info")!;
    image.fill(0, upd.offset, upd.offset + upd.size);
    image.set(new TextEncoder().encode(o.updateInformation), upd.offset);
  }

  const fs = writeSquashfs(appDir(ctx, o, exeBytes), { compression: o.compression, mtime: mtime() });
  const out = join(ctx.outDir, appImageName(ctx, o));
  const partial = `${out}.partial`;
  const fd = openSync(partial, "w");
  try {
    writeSync(fd, image);
    for (const chunk of fs) writeSync(fd, chunk);
  } finally {
    closeSync(fd);
  }
  chmodSync(partial, 0o755);
  rmSync(out, { force: true });
  renameSync(partial, out);
  ctx.log.info(
    `${basename(out)}: ${(statSync(out).size / 2 ** 20).toFixed(1)} MiB, squashfs (${o.compression}) at ${end}`,
  );

  let signed = false;
  if (o.signKey) {
    signImage(out, o.signKey, sections);
    signed = true;
  }
  if (ctx.sign) {
    await ctx.sign(out, "appimage");
    signed = true;
  }
  const artifacts: Artifact[] = [{ path: out, kind: "installer", format: "appimage", signed }];

  if (o.updateInformation) {
    const zsyncmake = Bun.which("zsyncmake");
    if (zsyncmake) {
      const r = spawnSync(zsyncmake, ["-u", basename(out), "-o", `${out}.zsync`, out], { cwd: ctx.outDir });
      if (r.status !== 0) throw new Error(`zsyncmake failed: ${r.stderr?.toString().trim()}`);
      artifacts.push({ path: `${out}.zsync`, kind: "update", format: "appimage", signed: false });
    } else {
      ctx.log.warn(
        "zsyncmake is not on PATH: no .zsync was written beside the image, so updaters find nothing to read",
      );
    }
  }
  return artifacts;
}

export const format: PackageFormat<"appimage"> = {
  name: "appimage",
  platform: "linux",
  optionKeys: KEYS,
  parseOptions,
  validate,
  write,
};
