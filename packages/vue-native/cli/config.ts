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
//     "httpCache": { "maxMB": 256 },  the disk HTTP cache's cap; 0 or false = none (docs/STORAGE.md §12)
//     "id": "gl.argon.hello",     launch identity (instance key, protocol owner) — default name
//     "singleInstance": true,     one process per user session; a 2nd launch forwards (docs/LAUNCH.md)
//     "raiseOnSecondInstance": true,  bring the window forward on a forwarded launch (default true)
//     "protocols": ["hello"],     custom URL schemes, registered per-user at startup (prod)
//     "assets": ["assets"],       what is packaged: dirs/globs relative to the app root
//     "publicDir": "public",      Vite's public/: packaged with keys relative to IT (served at /)
//     "define": { "K": … },       Vite's `define` — read by the builders (vitecompat.ts appDefines)
//     "windows": {                Windows exe identity — only `vn build` reads it
//       "exeName": "Hello", "icon": "icon.png", "company": "…", "copyright": "…",
//       "description": "…", "console": false,
//       "without": ["voice"]      the engine parts the host leaves out (OPTIONAL below);
//                                 or "features": [...] — the parts it TAKES;
//                                 or "host": "no-devtools" — a whole variant, by name
//     },
//     "macos": {                  the .app's identity — only `vn build --platform macos` uses it
//       "bundleId": "gl.argon.hello",   CFBundleIdentifier — default "id"
//       "exeName": "Hello", "icon": "icon.png", "category": "developer-tools",
//       "minimumSystemVersion": "13.0", "copyright": "…",
//       "microphoneUsageDescription": "…", "cameraUsageDescription": "…",
//       "sign": "Developer ID Application: …"   or "-" (ad-hoc) or false
//     }
//   }
//
// Unknown keys under `windows` and `macos` are ERRORS: a typo'd `"icno"` must
// not ship an iconless exe without a word. Unknown top-level keys are warnings,
// because the file is shared with the dev builder and other tools.

import { existsSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, isAbsolute, join, resolve, sep } from "node:path";
import type { Platform } from "./platform";
import { numericVersion } from "./versioninfo";

/**
 * The parts of the engine a host can be built WITHOUT (docs/PACKAGING.md
 * §7.1) — the same list, in the same order, as `OPTIONAL` in
 * crates/vn-app/src/payload.rs, which is what a host's `variant` is named
 * from. Every one is in a host unless it is left out:
 *
 * - `devtools`  — the devtools listener, the embedded inspector, F12
 *                 (`--features no-devtools`; a runtime switch exists too);
 * - `voice`     — `livekit-client`: LiveKit and the prebuilt libwebrtc;
 * - `audio`     — Web Audio and `getUserMedia`;
 * - `clipboard` — `navigator.clipboard`;
 * - `storages`  — `localStorage` / `sessionStorage` / `appStorage`;
 * - `indexeddb` — `indexedDB` and `navigator.storage`;
 * - `webgpu`    — native GPU compute/rendering and canvas presentation.
 *
 * Each but `devtools` is a default cargo feature of vn-app of the same name.
 */
export const OPTIONAL = ["devtools", "voice", "audio", "clipboard", "storages", "indexeddb", "webgpu"] as const;
export type Part = (typeof OPTIONAL)[number];

/**
 * A host's name for what is in it: `standard` with every part, else the parts
 * left out as `no-<part>` joined by `+`, in {@link OPTIONAL}'s order —
 * `no-devtools`, `no-voice`, `no-devtools+no-voice`. The host cache is keyed
 * by it, and a host is asked for by it.
 */
export type Variant = string;
/** What `vn host build` builds with no selection: the two pre-features variants. */
export const VARIANTS: Variant[] = ["standard", "no-devtools"];

const isPart = (p: unknown): p is Part => typeof p === "string" && (OPTIONAL as readonly string[]).includes(p);

/** The variant of a host built without `without`. */
export function variantFor(without: Iterable<Part>): Variant {
  const off = new Set(without);
  const names = OPTIONAL.filter((p) => off.has(p)).map((p) => `no-${p}`);
  return names.length ? names.join("+") : "standard";
}

