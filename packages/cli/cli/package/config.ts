// sk.config.json "package": parsed into a PackageConfig with every problem collected, so `sk check` can list
// them all and `sk package` can refuse with all of them at once.

import { existsSync } from "node:fs";
import { extname, isAbsolute, join, resolve } from "node:path";
import { BUNDLE_ID, RESERVED_SCHEMES, SCHEME, type AppConfig } from "../config";
import { PLATFORMS, type Platform } from "../platform";
import { numericVersion } from "../versioninfo";
import { iconProblem } from "./icons";
import { FORMATS, type FormatRegistry } from "./index";
import { format as msix } from "./msix";
import {
  PLATFORM_FORMATS,
  type FileAssociation,
  type FormatName,
  type FormatOptions,
  type OptionsContext,
  type PackageCheckContext,
  type PackageConfig,
  type PackageFormat,
  type PackageIdentity,
  type Problem,
  type UrlProtocol,
} from "./types";

export const PACKAGE_KEYS: ReadonlySet<string> = new Set([
  "displayName",
  "publisher",
  "id",
  "version",
  "description",
  "copyright",
  "icon",
  "fileAssociations",
  "protocols",
  "out",
  "windows",
  "macos",
  "linux",
]);
export const FILE_ASSOCIATION_KEYS: ReadonlySet<string> = new Set(["extensions", "name", "mimeType", "role", "icon"]);
export const PROTOCOL_KEYS: ReadonlySet<string> = new Set(["scheme", "name"]);
export const platformKeys = (platform: Platform): string[] => ["formats", ...PLATFORM_FORMATS[platform]];

const ROLES: readonly FileAssociation["role"][] = ["editor", "viewer", "none"];
const EXTENSION = /^[a-z0-9][a-z0-9_+-]{0,63}$/;
const MIME = /^[A-Za-z0-9][\w.+-]*\/[A-Za-z0-9][\w.+-]*$/;
/** Extensions Windows refuses to hand to a packaged app, and that no app should claim on any system. */
const FORBIDDEN_EXTENSIONS = new Set([
  "exe",
  "com",
  "bat",
  "cmd",
  "lnk",
  "msi",
  "msix",
  "appx",
  "scr",
  "pif",
  "ps1",
  "vbs",
  "reg",
  "url",
  "cpl",
  "dll",
  "sys",
  "inf",
]);

type Raw = Record<string, unknown>;

const isObject = (v: unknown): v is Raw => !!v && typeof v === "object" && !Array.isArray(v);

export interface ParsedPackage {
  config: PackageConfig;
  problems: Problem[];
}

class Collector {
  readonly problems: Problem[] = [];
  error(key: string, message: string): void {
    this.problems.push({ code: "SK8001", severity: "error", key, message });
  }
  warn(key: string, message: string): void {
    this.problems.push({ code: "SK8002", severity: "warning", key, message });
  }
  unknownKeys(raw: Raw, at: string, known: ReadonlySet<string> | readonly string[]): void {
    const list = [...known];
    for (const key of Object.keys(raw)) {
      if (key.startsWith("$") || list.includes(key)) continue;
      this.error(`${at}.${key}`, `"${at}": unknown key ${JSON.stringify(key)}; the known keys are ${list.join(", ")}`);
    }
  }
  string(raw: Raw, key: string, at: string): string | undefined {
    const v = raw[key];
    if (v === undefined) return undefined;
    if (typeof v !== "string" || !v.trim()) {
      this.error(`${at}.${key}`, `"${at}.${key}" must be a non-empty string`);
      return undefined;
    }
    return v.trim();
  }
}

