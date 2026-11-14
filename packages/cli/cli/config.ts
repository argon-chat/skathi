// `sk.config.json`, as `sk build` reads it. Keys beyond the dev builder's (name, window, devtools):
//
//   "name"                       identity: %LOCALAPPDATA%\<name>\storage, the devtools slot
//   "productName", "version"     display name (default window.title ?? name); default: package.json version
//   "httpCache": { "maxMB": 256 }  disk HTTP cache cap; 0 or false = none
//   "id", "singleInstance", "raiseOnSecondInstance", "protocols"  launch identity and URL schemes
//   "assets", "publicDir"        what is packaged (relative to the app root); Vite's public/, keyed relative to it
//   "packs": [{ "name": "fonts", "match": ["**/*.woff2"] }, { "name": "assets" }]
//                                assets in their own .skpk files, in order; no "match" takes the rest
//   "define", "aliases"          Vite's `define`; a bare module name -> the app's own stand-in file
//   "publishDevtools": true      sk publish keeps devtools (the standard host instead of no-devtools)
//   "aot": true                  sk build and sk publish compile the app to a native image (Windows x64)
//   "windows": { exeName, icon, company, copyright, description, console }
//   "macos": { bundleId, exeName, icon, category, minimumSystemVersion, copyright,
//              microphoneUsageDescription, cameraUsageDescription, sign }
//
// Unknown keys under `windows` and `macos` are ERRORS (a typo'd "icno" must not ship an iconless
// exe silently); unknown top-level keys are warnings, since the file is shared with other tools.

import { existsSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, isAbsolute, join, resolve, sep } from "node:path";
import type { Platform } from "./platform";
import { numericVersion } from "./versioninfo";

/** A prod host's name, the key the host cache and the release find it by: the whole engine, with or without devtools. */
export type Variant = string;
export const VARIANTS: readonly Variant[] = ["standard", "no-devtools"];

/** The cargo features of sk-app every published host carries (crates/sk-app/src/payload.rs `OPTIONAL`, after devtools). */
export const ENGINE_FEATURES = ["audio", "clipboard", "storages", "indexeddb", "webgpu"] as const;

export const PARTS_NOT_SELECTABLE =
  "engine parts are no longer selectable: every published app carries the whole engine. " +
  '"publishDevtools": true in sk.config.json keeps devtools in sk publish builds; ' +
  "to trim the engine, embed it with the sk-app crate (sk_app::embed) and its cargo features";

export function parseVariant(v: unknown, where: string): Variant {
  const name = typeof v === "string" ? v.trim() : "";
  if (VARIANTS.includes(name)) return name;
  throw new ConfigError(`${where}: ${JSON.stringify(v)} is not a host variant — ${VARIANTS.join(" or ")}`);
}

export function cargoFeatureArgs(variant: Variant): string[] {
  return ["--features", parseVariant(variant, "cargo") === "no-devtools" ? "prod,no-devtools" : "prod"];
}

export interface WindowsConfig {
  exeName?: string | undefined;
  icon?: string | undefined;
  company?: string | undefined;
  copyright?: string | undefined;
  description?: string | undefined;
  console?: boolean;
}

/** `"macos"`, defaults applied. Read on every platform, so a typo fails a Windows build too. */
export interface MacConfig {
  bundleId: string;
  bundleIdFrom: "macos.bundleId" | "id";
  exeName: string;
  iconPath: string | null;
  category: string | null;
  minimumSystemVersion: string;
  copyright: string | null;
  microphoneUsageDescription: string | null;
  cameraUsageDescription: string | null;
  /** A codesign identity; `"-"` signs ad hoc; `false` leaves the bundle unsigned. `undefined`: ad hoc wherever `codesign` exists. */
  sign: string | false | undefined;
}

export const MACOS_MINIMUM = "13.0";

