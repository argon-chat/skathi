// The per-user caches `sk workload list` shows and `sk prune` empties: the workloads and sk-aot of each
// Skathi version (<cache>/skathi/hosts/<version>/<variant>|aot), the native images AOT apps expand
// at start (Windows), and caches older versions of sk left behind.

import { existsSync, lstatSync, readdirSync, rmdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { cacheRoot } from "./host";
import { currentPlatform, hostsFor, type Platform } from "./platform";
import { mib, paint } from "./term";

export type CacheKind = "workload" | "compiler" | "native-images" | "legacy" | "other";

export interface CacheEntry {
  kind: CacheKind;
  path: string;
  bytes: number;
  version?: string;
  variant?: string;
  /** False when the executable is missing, e.g. after an interrupted download. */
  complete?: boolean;
  items?: number;
}

export interface CacheRoots {
  workloads: string;
  nativeImages: string | null;
  legacy: string[];
}

export function cacheRoots(platform: Platform = currentPlatform()): CacheRoots {
  const workloads = cacheRoot(platform);
  const skathi = dirname(workloads);
  const native = process.env.SK_AOT_CACHE_DIR || (platform === "windows" ? join(skathi, "native-aot", "v1") : null);
  return { workloads, nativeImages: native, legacy: [join(skathi, "aot-toolchain")] };
}

const names = (dir: string): string[] => {
  try {
    return readdirSync(dir);
  } catch {
    return [];
  }
};

const isDir = (path: string) => {
  try {
    return lstatSync(path).isDirectory();
  } catch {
    return false;
  }
};

export function sizeOf(path: string): number {
  try {
    const st = lstatSync(path);
    if (!st.isDirectory()) return st.size;
  } catch {
    return 0;
  }
  return names(path).reduce((n, name) => n + sizeOf(join(path, name)), 0);
}

/** Newest first; a release before its prereleases. */
export function compareVersions(a: string, b: string): number {
  const split = (v: string) => {
    const at = v.indexOf("-");
    return at < 0 ? { main: v, pre: null } : { main: v.slice(0, at), pre: v.slice(at + 1) };
  };
  const x = split(a);
  const y = split(b);
  const numeric = { numeric: true } as const;
  if (x.main !== y.main) return y.main.localeCompare(x.main, "en", numeric);
  if (x.pre === null || y.pre === null) return (x.pre === null ? -1 : 0) - (y.pre === null ? -1 : 0);
  return y.pre.localeCompare(x.pre, "en", numeric);
}

export function inventory(roots: CacheRoots, platform: Platform = currentPlatform()): CacheEntry[] {
  const exe = hostsFor(platform).cachedFile;
  const compiler = `sk-aot${platform === "windows" ? ".exe" : ""}`;
  const entries: CacheEntry[] = [];
  const top = names(roots.workloads);
  for (const version of top.filter((n) => isDir(join(roots.workloads, n))).sort(compareVersions)) {
    const dir = join(roots.workloads, version);
    for (const child of names(dir).sort((a, b) => Number(a === "aot") - Number(b === "aot") || a.localeCompare(b))) {
      const path = join(dir, child);
      const bytes = sizeOf(path);
      if (!isDir(path)) entries.push({ kind: "other", path, bytes, version });
      else if (child === "aot")
        entries.push({ kind: "compiler", path, bytes, version, complete: existsSync(join(path, compiler)) });
      else
        entries.push({ kind: "workload", path, bytes, version, variant: child, complete: existsSync(join(path, exe)) });
    }
  }
  for (const name of top.filter((n) => !isDir(join(roots.workloads, n))).sort()) {
    const path = join(roots.workloads, name);
    entries.push({ kind: "other", path, bytes: sizeOf(path) });
  }
  const images = roots.nativeImages ? names(roots.nativeImages) : [];
  if (roots.nativeImages && images.length) {
    entries.push({
      kind: "native-images",
      path: roots.nativeImages,
      bytes: sizeOf(roots.nativeImages),
      items: images.length,
    });
  }
  for (const path of roots.legacy) if (existsSync(path)) entries.push({ kind: "legacy", path, bytes: sizeOf(path) });
  return entries;
}

export interface KeepOptions {
  /** The version this sk is. */
  current: string;
  keep?: string[];
  all?: boolean;
}

/** Whether `entry` stays: a complete workload or compiler of the current version or a kept one. */
export function needed(entry: CacheEntry, options: KeepOptions): boolean {
  if (options.all || entry.version === undefined || entry.complete === false) return false;
  if (entry.kind === "native-images" || entry.kind === "legacy") return false;
  return entry.version === options.current || (options.keep ?? []).includes(entry.version);
}

export interface PruneResult {
  removed: CacheEntry[];
  failed: { entry: CacheEntry; error: string }[];
  bytes: number;
}

/** Removes what is not {@link needed}; an image a running app holds open is skipped, not an error. */
export function prune(
  entries: CacheEntry[],
  options: KeepOptions & { dryRun?: boolean; roots?: CacheRoots },
): PruneResult {
  const result: PruneResult = { removed: [], failed: [], bytes: 0 };
  for (const entry of entries) {
    if (needed(entry, options)) continue;
    if (options.dryRun) {
      result.removed.push(entry);
      result.bytes += entry.bytes;
      continue;
    }
    const parts = entry.kind === "native-images" ? names(entry.path).map((n) => join(entry.path, n)) : [entry.path];
    let freed = 0;
    let error: string | null = null;
    for (const part of parts) {
      const bytes = sizeOf(part);
      try {
        rmSync(part, { recursive: true, force: true });
        freed += bytes;
      } catch (e) {
        error ??= e instanceof Error ? e.message : String(e);
      }
    }
    result.bytes += freed;
    if (error) result.failed.push({ entry, error });
    else result.removed.push(entry);
  }
  if (!options.dryRun && options.roots) {
    for (const version of names(options.roots.workloads)) {
      const dir = join(options.roots.workloads, version);
      if (isDir(dir) && !names(dir).length) rmdirSync(dir);
    }
  }
  return result;
}

export function entryName(e: CacheEntry): string {
  if (e.kind === "workload") return `${e.variant} workload`;
  if (e.kind === "compiler") return "sk-aot";
  if (e.kind === "native-images") return `native images (${e.items})`;
  if (e.kind === "legacy") return "old AOT toolchain";
  return "stray file";
}

type Line = readonly [version: string, name: string, size: string, note: string];

function lines(rows: Line[], p: ReturnType<typeof paint>): string[] {
  const width = (i: 0 | 1 | 2) => Math.max(...rows.map((r) => r[i].length));
  const [w0, w1, w2] = [width(0), width(1), width(2)];
  return rows.map(([v, name, size, note]) =>
    `  ${p.cyan(v.padEnd(w0))}  ${name.padEnd(w1)}  ${size.padStart(w2)}  ${note}`.trimEnd(),
  );
}

const where = (e: CacheEntry, p: ReturnType<typeof paint>) => (e.version === undefined ? p.dim(e.path) : "");

export function formatInventory(
  entries: CacheEntry[],
  roots: CacheRoots,
  options: KeepOptions,
  color: boolean,
): string {
  const p = paint(color);
  if (!entries.length) return `${p.dim("nothing cached in")} ${roots.workloads}`;
  const rows = entries.map((e): Line => {
    const state =
      e.complete === false
        ? p.yellow("incomplete")
        : !needed(e, options)
          ? ""
          : e.version === options.current
            ? p.green("this sk")
            : p.green("kept");
    return [e.version ?? "", entryName(e), mib(e.bytes), [state, where(e, p)].filter(Boolean).join(" ")];
  });
  const total = entries.reduce((n, e) => n + e.bytes, 0);
  const free = entries.filter((e) => !needed(e, options)).reduce((n, e) => n + e.bytes, 0);
  return [
    `${p.bold("Workloads")} ${p.dim(roots.workloads)}`,
    ...lines(rows, p),
    "",
    `${mib(total)} in all; sk prune frees ${free ? p.bold(mib(free)) : "nothing"}`,
  ].join("\n");
}

export function formatPrune(result: PruneResult, options: KeepOptions & { dryRun?: boolean }, color: boolean): string {
  const p = paint(color);
  const kept = [options.current, ...(options.keep ?? [])].join(", ");
  if (!result.removed.length && !result.failed.length) return `nothing to prune; the workloads of ${kept} stay`;
  const verb = options.dryRun ? "would remove" : "removed";
  const rows = [
    ...result.removed.map((e): Line => [e.version ?? "", entryName(e), mib(e.bytes), p.dim(verb)]),
    ...result.failed.map(({ entry: e, error }): Line => [e.version ?? "", entryName(e), mib(e.bytes), p.yellow(error)]),
  ];
  return [
    ...lines(rows, p),
    "",
    `${options.dryRun ? "would free" : "freed"} ${p.bold(mib(result.bytes))}` +
      (options.all ? "" : `; the workloads of ${kept} stay (--all removes them too)`),
    ...(result.failed.length ? [`${p.yellow("in use")}: close the apps that hold them and run sk prune again`] : []),
  ].join("\n");
}
