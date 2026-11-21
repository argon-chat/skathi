// Prebuilt bare hosts from the Skathi GitHub release of the CLI's own version, so `sk build`
// needs neither a checkout nor a Rust toolchain. The release (.github/workflows/release.yml,
// scripts/release-hosts.ts) carries one gzipped host per platform, arch and variant, plus
// `hosts.json`, which names each with its sha256 and its `--sk-host-info`. Cached beside a host: the full ICU data
// when it links only the root locale, the empty native image container when it has the AOT loader, and its
// inspector's container when its inspector runs from one.
//
// Assets are read through the GitHub API, so a private repository works with GH_TOKEN or
// GITHUB_TOKEN set. SK_HOST_REPO (owner/name) and SK_HOST_API point elsewhere, a fork or a mirror.

import { chmodSync, mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { CompilerInfo } from "../aot/pin";
import type { Variant } from "./config";
import {
  CLI_VERSION,
  cachedHost,
  cacheRoot,
  HostError,
  icuDataPath,
  imageTemplatePath,
  inspectorContainerPath,
  resolveHost,
  type HostInfo,
  type ResolvedHost,
  type ResolveOptions,
} from "./host";
import { currentPlatform, type Platform } from "./platform";

export const MANIFEST_NAME = "hosts.json";
export const DEFAULT_REPO = "argon-chat/skathi";

export interface HostManifestEntry {
  platform: Platform;
  arch: string;
  variant: Variant;
  /** The release asset: the host, gzipped. */
  file: string;
  /** Of the host itself, not of the gzip. */
  sha256: string;
  size: number;
  info: HostInfo;
  /** A host that links only the root ICU locale: the full data, gzipped (`<package>.dat` once cached). */
  icu?: { file: string; sha256: string; size: number };
  /** A host with the native AOT loader: the empty container native images are written into, gzipped (`skimage.dll` once cached). */
  imageTemplate?: { file: string; sha256: string; size: number };
  /** A host whose inspector is a container of its own: that container, gzipped (named by `info.aot.inspectorContainer`). */
  inspector?: { file: string; sha256: string; size: number };
}

/** sk-aot (`sk build --aot`), matching the release's hosts. */
export interface AotCompilerEntry {
  platform: Platform;
  arch: string;
  /** The release asset: sk-aot, gzipped. */
  file: string;
  /** Of sk-aot itself, not of the gzip. */
  sha256: string;
  size: number;
  /** What `sk-aot info` prints: the frontend, engine and builtin table it compiles for. */
  info: CompilerInfo;
}

export interface HostManifest {
  skathi: string;
  hosts: HostManifestEntry[];
  aot?: AotCompilerEntry[];
}

export const hostAssetName = (platform: Platform, arch: string, variant: Variant): string =>
  `sk-host-${platform}-${arch}-${variant}${platform === "windows" ? ".exe" : ""}.gz`;

export const icuAssetName = (platform: Platform, arch: string): string => `sk-icu-${platform}-${arch}.dat.gz`;

export const imageTemplateAssetName = (platform: Platform, arch: string): string =>
  `sk-image-template-${platform}-${arch}.${platform === "macos" ? "dylib" : "dll"}.gz`;

export const inspectorAssetName = (platform: Platform, arch: string): string =>
  `sk-inspector-image-${platform}-${arch}.${platform === "macos" ? "dylib" : "dll"}.gz`;

export const aotCompilerAssetName = (platform: Platform, arch: string): string =>
  `sk-aot-${platform}-${arch}${platform === "windows" ? ".exe" : ""}.gz`;

export const sha256 = (bytes: Uint8Array): string =>
  new Bun.CryptoHasher("sha256").update(new Uint8Array(bytes)).digest("hex");

interface ReleaseAsset {
  name: string;
  url: string;
  size: number;
}

export interface FetchOptions {
  variant: Variant;
  version?: string;
  platform?: Platform;
  arch?: string;
  log?: (line: string) => void;
  onBytes?: OnBytes | undefined;
}

function source() {
  const repo = process.env.SK_HOST_REPO || DEFAULT_REPO;
  const api = (process.env.SK_HOST_API || "https://api.github.com").replace(/\/+$/, "");
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  const headers: Record<string, string> = { "User-Agent": "skathi-cli", "X-GitHub-Api-Version": "2022-11-28" };
  if (token) headers.Authorization = `Bearer ${token}`;
  return { repo, api, headers };
}

export type OnBytes = (done: number, total: number) => void;

async function download(
  asset: ReleaseAsset,
  headers: Record<string, string>,
  onBytes?: OnBytes,
): Promise<Uint8Array<ArrayBuffer>> {
  const res = await fetch(asset.url, { headers: { ...headers, Accept: "application/octet-stream" } });
  if (!res.ok) throw new HostError(`downloading ${asset.name}: HTTP ${res.status}`);
  if (!onBytes || !res.body) return new Uint8Array(await res.arrayBuffer());
  const total = Number(res.headers.get("content-length")) || asset.size;
  const chunks: Uint8Array[] = [];
  let done = 0;
  for await (const chunk of res.body) {
    chunks.push(chunk);
    done += chunk.length;
    onBytes(done, total);
  }
  const bytes = new Uint8Array(done);
  let at = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, at);
    at += chunk.length;
  }
  return bytes;
}

