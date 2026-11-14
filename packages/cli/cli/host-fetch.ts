// Prebuilt bare hosts from the Skathi GitHub release of the CLI's own version, so `sk build`
// needs neither a checkout nor a Rust toolchain. The release (.github/workflows/release.yml,
// scripts/release-hosts.ts) carries one gzipped host per platform, arch and variant, plus
// `hosts.json`, which names each with its sha256 and its `--sk-host-info`.
//
// Assets are read through the GitHub API, so a private repository works with GH_TOKEN or
// GITHUB_TOKEN set. SK_HOST_REPO (owner/name) and SK_HOST_API point elsewhere, a fork or a mirror.

import { chmodSync, mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { partsLeftOut, VARIANTS, type Variant } from "./config";
import {
  CLI_VERSION,
  cachedHost,
  HostError,
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
}

export interface HostManifest {
  skathi: string;
  hosts: HostManifestEntry[];
}

export const hostAssetName = (platform: Platform, arch: string, variant: Variant): string =>
  `sk-host-${platform}-${arch}-${variant}${platform === "windows" ? ".exe" : ""}.gz`;

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
}

function source() {
  const repo = process.env.SK_HOST_REPO || DEFAULT_REPO;
  const api = (process.env.SK_HOST_API || "https://api.github.com").replace(/\/+$/, "");
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  const headers: Record<string, string> = { "User-Agent": "skathi-cli", "X-GitHub-Api-Version": "2022-11-28" };
  if (token) headers.Authorization = `Bearer ${token}`;
  return { repo, api, headers };
}

async function download(asset: ReleaseAsset, headers: Record<string, string>): Promise<Uint8Array<ArrayBuffer>> {
  const res = await fetch(asset.url, { headers: { ...headers, Accept: "application/octet-stream" } });
  if (!res.ok) throw new HostError(`downloading ${asset.name}: HTTP ${res.status}`);
  return new Uint8Array(await res.arrayBuffer());
}

/** Downloads the host for `variant` from the release of `version` into the local cache, checked against `hosts.json`. */
export async function fetchHost(options: FetchOptions): Promise<{ path: string; info: HostInfo }> {
  const version = options.version ?? CLI_VERSION;
  const platform = options.platform ?? currentPlatform();
  const arch = options.arch ?? process.arch;
  const log = options.log ?? (() => {});
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
  const entry = manifest.hosts.find((h) => h.platform === platform && h.arch === arch && h.variant === options.variant);
  if (!entry) {
    const offered = manifest.hosts
      .filter((h) => h.platform === platform && h.arch === arch)
      .map((h) => h.variant)
      .join(", ");
    throw new HostError(
      `release ${tag} has no ${options.variant} host for ${platform}-${arch}` +
        (offered ? ` (it has: ${offered}); other variants are built from a checkout with sk host build` : ""),
    );
  }
  const hostAsset = asset(entry.file);
  if (!hostAsset) throw new HostError(`release ${tag} lists ${entry.file} in ${MANIFEST_NAME} but does not carry it`);

  log(`downloading ${entry.file} (${(hostAsset.size / 1048576).toFixed(1)} MB) from github.com/${repo} ${tag}`);
  const bytes = Bun.gunzipSync(await download(hostAsset, headers));
  if (bytes.length !== entry.size || sha256(bytes) !== entry.sha256) {
    throw new HostError(`${entry.file} does not match the sha256 in ${MANIFEST_NAME}; nothing was cached`);
  }

  const dest = cachedHost(version, options.variant, platform);
  mkdirSync(dirname(dest), { recursive: true });
  const tmp = `${dest}.partial`;
  writeFileSync(tmp, bytes);
  if (process.platform !== "win32") chmodSync(tmp, 0o755);
  rmSync(dest, { force: true });
  renameSync(tmp, dest);
  writeFileSync(join(dirname(dest), "host.json"), JSON.stringify(entry.info, null, 2) + "\n");
  log(`cached ${options.variant} host ${version} → ${dest}`);
  return { path: dest, info: entry.info };
}

/**
 * The published variant that can stand in for `variant`: devtools exactly as asked, and every other
 * part the app may use, with the fewest extra. A host with a part the app does not use only costs size.
 */
export function standIn(variant: Variant, published: readonly Variant[] = VARIANTS): Variant | null {
  const off = new Set(partsLeftOut(variant));
  const fits = published.filter((v) => {
    const o = partsLeftOut(v);
    return o.includes("devtools") === off.has("devtools") && o.every((p) => off.has(p));
  });
  return fits.sort((a, b) => partsLeftOut(b).length - partsLeftOut(a).length)[0] ?? null;
}

/**
 * {@link resolveHost}, and when it finds nothing for this OS and no host was named, a download into the
 * cache first. A variant no release carries falls back to its {@link standIn}, with `substitute` saying
 * so. SK_HOST_OFFLINE=1 turns the download off.
 */
export async function resolveOrFetchHost(options: ResolveOptions): Promise<ResolvedHost & { substitute?: string }> {
  try {
    return resolveHost(options);
  } catch (e) {
    const named = options.host ?? process.env.SK_HOST;
    const cross = (options.platform ?? currentPlatform()) !== currentPlatform();
    if (!(e instanceof HostError) || named || cross || process.env.SK_HOST_OFFLINE === "1") throw e;
    const instead = VARIANTS.includes(options.variant) ? null : standIn(options.variant);
    if (instead) {
      const host = await resolveOrFetchHost({ ...options, variant: instead });
      const extra = partsLeftOut(instead).length < partsLeftOut(options.variant).length;
      return {
        ...host,
        substitute:
          `no prebuilt ${options.variant} host: using ${instead}` +
          (extra ? ", which also carries parts this app leaves out (sk host build makes an exact one)" : ""),
      };
    }
    try {
      await fetchHost({ variant: options.variant, ...(options.log && { log: options.log }) });
    } catch (f) {
      throw new HostError(`${e.message}\n(downloading it failed too: ${(f as Error).message})`);
    }
    return resolveHost(options);
  }
}
