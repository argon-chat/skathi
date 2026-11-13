// The macOS side of `vn build`: a copy of the host becomes one app's `.app`.
//
//   <exeName>.app/Contents/{Info.plist, PkgInfo, MacOS/<exeName> (the bare host, verbatim),
//                           Resources/app.vnpk, Resources/AppIcon.icns}
//
// Nothing is written into the Mach-O (it would break the host's signature); the payload is a file in
// the bundle, covered by its signature. The bundle is assembled as `<out>.partial` and renamed.
// `codesign` runs only on macOS: ad hoc without an identity (Apple Silicon needs it), otherwise
// hardened runtime, a secure timestamp and the entitlements JSC's JIT and the microphone and
// camera need. Notarisation is the author's step. The rest is platform-free, so it is tested on Windows.

import { spawnSync } from "node:child_process";
import { chmodSync, closeSync, copyFileSync, existsSync, mkdirSync, openSync, readFileSync, readSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, extname, join } from "node:path";
import { decodePng, encodePng, isPng, parseIco, resizeRgba } from "./icon";

export type PlistValue = string | number | boolean | PlistValue[] | { [key: string]: PlistValue };

const XML_INVALID = /[\u0000-\u0008\u000B\u000C\u000E-\u001F￾￿]|[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

export function xmlText(s: string, where = "plist"): string {
  const bad = XML_INVALID.exec(s);
  if (bad) {
    const code = bad[0].charCodeAt(0).toString(16).padStart(4, "0");
    throw new Error(`${where}: U+${code} cannot appear in a property list (XML 1.0)`);
  }
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function plistLines(value: PlistValue, depth: number, where: string, out: string[]) {
  const pad = "\t".repeat(depth);
  if (typeof value === "string") out.push(`${pad}<string>${xmlText(value, where)}</string>`);
  else if (typeof value === "boolean") out.push(`${pad}<${value}/>`);
  else if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error(`${where}: ${value} is not a plist number`);
    out.push(Number.isInteger(value) ? `${pad}<integer>${value}</integer>` : `${pad}<real>${value}</real>`);
  } else if (Array.isArray(value)) {
    if (!value.length) out.push(`${pad}<array/>`);
    else {
      out.push(`${pad}<array>`);
      value.forEach((v, i) => plistLines(v, depth + 1, `${where}[${i}]`, out));
      out.push(`${pad}</array>`);
    }
  } else {
    const keys = Object.keys(value);
    if (!keys.length) out.push(`${pad}<dict/>`);
    else {
      out.push(`${pad}<dict>`);
      for (const k of keys) {
        out.push(`${pad}\t<key>${xmlText(k, where)}</key>`);
        plistLines(value[k]!, depth + 1, `${where}.${k}`, out);
      }
      out.push(`${pad}</dict>`);
    }
  }
}

export function plistXml(root: { [key: string]: PlistValue }): string {
  const out = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">',
  ];
  plistLines(root, 0, "plist", out);
  out.push("</plist>", "");
  return out.join("\n");
}

export interface MacInfo {
  bundleId: string;
  name: string;
  executable: string;
  version: string;
  build?: string;
  iconFile: string | null;
  minimumSystemVersion: string;
  category?: string | null;
  copyright?: string | null;
  microphoneUsageDescription?: string | null;
  cameraUsageDescription?: string | null;
  urlSchemes?: string[];
}

export function infoPlist(m: MacInfo): { [key: string]: PlistValue } {
  const d: { [key: string]: PlistValue } = {
    CFBundleDevelopmentRegion: "en",
    CFBundleDisplayName: m.name,
    CFBundleExecutable: m.executable,
  };
  if (m.iconFile) d.CFBundleIconFile = m.iconFile;
  d.CFBundleIdentifier = m.bundleId;
  d.CFBundleInfoDictionaryVersion = "6.0";
  d.CFBundleName = m.name;
  d.CFBundlePackageType = "APPL";
  d.CFBundleShortVersionString = m.version;
  d.CFBundleSignature = "????";
  if (m.urlSchemes?.length) {
    d.CFBundleURLTypes = [{ CFBundleURLName: m.bundleId, CFBundleURLSchemes: [...m.urlSchemes] }];
  }
  d.CFBundleVersion = m.build ?? m.version;
  if (m.category) d.LSApplicationCategoryType = m.category;
  d.LSMinimumSystemVersion = m.minimumSystemVersion;
  if (m.cameraUsageDescription) {
    d.NSCameraUsageDescription = m.cameraUsageDescription;
    // macOS 14+ lists an iPhone as a camera only for an app that opts in.
    d.NSCameraUseContinuityCameraDeviceType = true;
  }
  d.NSHighResolutionCapable = true;
  if (m.copyright) d.NSHumanReadableCopyright = m.copyright;
  if (m.microphoneUsageDescription) d.NSMicrophoneUsageDescription = m.microphoneUsageDescription;
  return d;
}