interface Release {
  tag: string;
  repo: string;
  headers: Record<string, string>;
  manifest: HostManifest;
  asset: (name: string) => ReleaseAsset | undefined;
}

async function readRelease(version: string): Promise<Release> {
  const { repo, api, headers } = source();
  const tag = `v${version}`;
  const res = await fetch(`${api}/repos/${repo}/releases/tags/${tag}`, {
    headers: { ...headers, Accept: "application/vnd.github+json" },
  });
  if (res.status === 404) {
    throw new HostError(
      `github.com/${repo} has no ${tag} release` +
        (headers.Authorization ? "" : " (a private repository needs GH_TOKEN or GITHUB_TOKEN)"),
    );
  }
  if (!res.ok) throw new HostError(`github.com/${repo} release ${tag}: HTTP ${res.status}`);
  const assets = ((await res.json()) as { assets?: ReleaseAsset[] }).assets ?? [];
  const asset = (name: string) => assets.find((a) => a.name === name);

  const manifestAsset = asset(MANIFEST_NAME);
  if (!manifestAsset) throw new HostError(`release ${tag} of github.com/${repo} has no ${MANIFEST_NAME}`);
  const manifest = JSON.parse(new TextDecoder().decode(await download(manifestAsset, headers))) as HostManifest;
  return { tag, repo, headers, manifest, asset };
}

/** Gunzips `name` and writes it to `dest` only when it matches `size` and `sha256`. */
async function cacheAsset(
  release: Release,
  name: string,
  size: number,
  digest: string,
  dest: string,
  log: (line: string) => void,
  onBytes?: OnBytes,
): Promise<void> {
  const asset = release.asset(name);
  if (!asset) throw new HostError(`release ${release.tag} lists ${name} in ${MANIFEST_NAME} but does not carry it`);
  log(`downloading ${name} (${(asset.size / 1048576).toFixed(1)} MB) from github.com/${release.repo} ${release.tag}`);
  const bytes = Bun.gunzipSync(await download(asset, release.headers, onBytes));
  if (bytes.length !== size || sha256(bytes) !== digest) {
    throw new HostError(`${name} does not match the sha256 in ${MANIFEST_NAME}; nothing was cached`);
  }
  mkdirSync(dirname(dest), { recursive: true });
  const tmp = `${dest}.partial`;
  writeFileSync(tmp, bytes);
  if (process.platform !== "win32") chmodSync(tmp, 0o755);
  rmSync(dest, { force: true });
  renameSync(tmp, dest);
}

