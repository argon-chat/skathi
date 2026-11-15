// redslangc, the Slang compiler the effect toolchain runs: pinned by version and sha256 per platform
// in redslangc.json, downloaded from its GitHub release into <cache>/skathi/redslangc/<version>/.
// `--compiler <exe>` or SK_REDSLANGC names another build (a checkout's `cargo build` of
// vendor/redslangc, or `cargo install redslangc`).

import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { inflateRawSync } from "node:zlib";
import { cacheRoot } from "../cli/host";
import type { OnBytes } from "../cli/host-fetch";
import { currentPlatform, type Platform } from "../cli/platform";

export class ShaderCompilerError extends Error {}

interface PinnedAsset {
  file: string;
  sha256: string;
}

export interface Pin {
  version: string;
  slang: string;
  release: string;
  assets: Record<string, PinnedAsset>;
}

export const PIN: Pin = JSON.parse(readFileSync(join(import.meta.dir, "redslangc.json"), "utf8"));

export interface ShaderCompiler {
  path: string;
  version: string;
  source: "--compiler" | "SK_REDSLANGC" | "cache" | "release";
}

const exeName = (platform: Platform) => (platform === "windows" ? "redslangc.exe" : "redslangc");

/** `<cache>/skathi/redslangc`, beside the workloads. */
export const compilerCacheRoot = (platform: Platform = currentPlatform()) =>
  join(dirname(cacheRoot(platform)), "redslangc");

export const cachedCompiler = (version: string = PIN.version, platform: Platform = currentPlatform()) =>
  join(compilerCacheRoot(platform), version, exeName(platform));

export const compilerExe = exeName;

function assetKey(platform: Platform = currentPlatform(), arch: string = process.arch): string {
  return `${platform}-${arch}`;
}

/** What `redslangc --version` says: `redslangc 0.1.0 (Slang 2026.12)`. */
export function compilerVersion(path: string): { version: string; slang: string } {
  const r = spawnSync(path, ["--version"], { encoding: "utf8", timeout: 30_000, windowsHide: true });
  if (r.error) throw new ShaderCompilerError(`${path} could not be run: ${r.error.message}`);
  const m = /redslangc\s+(\S+)\s+\(Slang\s+([^)]+)\)/.exec(r.stdout ?? "");
  if (r.status !== 0 || !m) throw new ShaderCompilerError(`${path} is not redslangc (--version: ${r.stdout.trim()})`);
  return { version: m[1]!, slang: m[2]!.trim() };
}

const sha256 = (bytes: Uint8Array): string =>
  new Bun.CryptoHasher("sha256").update(new Uint8Array(bytes)).digest("hex");

async function download(url: string, onBytes?: OnBytes): Promise<Uint8Array<ArrayBuffer>> {
  const res = await fetch(url, { headers: { "User-Agent": "skathi-cli" } });
  if (!res.ok) throw new ShaderCompilerError(`downloading ${url}: HTTP ${res.status}`);
  if (!onBytes || !res.body) return new Uint8Array(await res.arrayBuffer());
  const total = Number(res.headers.get("content-length")) || 0;
  const chunks: Uint8Array[] = [];
  let done = 0;
  for await (const chunk of res.body) {
    chunks.push(chunk);
    done += chunk.length;
    onBytes(done, total || done);
  }
  const bytes = new Uint8Array(done);
  let at = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, at);
    at += chunk.length;
  }
  return bytes;
}

export interface ArchiveFile {
  /** Relative to the archive's top directory. */
  name: string;
  data: Uint8Array;
}

/** The regular files of a release archive, its top-level directory stripped. */
export function unpack(archive: Uint8Array<ArrayBuffer>, file: string): ArchiveFile[] {
  const raw = file.endsWith(".zip") ? unzip(archive) : untar(Bun.gunzipSync(archive));
  const out: ArchiveFile[] = [];
  for (const { name, data } of raw) {
    const parts = name.replace(/\\/g, "/").split("/").filter(Boolean);
    if (parts.length < 2 || parts.some((p) => p === ".." || p === ".") || parts.at(-1)!.startsWith("._")) continue;
    out.push({ name: parts.slice(1).join("/"), data });
  }
  return out;
}

