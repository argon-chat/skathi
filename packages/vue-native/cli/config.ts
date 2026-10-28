// `vn.config.json`, as `vn build` reads it.
//
// The file is the one the dev builder already reads (`build.ts` copies its
// `name`, `window` and `devtools` into the dev manifest); `vn build` adds the
// keys only a packaged exe needs (research/reports/m25/packaging-design.md
// §4.8):
//
//   {
//     "name": "hello",            identity: %LOCALAPPDATA%\<name>\storage, the devtools slot
//     "productName": "Hello",     display name — default window.title ?? name
//     "version": "1.2.3",         default: the app's package.json version
//     "window": { … },            unchanged
//     "devtools": true,           unchanged (a runtime switch)
//     "id": "gl.argon.hello",     launch identity (instance key, protocol owner) — default name
//     "singleInstance": true,     one process per user session; a 2nd launch forwards (docs/LAUNCH.md)
//     "raiseOnSecondInstance": true,  bring the window forward on a forwarded launch (default true)
//     "protocols": ["hello"],     custom URL schemes, registered per-user at startup (prod)
//     "assets": ["assets"],       what is packaged: dirs/globs relative to the app root
//     "windows": {                Windows exe identity — only `vn build` reads it
//       "exeName": "Hello", "icon": "icon.png", "company": "…", "copyright": "…",
//       "description": "…", "console": false, "host": "standard" | "no-devtools"
//     }
//   }
//
// Unknown keys under `windows` are ERRORS: a typo'd `"icno"` must not ship an
// iconless exe without a word. Unknown top-level keys are warnings, because the
// file is shared with the dev builder and other tools.

import { existsSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, isAbsolute, join, resolve, sep } from "node:path";
import { numericVersion } from "./versioninfo";

export type Variant = "standard" | "no-devtools";
export const VARIANTS: Variant[] = ["standard", "no-devtools"];

export interface WindowsConfig {
  exeName?: string;
  icon?: string;
  company?: string;
  copyright?: string;
  description?: string;
  console?: boolean;
  host?: Variant;
}

export interface AppConfig {
  /** The directory a relative URL resolves against — `apps/<app>/`, where `vn.config.json` lives. */
  appRoot: string;
  entry: string;
  configPath: string | null;
  /** The raw parsed file (`{}` when there is none). */
  raw: Record<string, unknown>;
  name: string;
  productName: string;
  version: string;
  versionFrom: "vn.config.json" | "package.json" | "default";
  /** `vn.config.json` `assets`, or the default `["assets"]`. */
  assets: string[];
  assetsDeclared: boolean;
  windows: WindowsConfig;
  exeName: string;
  iconPath: string | null;
  /** `"id"`, or the name. */
  id: string;
  /** The launch keys, defaults applied (docs/LAUNCH.md). */
  launch: LaunchConfig;
  warnings: string[];
}

/** The `launch` object `vn build` writes into the payload's META. */
export interface LaunchConfig {
  id: string;
  singleInstance: boolean;
  raiseOnSecondInstance: boolean;
  protocols: string[];
}

const TOP_LEVEL = new Set([
  "name",
  "productName",
  "version",
  "window",
  "devtools",
  "assets",
  "windows",
  "id",
  "singleInstance",
  "raiseOnSecondInstance",
  "protocols",
]);

/** RFC 3986 scheme, lowercase, as the host registers it. */
const SCHEME = /^[a-z][a-z0-9+.-]{0,62}$/;
/**
 * Schemes an app may never claim — the browser's own, the shell's, and
 * Windows' `ms-*` / `search-ms`. The host refuses them too; refusing here is
 * what makes it a build error instead of a startup advisory.
 */
const RESERVED_SCHEMES = new Set([
  "http", "https", "file", "ftp", "mailto", "tel", "data", "blob", "javascript", "about", "vbscript", "search-ms",
]);
/** An id names a kernel object and a registry value: keep it to a safe alphabet. */
const ID = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;