export function entitlements(m: Pick<MacInfo, "microphoneUsageDescription" | "cameraUsageDescription">): { [key: string]: PlistValue } {
  const d: { [key: string]: PlistValue } = { "com.apple.security.cs.allow-jit": true };
  if (m.microphoneUsageDescription) d["com.apple.security.device.audio-input"] = true;
  if (m.cameraUsageDescription) d["com.apple.security.device.camera"] = true;
  return d;
}

// "icns" u32be total length, then entries: OSType(4) u32be length (incl. these 8 bytes) data.
// Since 10.7 every entry may hold a PNG, so an icns is a list of PNGs labelled by size.

/** The PNG-bearing types, smallest first: [OSType, pixels]. `@2x` types share a size with a 1x one. */
export const ICNS_TYPES: readonly (readonly [string, number])[] = [
  ["icp4", 16],
  ["ic11", 32], // 16@2x
  ["icp5", 32],
  ["ic12", 64], // 32@2x
  ["ic07", 128],
  ["ic13", 256], // 128@2x
  ["ic08", 256],
  ["ic14", 512], // 256@2x
  ["ic09", 512],
  ["ic10", 1024], // 512@2x
];

export interface IcnsEntry {
  type: string;
  data: Uint8Array;
}

export function encodeIcns(entries: IcnsEntry[]): Uint8Array {
  const total = 8 + entries.reduce((n, e) => n + 8 + e.data.length, 0);
  const out = new Uint8Array(total);
  const dv = new DataView(out.buffer);
  const tag = (o: number, s: string) => {
    if (!/^[\x20-\x7e]{4}$/.test(s)) throw new Error(`icns: ${JSON.stringify(s)} is not an OSType`);
    for (let i = 0; i < 4; i++) out[o + i] = s.charCodeAt(i);
  };
  tag(0, "icns");
  dv.setUint32(4, total);
  let o = 8;
  for (const e of entries) {
    tag(o, e.type);
    dv.setUint32(o + 4, 8 + e.data.length);
    out.set(e.data, o + 8);
    o += 8 + e.data.length;
  }
  return out;
}

export function parseIcns(bytes: Uint8Array, label = "icns"): IcnsEntry[] {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (o: number) => String.fromCharCode(...bytes.subarray(o, o + 4));
  if (bytes.length < 8 || tag(0) !== "icns") throw new Error(`${label}: not an .icns (no "icns" magic)`);
  if (dv.getUint32(4) !== bytes.length) {
    throw new Error(`${label}: the header says ${dv.getUint32(4)} bytes, the file is ${bytes.length}`);
  }
  const entries: IcnsEntry[] = [];
  for (let o = 8; o < bytes.length; ) {
    if (o + 8 > bytes.length) throw new Error(`${label}: truncated entry header at ${o}`);
    const len = dv.getUint32(o + 4);
    if (len < 8 || o + len > bytes.length) throw new Error(`${label}: entry ${tag(o)} at ${o} claims ${len} bytes`);
    entries.push({ type: tag(o), data: bytes.subarray(o + 8, o + len) });
    o += len;
  }
  return entries;
}

export interface Icns {
  bytes: Uint8Array;
  sizes: number[];
  verbatim: boolean;
}

function sourcePng(path: string, bytes: Uint8Array): Uint8Array {
  if (extname(path).toLowerCase() === ".ico") {
    const pngs = parseIco(bytes, path).filter((img) => isPng(img.data));
    if (!pngs.length) {
      throw new Error(`${path}: the .ico holds only BMP images — give macos.icon a square .png (1024 px is best) or an .icns`);
    }
    return pngs.reduce((a, b) => (b.width > a.width ? b : a)).data;
  }
  if (!isPng(bytes)) throw new Error(`${path}: a macOS icon must be an .icns, a square .png, or an .ico holding PNG images`);
  return bytes;
}