/** `package` of sk.config.json with defaults from the rest of the config; an absent section gives the defaults. */
export function parsePackageConfig(app: AppConfig, formats: FormatRegistry = FORMATS): ParsedPackage {
  const c = new Collector();
  const at = "package";
  let raw: Raw = {};
  if (app.raw.package !== undefined) {
    if (isObject(app.raw.package)) raw = app.raw.package;
    else c.error(at, '"package" must be an object, e.g. { "publisher": "Example Ltd", "id": "com.example.notes" }');
  }
  c.unknownKeys(raw, at, PACKAGE_KEYS);
  const path = (p: string) => (isAbsolute(p) ? p : join(app.appRoot, p));

  const identity = readIdentity(raw, app, c);
  const icons = readIcons(raw, app, c, path);
  const fileAssociations = readFileAssociations(raw.fileAssociations, c, path);
  const protocols = readProtocols(raw.protocols, app, identity.displayName, c);
  const out = c.string(raw, "out", at);

  const section = (platform: Platform): { raw: Raw; formats: FormatName[] } => {
    const key = `${at}.${platform}`;
    const value = raw[platform];
    if (value !== undefined && !isObject(value)) {
      c.error(
        key,
        `"${key}" must be an object, e.g. { "formats": [${JSON.stringify(PLATFORM_FORMATS[platform][0])}] }`,
      );
    }
    const s = isObject(value) ? value : {};
    c.unknownKeys(s, key, platformKeys(platform));
    return { raw: s, formats: readFormats(s.formats, platform, key, formats, c) };
  };
  const options = <N extends FormatName>(
    platform: Platform,
    s: Raw,
    name: N,
    fallback: (ctx: OptionsContext, problems: Problem[]) => FormatOptions[N],
  ): FormatOptions[N] => {
    const key = `${at}.${platform}.${name}`;
    const value = s[name];
    if (value !== undefined && !isObject(value)) c.error(key, `"${key}" must be an object`);
    const optionsRaw = isObject(value) ? value : {};
    const f = formats[name];
    if (f) c.unknownKeys(optionsRaw, key, f.optionKeys);
    const ctx: OptionsContext = { app, identity, at: key };
    return f ? f.parseOptions(optionsRaw, ctx, c.problems) : fallback(ctx, c.problems);
  };

  const windows = section("windows");
  const macos = section("macos");
  const linux = section("linux");
  const config: PackageConfig = {
    identity,
    icons,
    fileAssociations,
    protocols,
    outDir: resolve(out === undefined ? join(app.appRoot, "dist", "package") : path(out)),
    windows: {
      formats: windows.formats,
      msix: options("windows", windows.raw, "msix", (ctx, problems) => msix.parseOptions({}, ctx, problems)),
    },
    macos: { formats: macos.formats, dmg: options("macos", macos.raw, "dmg", () => ({})) },
    linux: { formats: linux.formats, appimage: options("linux", linux.raw, "appimage", () => ({})) },
  };
  return { config, problems: c.problems };
}

function readIdentity(raw: Raw, app: AppConfig, c: Collector): PackageIdentity {
  const at = "package";
  const displayName = c.string(raw, "displayName", at) ?? app.productName;
  const publisher = c.string(raw, "publisher", at) ?? app.windows.company ?? "";

  const explicitId = c.string(raw, "id", at);
  const id =
    explicitId ?? (app.macos.bundleIdFrom === "macos.bundleId" ? app.macos.bundleId : app.id.replace(/_/g, "-"));
  if (!BUNDLE_ID.test(id)) {
    c.error(
      "package.id",
      explicitId
        ? `"package.id": ${JSON.stringify(id)} is not a reverse-DNS id; use letters, digits and "-" in two or more parts joined by ".", e.g. "com.example.notes"`
        : `the app's id ${JSON.stringify(id)} is not reverse-DNS; set "package.id" (or "id") to one, e.g. "com.example.${id.toLowerCase().replace(/[^a-z0-9-]/g, "") || "app"}"`,
    );
  }

  const given = c.string(raw, "version", at);
  const source = given ?? app.version;
  let version = "0.0.0";
  try {
    const [a, b, d, e] = numericVersion(source);
    version = e ? `${a}.${b}.${d}.${e}` : `${a}.${b}.${d}`;
    if (/[-+]/.test(source.replace(/^\s*v/, ""))) {
      c.warn(
        "package.version",
        `the version ${JSON.stringify(source)} is packaged as ${version}: installers compare numbers only, so a prerelease and its release would collide; set "package.version" to a higher number for each prerelease`,
      );
    }
  } catch (err) {
    c.error("package.version", `"package.version": ${(err as Error).message}`);
  }

  return {
    displayName,
    publisher,
    id,
    version,
    description: c.string(raw, "description", at) ?? app.windows.description ?? displayName,
    copyright:
      c.string(raw, "copyright", at) ?? app.windows.copyright ?? app.macos.copyright ?? `© ${publisher || displayName}`,
  };
}