function readLaunch(raw: Record<string, unknown>, name: string): LaunchConfig {
  const bool = (k: string, dflt: boolean): boolean => {
    const v = raw[k];
    if (v === undefined) return dflt;
    if (typeof v !== "boolean") throw new ConfigError(`vn.config.json "${k}" must be true or false`);
    return v;
  };
  let id = name;
  if (raw.id !== undefined) {
    if (typeof raw.id !== "string" || !ID.test(raw.id.trim())) {
      throw new ConfigError(
        'vn.config.json "id" must be a reverse-DNS style identifier (letters, digits, ".", "_", "-"), e.g. "gl.argon.notes"',
      );
    }
    id = raw.id.trim();
  }
  const protocols: string[] = [];
  if (raw.protocols !== undefined) {
    if (!Array.isArray(raw.protocols)) throw new ConfigError('vn.config.json "protocols" must be an array of scheme names');
    for (const p of raw.protocols) {
      const s = typeof p === "string" ? p.trim().toLowerCase().replace(/:\/*$/, "") : "";
      if (!SCHEME.test(s)) {
        throw new ConfigError(`vn.config.json "protocols": ${JSON.stringify(p)} is not a URL scheme (a letter, then letters, digits, "+", "-" or ".")`);
      }
      if (RESERVED_SCHEMES.has(s) || s.startsWith("ms-")) {
        throw new ConfigError(`vn.config.json "protocols": "${s}" belongs to the browser or to Windows and cannot be registered by an app`);
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
const WINDOWS_KEYS = new Set(["exeName", "icon", "company", "copyright", "description", "console", "host"]);
const ENTRY_CANDIDATES = ["src/main.ts", "src/main.js", "src/main.mts", "main.ts", "main.js"];

export class ConfigError extends Error {}

/** `apps/hello`, `apps/hello/src/main.ts` or `apps/hello/vn.config.json` → the app root and its entry. */
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
  if (basename(abs) === "vn.config.json") return resolveApp(dirname(abs));
  // An entry file: the app root is its directory with a trailing `src/`
  // stripped — the SAME rule build.ts/prod.ts use for VN_CONFIG_DIR and the
  // dev host uses for the asset base, so all three agree on "the document".
  const srcDir = dirname(abs);
  const appRoot = srcDir.endsWith(`${sep}src`) ? dirname(srcDir) : srcDir;
  return { appRoot, entry: abs };
}

/** A Windows file name: no reserved characters, no device names, never empty. */
export function sanitizeExeName(name: string): string {
  let s = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "").trim().replace(/[. ]+$/, "");
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

export function loadAppConfig(target: string): AppConfig {
  const { appRoot, entry } = resolveApp(target);
  const configPath = join(appRoot, "vn.config.json");
  const has = existsSync(configPath);
  const raw = has ? readJson(configPath) : {};
  const where = (k: string) => `vn.config.json ${k}`;
  const warnings: string[] = [];

  for (const key of Object.keys(raw)) {
    if (key.startsWith("$") || TOP_LEVEL.has(key)) continue;
    warnings.push(`vn.config.json: unknown key ${JSON.stringify(key)} (ignored by vn build)`);
  }

  const name = str(raw.name, where('"name"')) ?? basename(appRoot);
  const launch = readLaunch(raw, name);
  if (launch.protocols.length && !launch.singleInstance) {
    // A-F4, at build time as well as at startup.
    warnings.push(
      'vn.config.json declares "protocols" without "singleInstance": every link to them will start a new instance of the app',
    );
  }
  const win = raw.window && typeof raw.window === "object" ? (raw.window as Record<string, unknown>) : {};
  const title = typeof win.title === "string" && win.title.trim() ? win.title.trim() : undefined;
  const productName = str(raw.productName, where('"productName"')) ?? title ?? name;

  let version = str(raw.version, where('"version"'));
  let versionFrom: AppConfig["versionFrom"] = "vn.config.json";
  if (!version) {
    const pkgPath = join(appRoot, "package.json");
    const pkgVersion = existsSync(pkgPath) ? readJson(pkgPath).version : undefined;
    if (typeof pkgVersion === "string" && pkgVersion.trim()) {
      version = pkgVersion.trim();
      versionFrom = "package.json";
    } else {
      version = "0.0.0";
      versionFrom = "default";
      warnings.push('no version: set vn.config.json "version" (or package.json "version") — stamping 0.0.0');
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
      throw new ConfigError('vn.config.json "assets" must be an array of non-empty strings (directories or globs)');
    }
    assets = (raw.assets as string[]).map((a) => a.trim());
  }

  const windows: WindowsConfig = {};
  if (raw.windows !== undefined) {
    if (!raw.windows || typeof raw.windows !== "object" || Array.isArray(raw.windows)) {
      throw new ConfigError('vn.config.json "windows" must be an object');
    }
    const w = raw.windows as Record<string, unknown>;
    for (const key of Object.keys(w)) {
      if (key.startsWith("$")) continue;
      if (!WINDOWS_KEYS.has(key)) {
        throw new ConfigError(
          `vn.config.json "windows": unknown key ${JSON.stringify(key)} — known keys are ${[...WINDOWS_KEYS].join(", ")}`,
        );
      }
    }
    windows.exeName = str(w.exeName, where('windows.exeName'));
    windows.icon = str(w.icon, where('windows.icon'));
    windows.company = str(w.company, where('windows.company'));
    windows.copyright = str(w.copyright, where('windows.copyright'));
    windows.description = str(w.description, where('windows.description'));
    if (w.console !== undefined) {
      if (typeof w.console !== "boolean") throw new ConfigError('vn.config.json "windows.console" must be true or false');
      windows.console = w.console;
    }
    if (w.host !== undefined) {
      if (w.host !== "standard" && w.host !== "no-devtools") {
        throw new ConfigError('vn.config.json "windows.host" must be "standard" or "no-devtools"');
      }
      windows.host = w.host;
    }
  }

  const exeName = sanitizeExeName(windows.exeName ?? productName) || sanitizeExeName(name) || "app";
  let iconPath: string | null = null;
  if (windows.icon) {
    iconPath = isAbsolute(windows.icon) ? windows.icon : join(appRoot, windows.icon);
    if (!existsSync(iconPath)) throw new ConfigError(`vn.config.json "windows.icon": ${iconPath} does not exist`);
  } else {
    warnings.push('no icon: set vn.config.json "windows.icon" to an .ico or a square .png — the exe gets the default Windows icon');
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
    windows,
    exeName,
    iconPath,
    id: launch.id,
    launch,
    warnings,
  };
}