/** Downloads the workload for `variant` from the release of `version` into the local cache, checked against `hosts.json`. */
export async function fetchHost(options: FetchOptions): Promise<{ path: string; info: HostInfo }> {
  const version = options.version ?? CLI_VERSION;
  const platform = options.platform ?? currentPlatform();
  const arch = options.arch ?? process.arch;
  const log = options.log ?? (() => {});
  const release = await readRelease(version);
  const { manifest, tag } = release;
  const entry = manifest.hosts.find((h) => h.platform === platform && h.arch === arch && h.variant === options.variant);
  if (!entry) {
    const offered = manifest.hosts
      .filter((h) => h.platform === platform && h.arch === arch)
      .map((h) => h.variant)
      .join(", ");
    throw new HostError(
      `release ${tag} has no ${options.variant} workload for ${platform}-${arch}` +
        (offered ? ` (it has: ${offered}); other variants are built from a checkout with sk workload build` : ""),
    );
  }
  const dest = cachedHost(version, options.variant, platform);
  await cacheAsset(release, entry.file, entry.size, entry.sha256, dest, log, options.onBytes);
  const icu = icuDataPath({ exe: dest, info: entry.info });
  if (icu && entry.icu) await cacheAsset(release, entry.icu.file, entry.icu.size, entry.icu.sha256, icu, log);
  if (entry.imageTemplate) {
    const t = entry.imageTemplate;
    await cacheAsset(release, t.file, t.size, t.sha256, imageTemplatePath({ exe: dest }, platform), log);
  }
  const inspector = inspectorContainerPath({ exe: dest, info: entry.info });
  if (inspector && entry.inspector) {
    const i = entry.inspector;
    await cacheAsset(release, i.file, i.size, i.sha256, inspector, log);
  }
  writeFileSync(join(dirname(dest), "host.json"), JSON.stringify(entry.info, null, 2) + "\n");
  log(`cached ${options.variant} workload ${version} → ${dest}`);
  return { path: dest, info: entry.info };
}

/** Where the sk-aot of Skathi `version` is cached. */
export const cachedAotCompiler = (version: string, platform: Platform = currentPlatform()) =>
  join(cacheRoot(platform), version, "aot", `sk-aot${platform === "windows" ? ".exe" : ""}`);

/** Downloads sk-aot from the release of `version` into the local cache, checked against `hosts.json`. */
export async function fetchAotCompiler(
  options: Omit<FetchOptions, "variant"> = {},
): Promise<{ path: string; info: CompilerInfo }> {
  const version = options.version ?? CLI_VERSION;
  const platform = options.platform ?? currentPlatform();
  const arch = options.arch ?? process.arch;
  const log = options.log ?? (() => {});
  const release = await readRelease(version);
  const entry = release.manifest.aot?.find((c) => c.platform === platform && c.arch === arch);
  if (!entry) throw new HostError(`release ${release.tag} has no sk-aot for ${platform}-${arch}`);
  const dest = cachedAotCompiler(version, platform);
  await cacheAsset(release, entry.file, entry.size, entry.sha256, dest, log, options.onBytes);
  writeFileSync(join(dirname(dest), "compiler.json"), JSON.stringify(entry.info, null, 2) + "\n");
  log(`cached sk-aot ${version} → ${dest}`);
  return { path: dest, info: entry.info };
}

/**
 * {@link resolveHost}, and when it finds nothing for this OS and no host was named, a download into the
 * cache first. SK_HOST_OFFLINE=1 turns the download off.
 */
export async function resolveOrFetchHost(options: ResolveOptions): Promise<ResolvedHost> {
  try {
    return resolveHost(options);
  } catch (e) {
    const named = options.host ?? process.env.SK_HOST;
    const cross = (options.platform ?? currentPlatform()) !== currentPlatform();
    if (!(e instanceof HostError) || named || cross || process.env.SK_HOST_OFFLINE === "1") throw e;
    try {
      await fetchHost({ variant: options.variant, onBytes: options.onBytes, ...(options.log && { log: options.log }) });
    } catch (f) {
      throw new HostError(`${e.message}\n(downloading it failed too: ${(f as Error).message})`);
    }
    return resolveHost(options);
  }
}