/** The parts a variant leaves out; an unknown name is a ConfigError. */
export function partsLeftOut(variant: Variant): Part[] {
  if (variant === "standard") return [];
  const off = new Set<Part>();
  for (const name of variant.split("+")) {
    const p = name.startsWith("no-") ? name.slice(3) : "";
    if (!isPart(p)) {
      throw new ConfigError(
        `${JSON.stringify(variant)} is not a host variant — "standard", or the parts left out as no-<part> joined by "+" ` +
          `(parts: ${OPTIONAL.join(", ")})`,
      );
    }
    off.add(p);
  }
  return OPTIONAL.filter((p) => off.has(p));
}

/**
 * A list of part names, checked: `"voice,audio"`, or an array of them. Spaces
 * separate too: PowerShell hands an unquoted `voice,audio` over as
 * `"voice audio"`.
 */
export function parseParts(list: unknown, where: string): Part[] {
  const items = typeof list === "string" ? list.split(/[\s,]+/).filter(Boolean) : list;
  if (!Array.isArray(items)) throw new ConfigError(`${where} must be a list of parts (${OPTIONAL.join(", ")})`);
  const out = new Set<Part>();
  for (const p of items) {
    if (!isPart(p)) throw new ConfigError(`${where}: ${JSON.stringify(p)} is not a part — known parts are ${OPTIONAL.join(", ")}`);
    out.add(p);
  }
  return OPTIONAL.filter((p) => out.has(p));
}

/**
 * The variant a selection asks for. `features` names what to TAKE (everything
 * else is left out); `without` names what to leave out; `host` is the
 * pre-features spelling of a whole variant. One of them, or none (`standard`).
 */
export function selectVariant(sel: { features?: Part[]; without?: Part[]; host?: Variant }, where: string): Variant {
  const given = [sel.features && "features", sel.without && "without", sel.host && "host"].filter(Boolean);
  if (given.length > 1) throw new ConfigError(`${where}: give one of ${given.join(" / ")}, not several`);
  if (sel.features) return variantFor(OPTIONAL.filter((p) => !sel.features!.includes(p)));
  if (sel.without) return variantFor(sel.without);
  if (sel.host) return variantFor(partsLeftOut(sel.host));
  return "standard";
}

/** What a host of `variant` will do that its author may not expect. */
export function variantWarnings(variant: Variant): string[] {
  const off = new Set(partsLeftOut(variant));
  const out: string[] = [];
  if (!off.has("voice") && off.has("audio")) {
    out.push("the host has voice but no audio: a voice room connects, but nothing it receives is heard");
  }
  if (!off.has("indexeddb") && off.has("storages")) {
    out.push("the host has indexedDB but no storages: its databases are in memory and nothing survives a restart");
  }
  return out;
}

/**
 * The cargo arguments that build a bare host of `variant` (after `cargo build
 * --release -p vn-app`). A host that leaves out none of the default features
 * keeps `--features prod[,no-devtools]`, the command line both pre-features
 * variants always had.
 */
export function cargoFeatureArgs(variant: Variant): string[] {
  const off = partsLeftOut(variant);
  const features = ["prod", ...(off.includes("devtools") ? ["no-devtools"] : [])];
  const defaults = OPTIONAL.filter((p) => p !== "devtools");
  if (!off.some((p) => p !== "devtools")) return ["--features", features.join(",")];
  return ["--no-default-features", "--features", [...features, ...defaults.filter((p) => !off.includes(p))].join(",")];
}

export interface WindowsConfig {
  exeName?: string;
  icon?: string;
  company?: string;
  copyright?: string;
  description?: string;
  console?: boolean;
  host?: Variant;
  /** The parts to take — every other one is left out of the host. */
  features?: Part[];
  /** The parts to leave out of the host. */
  without?: Part[];
  /** The host variant all of the above select. */
  variant: Variant;
}

/** `"macos"`, defaults applied. Read on every platform, so a typo fails a Windows build too. */
export interface MacConfig {
  /** CFBundleIdentifier. */
  bundleId: string;
  bundleIdFrom: "macos.bundleId" | "id";
  /** The bundle's name (`<exeName>.app`) and its executable's (`Contents/MacOS/<exeName>`). */
  exeName: string;
  /** `macos.icon`, else `windows.icon`: an `.icns`, a square PNG, or an `.ico` holding PNG images. */
  iconPath: string | null;
  /** LSApplicationCategoryType, always `public.app-category.<…>`. */
  category: string | null;
  /** LSMinimumSystemVersion. */
  minimumSystemVersion: string;
  /** NSHumanReadableCopyright: `macos.copyright`, else `windows.copyright`. */
  copyright: string | null;
  microphoneUsageDescription: string | null;
  cameraUsageDescription: string | null;
  /**
   * A codesign identity; `"-"` signs ad hoc; `false` leaves the bundle as the
   * copy made it. `undefined`: ad hoc wherever `codesign` exists.
   */
  sign: string | false | undefined;
}