function readIcons(raw: Raw, app: AppConfig, c: Collector, path: (p: string) => string): string[] {
  const value = raw.icon;
  if (value !== undefined) {
    const list = typeof value === "string" ? [value] : value;
    if (!Array.isArray(list) || !list.length || list.some((v) => typeof v !== "string" || !v.trim())) {
      c.error("package.icon", '"package.icon" must be a path or a list of paths: square .png files or an .ico');
      return [];
    }
    const icons: string[] = [];
    for (const p of list as string[]) {
      const abs = path(p.trim());
      const why = iconProblem(abs);
      if (why) c.error("package.icon", `"package.icon": ${abs} ${why}`);
      else icons.push(abs);
    }
    return icons;
  }
  const fallback = [...new Set([app.macos.iconPath, app.iconPath])].filter(
    (p): p is string => !!p && existsSync(p) && extname(p).toLowerCase() !== ".icns",
  );
  const icons = fallback.filter((p) => !iconProblem(p));
  if (!icons.length) {
    c.warn(
      "package.icon",
      'no icon for the packages: set "package.icon" to a square .png of 512px or more; without one the installer and Start menu show a blank tile',
    );
  }
  return icons;
}

function readFileAssociations(value: unknown, c: Collector, path: (p: string) => string): FileAssociation[] {
  const at = "package.fileAssociations";
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    c.error(at, `"${at}" must be an array of { "extensions": ["note"], "name": "Notes document" }`);
    return [];
  }
  const seen = new Set<string>();
  const out: FileAssociation[] = [];
  value.forEach((entry, i) => {
    const key = `${at}[${i}]`;
    if (!isObject(entry)) {
      c.error(key, `"${key}" must be an object with "extensions" and "name"`);
      return;
    }
    c.unknownKeys(entry, key, FILE_ASSOCIATION_KEYS);
    const extensions: string[] = [];
    const exts = entry.extensions;
    if (!Array.isArray(exts) || !exts.length) {
      c.error(`${key}.extensions`, `"${key}.extensions" must be a non-empty list of file extensions, e.g. ["note"]`);
    } else {
      for (const e of exts) {
        const ext = typeof e === "string" ? e.trim().replace(/^\./, "").toLowerCase() : "";
        if (!EXTENSION.test(ext)) {
          c.error(
            `${key}.extensions`,
            `"${key}.extensions": ${JSON.stringify(e)} is not a file extension; use letters, digits, "_", "+" or "-" without the dot`,
          );
        } else if (FORBIDDEN_EXTENSIONS.has(ext)) {
          c.error(
            `${key}.extensions`,
            `"${key}.extensions": ".${ext}" belongs to the system and cannot be claimed by an app`,
          );
        } else if (seen.has(ext)) {
          c.error(`${key}.extensions`, `"${key}.extensions": ".${ext}" is listed twice; keep it in one association`);
        } else {
          seen.add(ext);
          extensions.push(ext);
        }
      }
    }
    const name = c.string(entry, "name", key);
    if (name === undefined && entry.name === undefined) {
      c.error(`${key}.name`, `"${key}.name" is missing: the file type's name as users see it, e.g. "Notes document"`);
    }
    const mimeType = c.string(entry, "mimeType", key) ?? null;
    if (mimeType !== null && !MIME.test(mimeType)) {
      c.error(
        `${key}.mimeType`,
        `"${key}.mimeType": ${JSON.stringify(mimeType)} is not a MIME type like "application/x-notes"`,
      );
    }
    let role: FileAssociation["role"] = "editor";
    if (entry.role !== undefined) {
      if (ROLES.includes(entry.role as FileAssociation["role"])) role = entry.role as FileAssociation["role"];
      else c.error(`${key}.role`, `"${key}.role" must be ${ROLES.map((r) => JSON.stringify(r)).join(", ")}`);
    }
    let icon: string | null = null;
    const iconRaw = c.string(entry, "icon", key);
    if (iconRaw !== undefined) {
      const abs = path(iconRaw);
      const why = iconProblem(abs);
      if (why) c.error(`${key}.icon`, `"${key}.icon": ${abs} ${why}`);
      else icon = abs;
    }
    out.push({ extensions, name: name ?? "", mimeType, role, icon });
  });
  return out;
}