export interface AppConfig {
  appRoot: string;
  entry: string;
  configPath: string | null;
  raw: Record<string, unknown>;
  name: string;
  productName: string;
  version: string;
  versionFrom: "sk.config.json" | "package.json" | "default";
  assets: string[];
  assetsDeclared: boolean;
  publicDir: string | null;
  packs: PackConfig[];
  /** Whether `sk build` and `sk publish` compile the app to a native image (`--aot` / `--no-aot` win). */
  aot: boolean;
  /** Whether `sk publish` stamps the standard host (devtools in) instead of no-devtools. */
  publishDevtools: boolean;
  windows: WindowsConfig;
  macos: MacConfig;
  exeName: string;
  iconPath: string | null;
  id: string;
  launch: LaunchConfig;
  /** The OSes `sk publish` packages for; null: this OS only. */
  platforms: Platform[] | null;
  warnings: string[];
}

/** One asset pack: `<name>.skpk` beside the payload, holding the assets whose key matches `match` (none: every asset no earlier pack took). */
export interface PackConfig {
  name: string;
  match: string[] | null;
}

const PACK_NAME = /^[a-z0-9][a-z0-9_-]{0,63}$/;
export const PACK_KEYS: ReadonlySet<string> = new Set(["name", "match"]);

function parsePacks(raw: unknown): PackConfig[] {
  if (raw === undefined) return [];
  const bad = (why: string) => new ConfigError(`sk.config.json "packs": ${why}`);
  if (!Array.isArray(raw)) throw bad('must be an array of { "name": …, "match"?: [globs] }');
  const seen = new Set<string>();
  return raw.map((p, i) => {
    if (!p || typeof p !== "object" || Array.isArray(p)) throw bad(`entry ${i} must be an object`);
    const { name, match } = p as Record<string, unknown>;
    const extra = Object.keys(p).filter((k) => !k.startsWith("$") && !PACK_KEYS.has(k));
    if (extra.length)
      throw bad(
        `entry ${i} has unknown key(s) ${extra.map((k) => JSON.stringify(k)).join(", ")} — known keys are ${[...PACK_KEYS].join(", ")}`,
      );
    if (typeof name !== "string" || !PACK_NAME.test(name) || name === "app") {
      throw bad(
        `entry ${i}: "name" must be lowercase letters, digits, "-" or "_" (and not "app") — it names the file <name>.skpk`,
      );
    }
    if (seen.has(name)) throw bad(`two packs are named ${JSON.stringify(name)}`);
    seen.add(name);
    if (
      match !== undefined &&
      (!Array.isArray(match) || !match.length || match.some((m) => typeof m !== "string" || !m.trim()))
    ) {
      throw bad(
        `${JSON.stringify(name)}: "match" must be a non-empty array of globs over asset keys, or left out to take the rest`,
      );
    }
    return { name, match: match === undefined ? null : (match as string[]).map((m) => m.trim()) };
  });
}

export interface LaunchConfig {
  id: string;
  singleInstance: boolean;
  raiseOnSecondInstance: boolean;
  protocols: string[];
}

export const TOP_LEVEL: ReadonlySet<string> = new Set([
  "name",
  "platforms",
  "productName",
  "version",
  "window",
  "devtools",
  "publishDevtools",
  "aot",
  "httpCache",
  "assets",
  "publicDir",
  "packs",
  "define",
  "aliases",
  "check",
  "windows",
  "macos",
  "id",
  "singleInstance",
  "raiseOnSecondInstance",
  "protocols",
]);

const SCHEME = /^[a-z][a-z0-9+.-]{0,62}$/;
/** Schemes an app may never claim; the host refuses them too, this makes it a build error. */
const RESERVED_SCHEMES = new Set([
  "http",
  "https",
  "file",
  "ftp",
  "mailto",
  "tel",
  "data",
  "blob",
  "javascript",
  "about",
  "vbscript",
  "search-ms",
]);
/** An id names a kernel object and a registry value: keep it to a safe alphabet. */
const ID = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;