/** What vue-native's macOS host is built for (.cargo/config.toml `MACOSX_DEPLOYMENT_TARGET`). */
export const MACOS_MINIMUM = "13.0";

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
  /** `vn.config.json` `publicDir`, app-root-relative and `/`-separated; null when undeclared. */
  publicDir: string | null;
  windows: WindowsConfig;
  macos: MacConfig;
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
  // The disk HTTP cache's cap, folded into the window block like `devtools`
  // (docs/STORAGE.md §12).
  "httpCache",
  "assets",
  "publicDir",
  "define",
  "windows",
  "macos",
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
const WINDOWS_KEYS = new Set(["exeName", "icon", "company", "copyright", "description", "console", "host", "features", "without"]);
const MACOS_KEYS = new Set([
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
/** CFBundleIdentifier: a UTI — letters, digits, "-" and ".", reverse-DNS. */
const BUNDLE_ID = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/;
const CATEGORY = /^public\.app-category\.[a-z]+(-[a-z]+)*$/;

/**
 * The `"macos"` block. `fallback` is what it inherits when a key is absent:
 * the launch id for the bundle id, the Windows icon and copyright.
 */
function readMacos(
  raw: unknown,
  appRoot: string,
  fallback: { id: string; productName: string; name: string; icon: string | null; copyright?: string },
  warnings: string[],
): MacConfig {
  if (raw !== undefined && (!raw || typeof raw !== "object" || Array.isArray(raw))) {
    throw new ConfigError('vn.config.json "macos" must be an object');
  }
  const m = (raw ?? {}) as Record<string, unknown>;
  for (const key of Object.keys(m)) {
    if (key.startsWith("$")) continue;
    if (!MACOS_KEYS.has(key)) {
      throw new ConfigError(`vn.config.json "macos": unknown key ${JSON.stringify(key)} — known keys are ${[...MACOS_KEYS].join(", ")}`);
    }
  }
  const where = (k: string) => `vn.config.json macos.${k}`;

  const explicitId = str(m.bundleId, where("bundleId"));
  if (explicitId !== undefined && !BUNDLE_ID.test(explicitId)) {
    throw new ConfigError(
      `vn.config.json "macos.bundleId": ${JSON.stringify(explicitId)} is not a bundle identifier — reverse-DNS, ` +
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
        `vn.config.json "macos.category": ${JSON.stringify(cat)} is not an app category — e.g. "developer-tools" ` +
          'or "public.app-category.social-networking" (Apple\'s LSApplicationCategoryType list)',
      );
    }
  }

  const min = str(m.minimumSystemVersion, where("minimumSystemVersion")) ?? MACOS_MINIMUM;
  if (!/^\d+(\.\d+){0,2}$/.test(min)) {
    throw new ConfigError(`vn.config.json "macos.minimumSystemVersion": ${JSON.stringify(min)} is not a version like "13.0"`);
  }
  const [maj = 0, minor = 0] = min.split(".").map(Number);
  const [wantMaj = 0, wantMinor = 0] = MACOS_MINIMUM.split(".").map(Number);
  if (maj < wantMaj || (maj === wantMaj && minor < wantMinor)) {
    warnings.push(
      `vn.config.json "macos.minimumSystemVersion" ${min} is below ${MACOS_MINIMUM}, which the host is built for — ` +
        "an older macOS will offer to open the app and then fail to load it",
    );
  }

  let sign: MacConfig["sign"];
  if (m.sign !== undefined) {
    if (m.sign === false) sign = false;
    else if (typeof m.sign === "string" && m.sign.trim()) sign = m.sign.trim();
    else {
      throw new ConfigError('vn.config.json "macos.sign" must be a codesign identity, "-" (ad hoc) or false (leave unsigned)');
    }
  }

  let iconPath = fallback.icon;
  const icon = str(m.icon, where("icon"));
  if (icon) {
    iconPath = isAbsolute(icon) ? icon : join(appRoot, icon);
    if (!existsSync(iconPath)) throw new ConfigError(`vn.config.json "macos.icon": ${iconPath} does not exist`);
  }

  const exeName =
    sanitizeExeName(str(m.exeName, where("exeName")) ?? fallback.productName) || sanitizeExeName(fallback.name) || "app";
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

/**
 * A config override changes configuration only; entry and relative assets keep the target's app root.
 * `platform` (default `windows`) only picks which missing icon is worth a warning.
 */
export function loadAppConfig(target: string, options: { configPath?: string; platform?: Platform } = {}): AppConfig {
  const { appRoot, entry } = resolveApp(target);
  if (options.configPath !== undefined && !options.configPath.trim()) throw new ConfigError("--config needs a path");
  const configPath = options.configPath === undefined ? join(appRoot, "vn.config.json") : resolve(options.configPath);
  const has = existsSync(configPath);
  if (options.configPath !== undefined && !has) throw new ConfigError(`${configPath}: no such config file`);
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

  // Vite's `publicDir`: files served at the root, so `public/favicon.ico` is
  // `/favicon.ico`. Opt-in here (Vite defaults it to "public"): turning it on
  // by default would re-key a package that already ships a public/ directory
  // under "assets". An undeclared public/ next to vn.config.json is a warning
  // in collectAssets instead.
  let publicDir: string | null = null;
  if (raw.publicDir !== undefined && raw.publicDir !== false) {
    const p = typeof raw.publicDir === "string" ? raw.publicDir.trim().replace(/\\/g, "/").replace(/^\.\//, "").replace(/\/+$/, "") : "";
    if (!p || p.startsWith("/") || /^[a-zA-Z]:/.test(p) || p.split("/").includes("..")) {
      throw new ConfigError('vn.config.json "publicDir" must be a directory relative to the app root, inside it (or false)');
    }
    const abs = join(appRoot, p);
    if (!existsSync(abs) || !statSync(abs).isDirectory()) {
      warnings.push(`vn.config.json "publicDir": ${JSON.stringify(p)} is not a directory — nothing is packaged from it`);
    }
    publicDir = p;
  }
  if (raw.define !== undefined && (!raw.define || typeof raw.define !== "object" || Array.isArray(raw.define))) {
    throw new ConfigError('vn.config.json "define" must be an object of { "NAME": value }');
  }

  const windows: WindowsConfig = { variant: "standard" };
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
      if (typeof w.host !== "string") throw new ConfigError('vn.config.json "windows.host" must be a host variant, e.g. "no-devtools"');
      try {
        partsLeftOut(w.host);
      } catch (e) {
        throw new ConfigError(`vn.config.json "windows.host": ${(e as Error).message}`);
      }
      windows.host = w.host;
    }
    if (w.features !== undefined) windows.features = parseParts(w.features, where("windows.features"));
    if (w.without !== undefined) windows.without = parseParts(w.without, where("windows.without"));
    windows.variant = selectVariant(windows, where('"windows"'));
  }

  if (raw.httpCache !== undefined && raw.httpCache !== false) {
    const c = raw.httpCache as Record<string, unknown> | null;
    const mb = c && typeof c === "object" ? c.maxMB : undefined;
    if (typeof mb !== "number" || !(mb >= 0)) {
      throw new ConfigError('vn.config.json "httpCache" must be false or { "maxMB": <MB, 0 = no cache> }');
    }
  }

  const exeName = sanitizeExeName(windows.exeName ?? productName) || sanitizeExeName(name) || "app";
  let iconPath: string | null = null;
  if (windows.icon) {
    iconPath = isAbsolute(windows.icon) ? windows.icon : join(appRoot, windows.icon);
    if (!existsSync(iconPath)) throw new ConfigError(`vn.config.json "windows.icon": ${iconPath} does not exist`);
  }
  const macos = readMacos(
    raw.macos,
    appRoot,
    { id: launch.id, productName, name, icon: iconPath, copyright: windows.copyright },
    warnings,
  );
  if (options.platform === "macos") {
    if (!macos.iconPath) {
      warnings.push('no icon: set vn.config.json "macos.icon" (or "windows.icon") to a square .png or an .icns — the app gets the generic macOS icon');
    }
  } else if (!windows.icon) {
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
    publicDir,
    windows,
    macos,
    exeName,
    iconPath,
    id: launch.id,
    launch,
    warnings,
  };
}