export function icnsFromPng(png: Uint8Array, label = "png"): Icns {
  const img = decodePng(png, label);
  if (img.width !== img.height) {
    throw new Error(`${label}: an app icon must be square (this one is ${img.width}×${img.height})`);
  }
  if (img.width < 16) throw new Error(`${label}: ${img.width}px is smaller than the smallest icon size (16)`);
  const bySize = new Map<number, Uint8Array>();
  const entries: IcnsEntry[] = [];
  for (const [type, size] of ICNS_TYPES) {
    if (size > img.width) continue;
    let data = bySize.get(size);
    if (!data) {
      data = size === img.width ? png : encodePng(resizeRgba(img, size, size));
      bySize.set(size, data);
    }
    entries.push({ type, data });
  }
  return { bytes: encodeIcns(entries), sizes: [...bySize.keys()], verbatim: false };
}

export function icnsFrom(path: string): Icns {
  const bytes = new Uint8Array(readFileSync(path));
  if (extname(path).toLowerCase() === ".icns") {
    const entries = parseIcns(bytes, path);
    const sizes = new Set<number>();
    for (const e of entries) {
      const known = ICNS_TYPES.find(([t]) => t === e.type);
      if (known) sizes.add(known[1]);
    }
    return { bytes, sizes: [...sizes].sort((a, b) => a - b), verbatim: true };
  }
  return icnsFromPng(sourcePng(path, bytes), path);
}

const CPU: Record<number, string> = { 0x0100000c: "arm64", 0x01000007: "x86_64", 0x0000000c: "arm", 0x00000007: "i386" };

export function machOArchs(path: string): string[] | null {
  let head: Uint8Array;
  try {
    const fd = openSync(path, "r");
    try {
      head = new Uint8Array(4096);
      head = head.subarray(0, readSync(fd, head, 0, head.length, 0));
    } finally {
      closeSync(fd);
    }
  } catch {
    return null;
  }
  if (head.length < 8) return null;
  const dv = new DataView(head.buffer, head.byteOffset, head.byteLength);
  const le = dv.getUint32(0, true);
  if (le === 0xfeedfacf || le === 0xfeedface) return [CPU[dv.getUint32(4, true)] ?? `cpu 0x${dv.getUint32(4, true).toString(16)}`];
  const be = dv.getUint32(0);
  if (be === 0xcafebabe || be === 0xcafebabf) {
    const n = dv.getUint32(4);
    const stride = be === 0xcafebabf ? 32 : 20;
    // Java class files share 0xcafebabe; their "count" is a version number in the 40s+.
    if (n === 0 || n > 16 || 8 + n * stride > head.length) return null;
    const archs: string[] = [];
    for (let i = 0; i < n; i++) {
      const cpu = dv.getUint32(8 + i * stride);
      archs.push(CPU[cpu] ?? `cpu 0x${cpu.toString(16)}`);
    }
    return archs;
  }
  return null;
}

export function bundlePayloadPath(exe: string): string | null {
  const macos = dirname(exe);
  const contents = dirname(macos);
  const bundle = dirname(contents);
  if (basename(macos) !== "MacOS" || basename(contents) !== "Contents") return null;
  if (extname(bundle).toLowerCase() !== ".app") return null;
  return join(contents, "Resources", "app.vnpk");
}

export const PAYLOAD_FILE = "app.vnpk";
export const ICON_NAME = "AppIcon";

export interface AppBundleOptions {
  host: string;
  out: string;
  executable: string;
  payload: Uint8Array;
  icns?: Uint8Array | null;
  infoPlist: string;
  log?: (line: string) => void;
}

export interface AppBundleResult {
  app: string;
  executable: string;
  bytes: number;
  payloadBytes: number;
  iconBytes: number;
  ms: { copy: number; total: number };
}