function readLaunch(raw: Record<string, unknown>, name: string): LaunchConfig {
  const bool = (k: string, dflt: boolean): boolean => {
    const v = raw[k];
    if (v === undefined) return dflt;
    if (typeof v !== "boolean") throw new ConfigError(`sk.config.json "${k}" must be true or false`);
    return v;
  };
  let id = name;
  if (raw.id !== undefined) {
    if (typeof raw.id !== "string" || !ID.test(raw.id.trim())) {
      throw new ConfigError(
        'sk.config.json "id" must be a reverse-DNS style identifier (letters, digits, ".", "_", "-"), e.g. "gl.argon.notes"',
      );
    }
    id = raw.id.trim();
  }
  const protocols: string[] = [];
  if (raw.protocols !== undefined) {
    if (!Array.isArray(raw.protocols))
      throw new ConfigError('sk.config.json "protocols" must be an array of scheme names');
    for (const p of raw.protocols) {
      const s = typeof p === "string" ? p.trim().toLowerCase().replace(/:\/*$/, "") : "";
      if (!SCHEME.test(s)) {
        throw new ConfigError(
          `sk.config.json "protocols": ${JSON.stringify(p)} is not a URL scheme (a letter, then letters, digits, "+", "-" or ".")`,
        );
      }
      if (RESERVED_SCHEMES.has(s) || s.startsWith("ms-")) {
        throw new ConfigError(
          `sk.config.json "protocols": "${s}" belongs to the browser or to Windows and cannot be registered by an app`,
        );
      }
      if (!protocols.includes(s)) protocols.push(s);
    }
  }
  return {
    id,
    singleInstance: bool("singleInstance", false),
    raiseOnSecondInstance: bool("raiseOnSecondInstance", true),
    protocols,
  };
}
export const WINDOWS_KEYS: ReadonlySet<string> = new Set([
  "exeName",
  "icon",
  "company",
  "copyright",
  "description",
  "console",
]);
const REMOVED_WINDOWS_KEYS = new Set(["host", "features", "without"]);
export const MACOS_KEYS: ReadonlySet<string> = new Set([
  "bundleId",
  "exeName",
  "icon",
  "category",
  "minimumSystemVersion",
  "copyright",
  "microphoneUsageDescription",
  "cameraUsageDescription",
  "sign",
]);
const BUNDLE_ID = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/;
const CATEGORY = /^public\.app-category\.[a-z]+(-[a-z]+)*$/;

function readMacos(
  raw: unknown,
  appRoot: string,
  fallback: { id: string; productName: string; name: string; icon: string | null; copyright?: string | undefined },
  warnings: string[],
): MacConfig {
  if (raw !== undefined && (!raw || typeof raw !== "object" || Array.isArray(raw))) {
    throw new ConfigError('sk.config.json "macos" must be an object');
  }
  const m = (raw ?? {}) as Record<string, unknown>;
  for (const key of Object.keys(m)) {
    if (key.startsWith("$")) continue;
    if (!MACOS_KEYS.has(key)) {
      throw new ConfigError(
        `sk.config.json "macos": unknown key ${JSON.stringify(key)} — known keys are ${[...MACOS_KEYS].join(", ")}`,
      );
    }
  }
  const where = (k: string) => `sk.config.json macos.${k}`;

  const explicitId = str(m.bundleId, where("bundleId"));
  if (explicitId !== undefined && !BUNDLE_ID.test(explicitId)) {
    throw new ConfigError(
      `sk.config.json "macos.bundleId": ${JSON.stringify(explicitId)} is not a bundle identifier — reverse-DNS, ` +
        'letters, digits, "-" and "." only, e.g. "gl.argon.notes"',
    );
  }
  // "id" allows "_", which a bundle identifier does not.
  const bundleId = explicitId ?? fallback.id.replace(/_/g, "-");

  let category: string | null = null;
  const cat = str(m.category, where("category"));
  if (cat !== undefined) {
    category = cat.startsWith("public.app-category.") ? cat : `public.app-category.${cat}`;
    if (!CATEGORY.test(category)) {
      throw new ConfigError(
        `sk.config.json "macos.category": ${JSON.stringify(cat)} is not an app category — e.g. "developer-tools" ` +
          'or "public.app-category.social-networking" (Apple\'s LSApplicationCategoryType list)',
      );
    }
  }

  const min = str(m.minimumSystemVersion, where("minimumSystemVersion")) ?? MACOS_MINIMUM;
  if (!/^\d+(\.\d+){0,2}$/.test(min)) {
    throw new ConfigError(
      `sk.config.json "macos.minimumSystemVersion": ${JSON.stringify(min)} is not a version like "13.0"`,
    );
  }
  const [maj = 0, minor = 0] = min.split(".").map(Number);
  const [wantMaj = 0, wantMinor = 0] = MACOS_MINIMUM.split(".").map(Number);
  if (maj < wantMaj || (maj === wantMaj && minor < wantMinor)) {
    warnings.push(
      `sk.config.json "macos.minimumSystemVersion" ${min} is below ${MACOS_MINIMUM}, which the host is built for — ` +
        "an older macOS will offer to open the app and then fail to load it",
    );
  }

  let sign: MacConfig["sign"];
  if (m.sign !== undefined) {
    if (m.sign === false) sign = false;
    else if (typeof m.sign === "string" && m.sign.trim()) sign = m.sign.trim();
    else {
      throw new ConfigError(
        'sk.config.json "macos.sign" must be a codesign identity, "-" (ad hoc) or false (leave unsigned)',
      );
    }
  }

  let iconPath = fallback.icon;
  const icon = str(m.icon, where("icon"));
  if (icon) {
    iconPath = isAbsolute(icon) ? icon : join(appRoot, icon);
    if (!existsSync(iconPath)) throw new ConfigError(`sk.config.json "macos.icon": ${iconPath} does not exist`);
  }

  const exeName =
    sanitizeExeName(str(m.exeName, where("exeName")) ?? fallback.productName) ||
    sanitizeExeName(fallback.name) ||
    "app";
  return {
    bundleId,
    bundleIdFrom: explicitId ? "macos.bundleId" : "id",
    exeName,
    iconPath,
    category,
    minimumSystemVersion: min,
    copyright: str(m.copyright, where("copyright")) ?? fallback.copyright ?? null,
    microphoneUsageDescription: str(m.microphoneUsageDescription, where("microphoneUsageDescription")) ?? null,
    cameraUsageDescription: str(m.cameraUsageDescription, where("cameraUsageDescription")) ?? null,
    sign,
  };
}
const ENTRY_CANDIDATES = ["src/main.ts", "src/main.js", "src/main.mts", "main.ts", "main.js"];

export class ConfigError extends Error {}

export function resolveApp(target: string): { appRoot: string; entry: string } {
  const abs = resolve(target);
  if (!existsSync(abs)) throw new ConfigError(`${target}: no such file or directory`);
  if (statSync(abs).isDirectory()) {
    for (const rel of ENTRY_CANDIDATES) {
      const entry = join(abs, rel);
      if (existsSync(entry)) return { appRoot: abs, entry };
    }
    throw new ConfigError(
      `${target}: no entry module — looked for ${ENTRY_CANDIDATES.join(", ")}. Pass the entry file instead.`,
    );
  }
  if (basename(abs) === "sk.config.json") return resolveApp(dirname(abs));
  const srcDir = dirname(abs);
  const appRoot = srcDir.endsWith(`${sep}src`) ? dirname(srcDir) : srcDir;
  return { appRoot, entry: abs };
}

export function sanitizeExeName(name: string): string {
  let s = name
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "")
    .trim()
    .replace(/[. ]+$/, "");
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i.test(s)) s += "-app";
  return s.slice(0, 120);
}

function readJson(path: string): Record<string, unknown> {
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (e) {
    throw new ConfigError(`${path}: ${(e as Error).message}`);
  }
  try {
    const v = JSON.parse(text);
    if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("not a JSON object");
    return v as Record<string, unknown>;
  } catch (e) {
    throw new ConfigError(`${path}: ${(e as Error).message}`);
  }
}

const str = (v: unknown, where: string): string | undefined => {
  if (v === undefined) return undefined;
  if (typeof v !== "string") throw new ConfigError(`${where} must be a string`);
  return v.trim() || undefined;
};

/** A config override changes configuration only; entry and relative assets keep the target's app root. */
export function loadAppConfig(
  target: string,
  options: { configPath?: string | undefined; platform?: Platform | undefined } = {},
): AppConfig {
  const { appRoot, entry } = resolveApp(target);
  if (options.configPath !== undefined && !options.configPath.trim()) throw new ConfigError("--config needs a path");
  const configPath = options.configPath === undefined ? join(appRoot, "sk.config.json") : resolve(options.configPath);
  const has = existsSync(configPath);
  if (options.configPath !== undefined && !has) throw new ConfigError(`${configPath}: no such config file`);
  const raw = has ? readJson(configPath) : {};
  const where = (k: string) => `sk.config.json ${k}`;
  const warnings: string[] = [];

  for (const key of Object.keys(raw)) {
    if (key.startsWith("$") || TOP_LEVEL.has(key)) continue;
    warnings.push(`sk.config.json: unknown key ${JSON.stringify(key)} (ignored by sk build)`);
  }

  const name = str(raw.name, where('"name"')) ?? basename(appRoot);
  const launch = readLaunch(raw, name);
  if (launch.protocols.length && !launch.singleInstance) {
    warnings.push(
      'sk.config.json declares "protocols" without "singleInstance": every link to them will start a new instance of the app',
    );
  }
  const win = raw.window && typeof raw.window === "object" ? (raw.window as Record<string, unknown>) : {};
  const title = typeof win.title === "string" && win.title.trim() ? win.title.trim() : undefined;
  const productName = str(raw.productName, where('"productName"')) ?? title ?? name;

  let version = str(raw.version, where('"version"'));
  let versionFrom: AppConfig["versionFrom"] = "sk.config.json";
  if (!version) {
    const pkgPath = join(appRoot, "package.json");
    const pkgVersion = existsSync(pkgPath) ? readJson(pkgPath).version : undefined;
    if (typeof pkgVersion === "string" && pkgVersion.trim()) {
      version = pkgVersion.trim();
      versionFrom = "package.json";
    } else {
      version = "0.0.0";
      versionFrom = "default";
      warnings.push('no version: set sk.config.json "version" (or package.json "version") — stamping 0.0.0');
    }
  }
  try {
    numericVersion(version);
  } catch (e) {
    throw new ConfigError(`${versionFrom} "version": ${(e as Error).message}`);
  }

  let assets = ["assets"];
  const assetsDeclared = raw.assets !== undefined;
  if (assetsDeclared) {
    if (!Array.isArray(raw.assets) || raw.assets.some((a) => typeof a !== "string" || !a.trim())) {
      throw new ConfigError('sk.config.json "assets" must be an array of non-empty strings (directories or globs)');
    }
    assets = (raw.assets as string[]).map((a) => a.trim());
  }

  // Vite's `publicDir`, opt-in here: on by default it would re-key a package that already ships a public/ directory.
  let publicDir: string | null = null;
  if (raw.publicDir !== undefined && raw.publicDir !== false) {
    const p =
      typeof raw.publicDir === "string"
        ? raw.publicDir.trim().replace(/\\/g, "/").replace(/^\.\//, "").replace(/\/+$/, "")
        : "";
    if (!p || p.startsWith("/") || /^[a-zA-Z]:/.test(p) || p.split("/").includes("..")) {
      throw new ConfigError(
        'sk.config.json "publicDir" must be a directory relative to the app root, inside it (or false)',
      );
    }
    const abs = join(appRoot, p);
    if (!existsSync(abs) || !statSync(abs).isDirectory()) {
      warnings.push(
        `sk.config.json "publicDir": ${JSON.stringify(p)} is not a directory — nothing is packaged from it`,
      );
    }
    publicDir = p;
  }
  const packs = parsePacks(raw.packs);
  if (raw.define !== undefined && (!raw.define || typeof raw.define !== "object" || Array.isArray(raw.define))) {
    throw new ConfigError('sk.config.json "define" must be an object of { "NAME": value }');
  }

  if (raw.publishDevtools !== undefined && typeof raw.publishDevtools !== "boolean") {
    throw new ConfigError('sk.config.json "publishDevtools" must be true or false');
  }
  const publishDevtools = raw.publishDevtools === true;
  if (raw.aot !== undefined && typeof raw.aot !== "boolean") {
    throw new ConfigError('sk.config.json "aot" must be true or false');
  }

  const windows: WindowsConfig = {};
  if (raw.windows !== undefined) {
    if (!raw.windows || typeof raw.windows !== "object" || Array.isArray(raw.windows)) {
      throw new ConfigError('sk.config.json "windows" must be an object');
    }
    const w = raw.windows as Record<string, unknown>;
    for (const key of Object.keys(w)) {
      if (key.startsWith("$")) continue;
      if (REMOVED_WINDOWS_KEYS.has(key)) {
        throw new ConfigError(`sk.config.json "windows.${key}": ${PARTS_NOT_SELECTABLE}`);
      }
      if (!WINDOWS_KEYS.has(key)) {
        throw new ConfigError(
          `sk.config.json "windows": unknown key ${JSON.stringify(key)} — known keys are ${[...WINDOWS_KEYS].join(", ")}`,
        );
      }
    }
    windows.exeName = str(w.exeName, where("windows.exeName"));
    windows.icon = str(w.icon, where("windows.icon"));
    windows.company = str(w.company, where("windows.company"));
    windows.copyright = str(w.copyright, where("windows.copyright"));
    windows.description = str(w.description, where("windows.description"));
    if (w.console !== undefined) {
      if (typeof w.console !== "boolean")
        throw new ConfigError('sk.config.json "windows.console" must be true or false');
      windows.console = w.console;
    }
  }

  if (raw.httpCache !== undefined && raw.httpCache !== false) {
    const c = raw.httpCache as Record<string, unknown> | null;
    const mb = c && typeof c === "object" ? c.maxMB : undefined;
    if (typeof mb !== "number" || !(mb >= 0)) {
      throw new ConfigError('sk.config.json "httpCache" must be false or { "maxMB": <MB, 0 = no cache> }');
    }
  }

  const exeName = sanitizeExeName(windows.exeName ?? productName) || sanitizeExeName(name) || "app";
  let iconPath: string | null = null;
  if (windows.icon) {
    iconPath = isAbsolute(windows.icon) ? windows.icon : join(appRoot, windows.icon);
    if (!existsSync(iconPath)) throw new ConfigError(`sk.config.json "windows.icon": ${iconPath} does not exist`);
  }
  const macos = readMacos(
    raw.macos,
    appRoot,
    { id: launch.id, productName, name, icon: iconPath, copyright: windows.copyright },
    warnings,
  );
  if (options.platform === "macos") {
    if (!macos.iconPath) {
      warnings.push(
        'no icon: set sk.config.json "macos.icon" (or "windows.icon") to a square .png or an .icns — the app gets the generic macOS icon',
      );
    }
  } else if (options.platform !== "linux" && !windows.icon) {
    warnings.push(
      'no icon: set sk.config.json "windows.icon" to an .ico or a square .png — the exe gets the default Windows icon',
    );
  }

  return {
    appRoot,
    entry,
    configPath: has ? configPath : null,
    raw,
    name,
    productName,
    version,
    versionFrom,
    assets,
    assetsDeclared,
    publicDir,
    packs,
    publishDevtools,
    aot: raw.aot === true,
    windows,
    macos,
    exeName,
    iconPath,
    id: launch.id,
    launch,
    platforms: readPlatforms(raw.platforms),
    warnings,
  };
}

const PLATFORM_NAMES: readonly Platform[] = ["windows", "macos", "linux"];

function readPlatforms(v: unknown): Platform[] | null {
  if (v === undefined) return null;
  if (!Array.isArray(v) || !v.length || !v.every((p): p is Platform => PLATFORM_NAMES.includes(p as Platform))) {
    throw new ConfigError(`sk.config.json "platforms" must be a non-empty list of ${PLATFORM_NAMES.join(", ")}`);
  }
  return [...new Set(v)];
}