function unzip(zip: Uint8Array): ArchiveFile[] {
  const dv = new DataView(zip.buffer, zip.byteOffset, zip.byteLength);
  let eocd = -1;
  for (let i = zip.length - 22; i >= Math.max(0, zip.length - 22 - 0xffff); i--) {
    if (dv.getUint32(i, true) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new ShaderCompilerError("the redslangc archive is not a zip file");
  const count = dv.getUint16(eocd + 10, true);
  let at = dv.getUint32(eocd + 16, true);
  const decoder = new TextDecoder();
  const out: ArchiveFile[] = [];
  for (let i = 0; i < count; i++) {
    if (dv.getUint32(at, true) !== 0x02014b50) throw new ShaderCompilerError("the redslangc zip is damaged");
    const method = dv.getUint16(at + 10, true);
    const compressed = dv.getUint32(at + 20, true);
    const nameLen = dv.getUint16(at + 28, true);
    const extraLen = dv.getUint16(at + 30, true);
    const commentLen = dv.getUint16(at + 32, true);
    const local = dv.getUint32(at + 42, true);
    const name = decoder.decode(zip.subarray(at + 46, at + 46 + nameLen));
    at += 46 + nameLen + extraLen + commentLen;
    if (name.endsWith("/")) continue;
    const start = local + 30 + dv.getUint16(local + 26, true) + dv.getUint16(local + 28, true);
    const body = zip.subarray(start, start + compressed);
    if (method === 0) out.push({ name, data: body });
    else if (method === 8) out.push({ name, data: new Uint8Array(inflateRawSync(body)) });
    else throw new ShaderCompilerError(`the redslangc zip stores ${name} with method ${method}`);
  }
  return out;
}

function untar(tar: Uint8Array): ArchiveFile[] {
  const decoder = new TextDecoder();
  const field = (at: number, len: number) => decoder.decode(tar.subarray(at, at + len)).replace(/\0.*$/s, "");
  const out: ArchiveFile[] = [];
  let at = 0;
  while (at + 512 <= tar.length) {
    const name = field(at, 100);
    if (!name) break;
    const size = parseInt(field(at + 124, 12).trim() || "0", 8);
    const type = String.fromCharCode(tar[at + 156]!);
    const prefix = field(at + 257, 6).startsWith("ustar") ? field(at + 345, 155) : "";
    if (type === "0" || type === "\0") {
      out.push({ name: prefix ? `${prefix}/${name}` : name, data: tar.slice(at + 512, at + 512 + size) });
    }
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return out;
}

/** Downloads the pinned redslangc for this machine into the cache, checked against redslangc.json. */
export async function fetchCompiler(
  options: { log?: (line: string) => void; onBytes?: OnBytes; platform?: Platform; arch?: string } = {},
): Promise<string> {
  const platform = options.platform ?? currentPlatform();
  const key = assetKey(platform, options.arch);
  const asset = PIN.assets[key];
  if (!asset) {
    throw new ShaderCompilerError(
      `redslangc ${PIN.version} has no prebuilt binary for ${key} (there are: ${Object.keys(PIN.assets).join(", ")}) — ` +
        `build it with \`cargo install --locked redslangc@${PIN.version}\` and set SK_REDSLANGC to its path`,
    );
  }
  const url = `${PIN.release}/${asset.file}`;
  options.log?.(`downloading ${asset.file} from ${PIN.release}`);
  const archive = await download(url, options.onBytes);
  if (sha256(archive) !== asset.sha256) {
    throw new ShaderCompilerError(`${asset.file} does not match the sha256 sk pins for it; nothing was cached`);
  }
  const files = unpack(archive, asset.file);
  const exe = exeName(platform);
  if (!files.some((f) => f.name === exe)) throw new ShaderCompilerError(`${asset.file} has no ${exe}`);
  const dest = cachedCompiler(PIN.version, platform);
  const dir = dirname(dest);
  const partial = `${dir}.partial`;
  rmSync(partial, { recursive: true, force: true });
  for (const f of files) {
    const path = join(partial, f.name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, f.data);
  }
  if (platform !== "windows") chmodSync(join(partial, exe), 0o755);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dirname(dir), { recursive: true });
  renameSync(partial, dir);
  options.log?.(`cached redslangc ${PIN.version} → ${dest}`);
  return dest;
}

/** The redslangc to run, first hit wins: `named` or SK_REDSLANGC; the cache; a download. */
export async function resolveCompiler(
  options: { named?: string | undefined; log?: (line: string) => void; onBytes?: OnBytes } = {},
): Promise<ShaderCompiler> {
  const named = options.named ?? process.env.SK_REDSLANGC;
  if (named) {
    const explicit = resolve(named);
    if (!existsSync(explicit)) throw new ShaderCompilerError(`no redslangc at ${explicit}`);
    const { version, slang } = compilerVersion(explicit);
    if (slang !== PIN.slang) {
      options.log?.(
        `warning: ${explicit} is Slang ${slang}; the effects sk ships are compiled with Slang ${PIN.slang}`,
      );
    }
    return { path: explicit, version, source: options.named ? "--compiler" : "SK_REDSLANGC" };
  }
  const cached = cachedCompiler();
  if (existsSync(cached)) return { path: cached, version: PIN.version, source: "cache" };
  if (process.env.SK_HOST_OFFLINE === "1") {
    throw new ShaderCompilerError(
      `the app has effect shaders and redslangc ${PIN.version} is not cached (${cached}); SK_HOST_OFFLINE=1 ` +
        "forbids downloading it — set SK_REDSLANGC to a redslangc binary",
    );
  }
  const path = await fetchCompiler(options);
  return { path, version: PIN.version, source: "release" };
}
