// What goes into the package: `vn.config.json` `"assets"`, resolved to files. The KEY of each file
// is its path relative to the app root, `/`-separated, case-preserved: what dev resolves a
// relative URL to. Vite's `publicDir` is the one remapping (`public/favicon.ico` -> `favicon.ico`),
// only when declared; the dev host does not remap it yet, a known dev/exe difference.
// Never packaged: `src/`, `node_modules/`, `dist/`, dotfiles and dot-directories, `vn.config.json`.
// Files an import pulled in (`?url`) are listed by the builder in assets.json and packed by build-app.ts.

import { existsSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

export interface AssetFile {
  key: string;
  path: string;
  size: number;
}

export interface Collected {
  files: AssetFile[];
  totalBytes: number;
  warnings: string[];
}

const GLOB = /[*?[\]{}]/;
const EXCLUDED_ROOTS = new Set(["src", "node_modules", "dist"]);

export function excluded(key: string): boolean {
  const segs = key.split("/");
  if (EXCLUDED_ROOTS.has(segs[0]!)) return true;
  if (segs.some((s) => s.startsWith("."))) return true;
  return key === "vn.config.json";
}

const toKey = (appRoot: string, abs: string) => relative(appRoot, abs).split(sep).join("/");

function walk(dir: string, out: string[]) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const abs = join(dir, entry.name);
    if (entry.isDirectory()) walk(abs, out);
    else if (entry.isFile()) out.push(abs);
  }
}

export function collectAssets(
  appRoot: string,
  patterns: string[],
  declared: boolean,
  publicDir: string | null = null,
): Collected {
  const warnings: string[] = [];
  const byKey = new Map<string, AssetFile>();
  if (publicDir) {
    const root = join(appRoot, publicDir);
    const found: string[] = [];
    if (existsSync(root) && statSync(root).isDirectory()) walk(root, found);
    for (const abs of found) {
      const key = toKey(root, abs);
      byKey.set(key, { key, path: abs, size: statSync(abs).size });
    }
  } else if (existsSync(join(appRoot, "public")) && statSync(join(appRoot, "public")).isDirectory()) {
    warnings.push(
      'the app has a public/ directory but vn.config.json declares no "publicDir": Vite serves public/x at /x, ' +
        'a package does not unless you set "publicDir": "public"',
    );
  }
  const fromPublic = new Set(byKey.keys());
  for (const pattern of patterns) {
    const clean = pattern.replace(/\\/g, "/").replace(/^\.\//, "").replace(/\/+$/, "");
    if (!clean || clean.startsWith("/") || /^[a-zA-Z]:/.test(clean) || clean.split("/").includes("..")) {
      throw new Error(`vn.config.json "assets": ${JSON.stringify(pattern)} must be relative to the app root and stay inside it`);
    }
    const matched: string[] = [];
    if (GLOB.test(clean)) {
      for (const rel of new Bun.Glob(clean).scanSync({ cwd: appRoot, onlyFiles: true, dot: false })) {
        matched.push(join(appRoot, rel));
      }
    } else {
      const abs = join(appRoot, clean);
      if (existsSync(abs)) {
        if (statSync(abs).isDirectory()) walk(abs, matched);
        else matched.push(abs);
      }
    }
    let kept = 0;
    for (const abs of matched) {
      const key = toKey(appRoot, abs);
      if (key.startsWith("..") || excluded(key)) continue;
      kept++;
      if (fromPublic.has(key)) {
        warnings.push(`${JSON.stringify(key)} is both in "${publicDir}/" and in "assets" — the ${publicDir}/ file is packaged`);
        fromPublic.delete(key);
        continue;
      }
      if (!byKey.has(key)) byKey.set(key, { key, path: abs, size: statSync(abs).size });
    }
    if (!kept && (declared || clean !== "assets")) {
      warnings.push(`vn.config.json "assets": ${JSON.stringify(pattern)} matched no files`);
    }
  }
  const files = [...byKey.values()].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  // Two keys that differ only in case would be one file on NTFS; the host's case-insensitive fallback could pick either.
  const lower = new Map<string, string>();
  for (const f of files) {
    const k = f.key.toLowerCase();
    const prev = lower.get(k);
    if (prev) warnings.push(`assets ${JSON.stringify(prev)} and ${JSON.stringify(f.key)} differ only in case`);
    else lower.set(k, f.key);
  }
  return { files, totalBytes: files.reduce((n, f) => n + f.size, 0), warnings };
}