function readProtocols(value: unknown, app: AppConfig, displayName: string, c: Collector): UrlProtocol[] {
  const at = "package.protocols";
  if (value === undefined) return app.launch.protocols.map((scheme) => ({ scheme, name: displayName }));
  if (!Array.isArray(value)) {
    c.error(at, `"${at}" must be an array of schemes ("notes") or { "scheme": "notes", "name": "Notes link" }`);
    return [];
  }
  const out: UrlProtocol[] = [];
  value.forEach((entry, i) => {
    const key = `${at}[${i}]`;
    let scheme: unknown = entry;
    let name = displayName;
    if (isObject(entry)) {
      c.unknownKeys(entry, key, PROTOCOL_KEYS);
      scheme = entry.scheme;
      name = c.string(entry, "name", key) ?? displayName;
    }
    const s = typeof scheme === "string" ? scheme.trim().toLowerCase().replace(/:\/*$/, "") : "";
    if (!SCHEME.test(s)) {
      c.error(
        key,
        `"${key}": ${JSON.stringify(scheme)} is not a URL scheme (a letter, then letters, digits, "+", "-" or ".")`,
      );
      return;
    }
    if (RESERVED_SCHEMES.has(s) || s.startsWith("ms-")) {
      c.error(key, `"${key}": "${s}" belongs to the browser or to Windows and cannot be registered by an app`);
      return;
    }
    if (out.some((p) => p.scheme === s)) {
      c.error(key, `"${key}": "${s}" is listed twice`);
      return;
    }
    if (!app.launch.protocols.includes(s)) {
      c.warn(
        key,
        `"${key}": "${s}" is not in the top-level "protocols", so only the installed package handles it; add it there too so a build run without installing registers it as well`,
      );
    }
    out.push({ scheme: s, name });
  });
  return out;
}

function readFormats(
  value: unknown,
  platform: Platform,
  at: string,
  registry: FormatRegistry,
  c: Collector,
): FormatName[] {
  const allowed = PLATFORM_FORMATS[platform];
  const available = allowed.filter((f) => registry[f]);
  if (value === undefined) return available;
  const key = `${at}.formats`;
  if (!Array.isArray(value)) {
    c.error(key, `"${key}" must be a list, e.g. [${allowed.map((f) => JSON.stringify(f)).join(", ")}]`);
    return [];
  }
  const out: FormatName[] = [];
  for (const v of value) {
    const name = allowed.find((f) => f === v);
    if (!name) {
      const elsewhere = PLATFORMS.find((p) => PLATFORM_FORMATS[p].some((f) => f === v));
      c.error(
        key,
        elsewhere
          ? `"${key}": ${JSON.stringify(v)} is a ${elsewhere} format; list it under "package.${elsewhere}.formats"`
          : `"${key}": ${JSON.stringify(v)} is not a format; ${platform} has ${allowed.map((f) => JSON.stringify(f)).join(", ")}`,
      );
    } else if (!registry[name]) {
      c.error(key, `"${key}": ${JSON.stringify(name)} is not available in this version of sk`);
    } else if (!out.includes(name)) out.push(name);
  }
  return out;
}

/** Every problem of the config and of the formats it lists for `platforms` (or `only`); `sk check` reports the same list. */
export function packageProblems(
  app: AppConfig,
  platforms: readonly Platform[],
  only?: readonly FormatName[],
): { config: PackageConfig; problems: Problem[]; formats: PackageFormat[] } {
  const { config, problems } = parsePackageConfig(app);
  const formats: PackageFormat[] = [];
  for (const platform of platforms) {
    for (const name of only ?? config[platform].formats) {
      const format = FORMATS[name];
      if (!format || format.platform !== platform) continue;
      formats.push(format);
      const ctx: PackageCheckContext = { appRoot: app.appRoot, app, config, platform };
      problems.push(...format.validate(ctx));
    }
  }
  return { config, problems, formats };
}