export function writeAppBundle(options: AppBundleOptions): AppBundleResult {
  const t0 = performance.now();
  if (!options.executable || /[/\\:]/.test(options.executable)) {
    throw new Error(`${JSON.stringify(options.executable)} is not a file name for Contents/MacOS`);
  }
  const partial = `${options.out}.partial`;
  rmSync(partial, { recursive: true, force: true });
  const contents = join(partial, "Contents");
  const macosDir = join(contents, "MacOS");
  const resources = join(contents, "Resources");
  let done = false;
  try {
    mkdirSync(macosDir, { recursive: true });
    mkdirSync(resources, { recursive: true });
    const exe = join(macosDir, options.executable);
    copyFileSync(options.host, exe);
    chmodSync(exe, 0o755);
    const tCopy = performance.now();
    writeFileSync(join(resources, PAYLOAD_FILE), options.payload);
    if (options.icns) writeFileSync(join(resources, `${ICON_NAME}.icns`), options.icns);
    writeFileSync(join(contents, "Info.plist"), options.infoPlist);
    writeFileSync(join(contents, "PkgInfo"), "APPL????");

    if (existsSync(options.out)) {
      try {
        rmSync(options.out, { recursive: true, force: true });
      } catch (e) {
        throw new Error(`${options.out} cannot be replaced — is it running? Quit it and build again. (${(e as Error).message})`);
      }
    }
    renameSync(partial, options.out);
    done = true;
    const app = options.out;
    const executable = join(app, "Contents", "MacOS", options.executable);
    const bytes =
      statSync(executable).size + options.payload.length + (options.icns?.length ?? 0) + Buffer.byteLength(options.infoPlist) + 8;
    return {
      app,
      executable,
      bytes,
      payloadBytes: options.payload.length,
      iconBytes: options.icns?.length ?? 0,
      ms: { copy: Math.round(tCopy - t0), total: Math.round(performance.now() - t0) },
    };
  } finally {
    if (!done) rmSync(partial, { recursive: true, force: true });
  }
}

/** `-` is ad hoc: no certificate, no hardened runtime, nothing to notarise. */
export const AD_HOC = "-";

export function codesignArgs(app: string, identity: string, entitlementsPath: string | null): string[] {
  if (identity === AD_HOC) return ["--force", "--sign", AD_HOC, app];
  return [
    "--force",
    "--options",
    "runtime",
    "--timestamp",
    ...(entitlementsPath ? ["--entitlements", entitlementsPath] : []),
    "--sign",
    identity,
    app,
  ];
}

export interface SignPlan {
  identity: string | null;
  note: string | null;
}

const shellWord = (s: string) => (/^[\w@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);

export function signPlan(input: {
  want: string | false | undefined;
  app: string;
  entitlements: string;
  codesign: string | null;
  onMac: boolean;
}): SignPlan {
  const { want, app, codesign } = input;
  if (want === false) return { identity: null, note: "not signed (signing is off)" };
  if (!input.onMac) {
    const args = codesignArgs(app, want ?? AD_HOC, input.entitlements);
    return {
      identity: null,
      note:
        `not signed: codesign runs on macOS only. On a Mac: codesign ${args.map(shellWord).join(" ")}` +
        (want ? "" : " (Apple Silicon wants at least this ad-hoc signature on the bundle)"),
    };
  }
  if (!codesign) {
    if (want) throw new Error(`codesign is not on PATH, so ${app} cannot be signed as ${JSON.stringify(want)} — xcode-select --install`);
    return { identity: null, note: "not signed: codesign is not on PATH (xcode-select --install) — the executable keeps only its linker signature" };
  }
  return { identity: want ?? AD_HOC, note: null };
}

export function signBundle(
  codesign: string,
  app: string,
  identity: string,
  entitlementsPath: string | null,
  log: (line: string) => void = () => {},
): { ms: number } {
  const t0 = performance.now();
  const args = codesignArgs(app, identity, entitlementsPath);
  log(`[vn] codesign ${args.join(" ")}`);
  const r = spawnSync(codesign, args, { encoding: "utf8" });
  if (r.status !== 0) {
    throw new Error(`codesign failed (exit ${r.status ?? r.error?.message}):\n${(r.stderr ?? "").trim()}`);
  }
  const v = spawnSync(codesign, ["--verify", "--strict", "--verbose=2", app], { encoding: "utf8" });
  if (v.status !== 0) throw new Error(`codesign --verify rejected ${app}:\n${(v.stderr ?? "").trim()}`);
  return { ms: Math.round(performance.now() - t0) };
}
