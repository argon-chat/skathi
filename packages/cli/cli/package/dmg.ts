// macOS disk image: the signed `.app`, an Applications link, a Finder layout and optionally notarisation.
//
// The layout is a `.DS_Store` written here rather than by scripting Finder, so it works over ssh and in CI
// without a window server. Its background is referenced by an alias record, which holds the file ids the
// image's own HFS+ volume gave it: the store is written into the mounted read-write image, then converted.

import {
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, extname, isAbsolute, join } from "node:path";
import { isPng } from "../icon";
import { AD_HOC, ICON_NAME } from "../macos-bundle";
import type {
  Artifact,
  DmgImageFormat,
  DmgNotarize,
  DmgOptions,
  DmgPoint,
  DmgWindow,
  OptionsContext,
  PackageCheckContext,
  PackageContext,
  PackageFormat,
  Problem,
} from "./types";

export type ResolvedDmg = Required<DmgOptions>;

const FORMATS: readonly DmgImageFormat[] = ["ULFO", "ULMO", "UDZO", "UDBZ"];
const KEYS = [
  "volumeName",
  "fileName",
  "format",
  "window",
  "iconSize",
  "textSize",
  "appPosition",
  "applicationsPosition",
  "background",
  "backgroundColor",
  "volumeIcon",
  "sign",
  "notarize",
] as const;

export const DEFAULT_WINDOW: DmgWindow = { x: 200, y: 120, width: 660, height: 400 };
export const DEFAULT_APP_POSITION: DmgPoint = { x: 180, y: 170 };
export const DEFAULT_APPLICATIONS_POSITION: DmgPoint = { x: 480, y: 170 };

const NOT_ON_MAC =
  "a .dmg is made with macOS's own tools (hdiutil, codesign, notarytool): run sk package for macOS on a Mac";
const ENV_REF = /^env:([A-Za-z_][A-Za-z0-9_]*)$/;
const COLOR = /^#[0-9a-fA-F]{6}$/;
const FILE_NAME_UNSAFE = /[/\\:\x00-\x1f]/;

function isObject(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === "object" && !Array.isArray(v);
}

/** Defaults for whatever is absent; `name` is the display name and `version` the package version. */
export function resolveDmg(o: DmgOptions, name: string, version: string): ResolvedDmg {
  return {
    volumeName: o.volumeName ?? name,
    fileName: o.fileName ?? `${name}-${version}-{arch}`.replace(new RegExp(FILE_NAME_UNSAFE, "g"), "-"),
    format: o.format ?? "ULFO",
    window: o.window ?? DEFAULT_WINDOW,
    iconSize: o.iconSize ?? 128,
    textSize: o.textSize ?? 13,
    appPosition: o.appPosition ?? DEFAULT_APP_POSITION,
    applicationsPosition: o.applicationsPosition ?? DEFAULT_APPLICATIONS_POSITION,
    background: o.background ?? null,
    backgroundColor: o.backgroundColor ?? null,
    volumeIcon: o.volumeIcon ?? true,
    sign: o.sign ?? null,
    notarize: o.notarize ?? null,
  };
}

function parseOptions(raw: Record<string, unknown>, ctx: OptionsContext, problems: Problem[]): DmgOptions {
  const out: DmgOptions = {};
  const bad = (key: string, message: string) =>
    problems.push({ code: "SK8202", severity: "error", key: `${ctx.at}.${key}`, message });
  const text = (key: string): string | undefined => {
    const v = raw[key];
    if (v === undefined) return undefined;
    if (typeof v === "string" && v.trim()) return v.trim();
    bad(key, `"${key}" must be a non-empty string`);
    return undefined;
  };
  const num = (key: string, v: unknown, min: number, max: number): number | undefined => {
    if (typeof v === "number" && Number.isInteger(v) && v >= min && v <= max) return v;
    bad(key, `"${key}" must be a whole number from ${min} to ${max}`);
    return undefined;
  };
  const point = (key: string): DmgPoint | undefined => {
    const v = raw[key];
    if (v === undefined) return undefined;
    if (!isObject(v)) {
      bad(key, `"${key}" must be { "x": …, "y": … } in points from the window's top-left corner`);
      return undefined;
    }
    const x = num(`${key}.x`, v.x, 0, 10000);
    const y = num(`${key}.y`, v.y, 0, 10000);
    return x === undefined || y === undefined ? undefined : { x, y };
  };

  const volumeName = text("volumeName");
  if (volumeName !== undefined) {
    if (volumeName.includes(":") || volumeName.includes("/"))
      bad("volumeName", '"volumeName" cannot contain ":" or "/"');
    else out.volumeName = volumeName;
  }
  const fileName = text("fileName");
  if (fileName !== undefined) {
    if (FILE_NAME_UNSAFE.test(fileName) || fileName.toLowerCase().endsWith(".dmg")) {
      bad("fileName", '"fileName" is a file name without ".dmg" and without "/", "\\" or ":"');
    } else out.fileName = fileName;
  }
  if (raw.format !== undefined) {
    if (FORMATS.includes(raw.format as DmgImageFormat)) out.format = raw.format as DmgImageFormat;
    else bad("format", `"format" must be one of ${FORMATS.join(", ")}`);
  }
  if (raw.window !== undefined) {
    const w = raw.window;
    if (!isObject(w)) bad("window", '"window" must be { "width", "height", "x"?, "y"? } in points');
    else {
      const width = num("window.width", w.width ?? DEFAULT_WINDOW.width, 200, 4000);
      const height = num("window.height", w.height ?? DEFAULT_WINDOW.height, 150, 4000);
      const x = num("window.x", w.x ?? DEFAULT_WINDOW.x, 0, 10000);
      const y = num("window.y", w.y ?? DEFAULT_WINDOW.y, 0, 10000);
      if (width !== undefined && height !== undefined && x !== undefined && y !== undefined) {
        out.window = { x, y, width, height };
      }
    }
  }
  if (raw.iconSize !== undefined) {
    const v = num("iconSize", raw.iconSize, 16, 512);
    if (v !== undefined) out.iconSize = v;
  }
  if (raw.textSize !== undefined) {
    const v = num("textSize", raw.textSize, 10, 16);
    if (v !== undefined) out.textSize = v;
  }
  const appPosition = point("appPosition");
  if (appPosition) out.appPosition = appPosition;
  const applicationsPosition = point("applicationsPosition");
  if (applicationsPosition) out.applicationsPosition = applicationsPosition;
  const background = text("background");
  if (background !== undefined)
    out.background = isAbsolute(background) ? background : join(ctx.app.appRoot, background);
  const color = text("backgroundColor");
  if (color !== undefined) {
    if (COLOR.test(color)) out.backgroundColor = color.toLowerCase();
    else bad("backgroundColor", '"backgroundColor" must be "#rrggbb"');
  }
  if (raw.volumeIcon !== undefined) {
    if (typeof raw.volumeIcon === "boolean") out.volumeIcon = raw.volumeIcon;
    else bad("volumeIcon", '"volumeIcon" must be true or false');
  }
  if (raw.sign !== undefined) {
    if (raw.sign === false) out.sign = false;
    else if (typeof raw.sign === "string" && raw.sign.trim() && raw.sign.trim() !== AD_HOC) out.sign = raw.sign.trim();
    else bad("sign", '"sign" must be a codesign identity (a Developer ID Application certificate) or false');
  }
  if (raw.notarize !== undefined && raw.notarize !== false) {
    const n = parseNotarize(raw.notarize);
    if (typeof n === "string") bad("notarize", n);
    else out.notarize = n;
  }
  return out;
}

function parseNotarize(v: unknown): DmgNotarize | string {
  const shape =
    '"notarize" must be { "keychainProfile": "<name>" } or { "apiKey": "env:<VAR>", "apiKeyId": …, "apiIssuer": … }';
  if (!isObject(v)) return shape;
  const s = (k: string) => (typeof v[k] === "string" && (v[k] as string).trim() ? (v[k] as string).trim() : null);
  const known = ["keychainProfile", "keychain", "apiKey", "apiKeyId", "apiIssuer"];
  const unknown = Object.keys(v).filter((k) => !k.startsWith("$") && !known.includes(k));
  if (unknown.length)
    return `"notarize": unknown key ${JSON.stringify(unknown[0])} — known keys are ${known.join(", ")}`;
  const profile = s("keychainProfile");
  const apiKey = s("apiKey");
  if (profile && v.apiKey !== undefined) return '"notarize" takes "keychainProfile" or "apiKey", not both';
  if (profile) return { keychainProfile: profile, keychain: s("keychain") };
  const apiKeyId = s("apiKeyId");
  const apiIssuer = s("apiIssuer");
  if (apiKey && apiKeyId && apiIssuer) return { apiKey, apiKeyId, apiIssuer };
  return shape;
}

/** A setting that must come from the environment: `"env:NAME"`. */
export function envName(value: string): string | null {
  return ENV_REF.exec(value)?.[1] ?? null;
}

function signIdentity(ctx: PackageCheckContext, dmg: ResolvedDmg): string | null {
  if (dmg.sign === false) return null;
  if (typeof dmg.sign === "string") return dmg.sign;
  const app = ctx.app.macos.sign;
  return typeof app === "string" && app !== AD_HOC ? app : null;
}

function resolved(ctx: PackageCheckContext): ResolvedDmg {
  return resolveDmg(ctx.config.macos.dmg, ctx.config.identity.displayName, ctx.config.identity.version);
}

export function validateDmg(ctx: PackageCheckContext, hostPlatform: string = process.platform): Problem[] {
  const problems: Problem[] = [];
  const at = "package.macos.dmg";
  const err = (code: string, key: string, message: string) => problems.push({ code, severity: "error", key, message });
  const warn = (code: string, key: string, message: string) =>
    problems.push({ code, severity: "warning", key, message });
  const dmg = resolved(ctx);

  // A warning, so `sk check` on another system still passes for an app that also ships for macOS; write refuses.
  if (hostPlatform !== "darwin") warn("SK8201", "package.macos.formats", NOT_ON_MAC);
  if (dmg.background) {
    if (!existsSync(dmg.background)) err("SK8203", `${at}.background`, `${dmg.background} does not exist`);
    else if (!isPng(readFileSync(dmg.background).subarray(0, 8)) && !isTiff(dmg.background)) {
      err("SK8203", `${at}.background`, `${dmg.background}: the background must be a PNG or a TIFF`);
    }
  }
  if (dmg.background && dmg.backgroundColor) {
    warn("SK8208", `${at}.backgroundColor`, '"backgroundColor" is ignored when "background" is set');
  }
  const { width, height } = dmg.window;
  for (const [key, p] of [
    ["appPosition", dmg.appPosition],
    ["applicationsPosition", dmg.applicationsPosition],
  ] as const) {
    if (p.x > width || p.y > height) {
      warn("SK8208", `${at}.${key}`, `(${p.x}, ${p.y}) is outside the ${width}×${height} window — the icon is hidden`);
    }
  }
  const n = dmg.notarize;
  if (n) {
    if ("apiKey" in n) {
      for (const k of ["apiKey", "apiKeyId", "apiIssuer"] as const) {
        const value = n[k];
        if (envName(value) || (k !== "apiKey" && !value.includes("BEGIN"))) continue;
        err(
          "SK8204",
          `${at}.notarize.${k}`,
          `"${k}" must name an environment variable as "env:NAME" — a key in sk.config.json ends up in the repository. ` +
            "Put the .p8 file's path or contents in that variable, or use a keychain profile",
        );
      }
    }
    const identity = typeof ctx.app.macos.sign === "string" && ctx.app.macos.sign !== AD_HOC;
    if (!identity) {
      err(
        "SK8205",
        `${at}.notarize`,
        'notarisation needs the app signed with a Developer ID Application identity: set "macos.sign" to it ' +
          "(ad hoc and unsigned apps are rejected by the notary service)",
      );
    }
  }
  return problems;
}

function isTiff(path: string): boolean {
  const b = readFileSync(path).subarray(0, 4);
  return (
    (b[0] === 0x49 && b[1] === 0x49 && b[2] === 42 && b[3] === 0) ||
    (b[0] === 0x4d && b[1] === 0x4d && b[2] === 0 && b[3] === 42)
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Binary property lists (bplist00): what Finder keeps in the store's `bwsp` and `icvp` blobs.

export type BplistValue = string | number | boolean | Uint8Array | { real: number } | { [key: string]: BplistValue };

function isReal(v: BplistValue): v is { real: number } {
  return isObject(v) && Object.keys(v).length === 1 && typeof v.real === "number";
}

export function bplist(root: BplistValue): Uint8Array {
  const objects: Uint8Array[] = [];
  const parts: number[][] = [];
  const flatten = (v: BplistValue): number => {
    const index = objects.length;
    objects.push(new Uint8Array(0));
    parts.push([]);
    if (typeof v === "boolean") objects[index] = Uint8Array.of(v ? 0x09 : 0x08);
    else if (typeof v === "number") {
      if (!Number.isInteger(v)) objects[index] = real(v);
      else objects[index] = int(v, 0x10);
    } else if (typeof v === "string") objects[index] = str(v);
    else if (v instanceof Uint8Array) objects[index] = concat([marker(0x40, v.length), v]);
    else if (isReal(v)) objects[index] = real(v.real);
    else {
      const keys = Object.keys(v);
      const refs = [...keys.map((k) => flatten(k)), ...keys.map((k) => flatten(v[k]!))];
      parts[index] = refs;
    }
    return index;
  };
  flatten(root);
  const refSize = objects.length < 0x100 ? 1 : 2;
  const encoded = objects.map((o, i) => {
    const refs = parts[i]!;
    if (!o.length) {
      const body = new Uint8Array(refs.length * refSize);
      refs.forEach((r, j) => (refSize === 1 ? (body[j] = r) : new DataView(body.buffer).setUint16(j * 2, r)));
      return concat([marker(0xd0, refs.length / 2), body]);
    }
    return o;
  });
  const header = new TextEncoder().encode("bplist00");
  const offsets: number[] = [];
  let at = header.length;
  for (const e of encoded) {
    offsets.push(at);
    at += e.length;
  }
  const offsetSize = at < 0x100 ? 1 : at < 0x10000 ? 2 : 4;
  const table = new Uint8Array(offsets.length * offsetSize);
  const tv = new DataView(table.buffer);
  offsets.forEach((o, i) => {
    if (offsetSize === 1) table[i] = o;
    else if (offsetSize === 2) tv.setUint16(i * 2, o);
    else tv.setUint32(i * 4, o);
  });
  const trailer = new Uint8Array(32);
  const dv = new DataView(trailer.buffer);
  trailer[6] = offsetSize;
  trailer[7] = refSize;
  dv.setBigUint64(8, BigInt(objects.length));
  dv.setBigUint64(16, 0n);
  dv.setBigUint64(24, BigInt(at));
  return concat([header, ...encoded, table, trailer]);

  function marker(type: number, count: number): Uint8Array {
    return count < 15 ? Uint8Array.of(type | count) : concat([Uint8Array.of(type | 0xf), int(count, 0x10)]);
  }
  function int(n: number, type: number): Uint8Array {
    if (n >= 0 && n < 0x100) return Uint8Array.of(type, n);
    if (n >= 0 && n < 0x10000) return Uint8Array.of(type | 1, n >> 8, n & 0xff);
    const b = new Uint8Array(9);
    b[0] = type | 3;
    new DataView(b.buffer).setBigInt64(1, BigInt(n));
    return b;
  }
  function real(n: number): Uint8Array {
    const b = new Uint8Array(9);
    b[0] = 0x23;
    new DataView(b.buffer).setFloat64(1, n);
    return b;
  }
  function str(s: string): Uint8Array {
    if (/^[\x00-\x7f]*$/.test(s)) return concat([marker(0x50, s.length), new TextEncoder().encode(s)]);
    return concat([marker(0x60, s.length), utf16be(s)]);
  }
}

function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

function utf16be(s: string): Uint8Array {
  const b = new Uint8Array(s.length * 2);
  const dv = new DataView(b.buffer);
  for (let i = 0; i < s.length; i++) dv.setUint16(i * 2, s.charCodeAt(i));
  return b;
}

// ---------------------------------------------------------------------------------------------------------------
// `.DS_Store`: a buddy-allocated file holding one B-tree ("DSDB") of (file name, code, typed value) records.
// Block addresses are relative to byte 4 and carry log2(size) in their low five bits.

export type DsValue =
  | { type: "blob"; value: Uint8Array }
  | { type: "long"; value: number }
  | { type: "bool"; value: boolean }
  | { type: "type"; value: string };

export interface DsRecord {
  name: string;
  code: string;
  value: DsValue;
}

const fourCC = (s: string) => {
  if (!/^[\x20-\x7e]{4}$/.test(s)) throw new Error(`${JSON.stringify(s)} is not a four-character code`);
  return new TextEncoder().encode(s);
};

function u32(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n >>> 0);
  return b;
}

function encodeRecord(r: DsRecord): Uint8Array {
  const v = r.value;
  const value =
    v.type === "blob"
      ? concat([u32(v.value.length), v.value])
      : v.type === "long"
        ? u32(v.value)
        : v.type === "bool"
          ? Uint8Array.of(v.value ? 1 : 0)
          : fourCC(v.value);
  return concat([u32(r.name.length), utf16be(r.name), fourCC(r.code), fourCC(v.type), value]);
}

const PAGE = 0x1000;

export function dsStore(records: DsRecord[]): Uint8Array {
  const sorted = [...records].sort((a, b) => {
    const l = a.name.toLowerCase();
    const r = b.name.toLowerCase();
    return l < r ? -1 : l > r ? 1 : a.code < b.code ? -1 : a.code > b.code ? 1 : 0;
  });
  const leaf = concat([u32(0), u32(sorted.length), ...sorted.map(encodeRecord)]);
  if (leaf.length > PAGE) throw new Error(".DS_Store: the layout does not fit one B-tree page");

  const free: number[][] = Array.from({ length: 32 }, () => []);
  free[31] = [0];
  const alloc = (width: number): number => {
    let w = width;
    while (w < 32 && !free[w]!.length) w++;
    if (w === 32) throw new Error(".DS_Store: out of space");
    const off = free[w]!.shift()!;
    while (w > width) {
      w--;
      free[w]!.push(off + 2 ** w);
      free[w]!.sort((a, b) => a - b);
    }
    return off;
  };
  alloc(5); // the file header lives at address 0
  const rootWidth = 11;
  const root = alloc(rootWidth);
  const dsdb = alloc(5);
  const node = alloc(12);

  const offsets = [root | rootWidth, dsdb | 5, node | 12];
  const offsetTable = new Uint8Array(4 * 256);
  offsets.forEach((o, i) => offsetTable.set(u32(o), i * 4));
  const name = new TextEncoder().encode("DSDB");
  const rootBlock = concat([
    u32(offsets.length),
    u32(0),
    offsetTable,
    u32(1),
    Uint8Array.of(name.length),
    name,
    u32(1),
    ...free.map((list) => concat([u32(list.length), ...list.map(u32)])),
  ]);
  if (rootBlock.length > 2 ** rootWidth) throw new Error(".DS_Store: root block overflow");
  const dsdbBlock = concat([u32(2), u32(0), u32(sorted.length), u32(1), u32(PAGE)]);

  const file = new Uint8Array(4 + node + PAGE);
  file.set(u32(1), 0);
  file.set(new TextEncoder().encode("Bud1"), 4);
  file.set(u32(root), 8);
  file.set(u32(2 ** rootWidth), 12);
  file.set(u32(root), 16);
  file.set(Uint8Array.of(0, 0, 0x10, 0x0c, 0, 0, 0, 0x87, 0, 0, 0x20, 0x0b, 0, 0, 0, 0), 20);
  file.set(rootBlock, 4 + root);
  file.set(dsdbBlock, 4 + dsdb);
  file.set(leaf, 4 + node);
  return file;
}

// ---------------------------------------------------------------------------------------------------------------
// Alias record (version 2), which `icvp.backgroundImageAlias` holds.

export interface AliasTarget {
  volumeName: string;
  /** The volume's creation date; HFS+ stamps its root folder with it. */
  volumeCreated: Date;
  /** Path inside the volume, e.g. `/.background/background.png`. */
  path: string;
  cnid: number;
  parentCnid: number;
  created: Date;
}

const HFS_EPOCH = Date.UTC(1904, 0, 1) / 1000;
const hfsDate = (d: Date) => Math.max(0, Math.floor(d.getTime() / 1000 - HFS_EPOCH)) >>> 0;

function pascal(s: string, size: number): Uint8Array {
  const out = new Uint8Array(size);
  const bytes = new TextEncoder().encode(s).subarray(0, size - 1);
  out[0] = bytes.length;
  out.set(bytes, 1);
  return out;
}

export function aliasRecord(t: AliasTarget): Uint8Array {
  const segments = t.path.split("/").filter(Boolean);
  const fileName = segments.at(-1) ?? "";
  const folderName = segments.at(-2) ?? t.volumeName;
  const fixed = new Uint8Array(142);
  const dv = new DataView(fixed.buffer);
  dv.setInt16(0, 0); // a file
  fixed.set(pascal(t.volumeName, 28), 2);
  dv.setUint32(30, hfsDate(t.volumeCreated));
  fixed.set(new TextEncoder().encode("H+"), 34);
  dv.setInt16(36, 5); // ejectable disk
  dv.setUint32(38, t.parentCnid);
  fixed.set(pascal(fileName.replace(/:/g, "/"), 64), 42);
  dv.setUint32(106, t.cnid);
  dv.setUint32(110, hfsDate(t.created));
  dv.setInt16(122, -1);
  dv.setInt16(124, -1);

  const tag = (id: number, data: Uint8Array) => {
    const head = new Uint8Array(4);
    new DataView(head.buffer).setInt16(0, id);
    new DataView(head.buffer).setUint16(2, data.length);
    return concat([head, data, data.length % 2 ? Uint8Array.of(0) : new Uint8Array(0)]);
  };
  const unicode = (s: string) => concat([Uint8Array.of(s.length >> 8, s.length & 0xff), utf16be(s)]);
  const enc = (s: string) => new TextEncoder().encode(s);
  const tags = concat([
    tag(0, enc(folderName)),
    tag(1, u32(t.parentCnid)),
    tag(2, enc([t.volumeName, ...segments].join(":"))),
    tag(14, unicode(fileName)),
    tag(15, unicode(t.volumeName)),
    tag(18, enc(`/${segments.join("/")}`)),
    tag(19, enc(`/Volumes/${t.volumeName}`)),
    Uint8Array.of(0xff, 0xff, 0, 0),
  ]);
  const head = new Uint8Array(8);
  const hv = new DataView(head.buffer);
  hv.setUint16(4, 8 + fixed.length + tags.length);
  hv.setUint16(6, 2);
  return concat([head, fixed, tags]);
}

// ---------------------------------------------------------------------------------------------------------------
// The layout records.

export function layoutRecords(dmg: ResolvedDmg, appName: string, background: { alias: Uint8Array } | null): DsRecord[] {
  const w = dmg.window;
  const blob = (value: BplistValue): DsValue => ({ type: "blob", value: bplist(value) });
  const rgb = dmg.backgroundColor
    ? [1, 3, 5].map((i) => parseInt(dmg.backgroundColor!.slice(i, i + 2), 16) / 255)
    : null;
  const icvp: Record<string, BplistValue> = {
    viewOptionsVersion: 1,
    backgroundType: background ? 2 : rgb ? 1 : 0,
    backgroundColorRed: { real: rgb?.[0] ?? 1 },
    backgroundColorGreen: { real: rgb?.[1] ?? 1 },
    backgroundColorBlue: { real: rgb?.[2] ?? 1 },
    gridOffsetX: { real: 0 },
    gridOffsetY: { real: 0 },
    gridSpacing: { real: 100 },
    arrangeBy: "none",
    showIconPreview: true,
    showItemInfo: false,
    labelOnBottom: true,
    textSize: { real: dmg.textSize },
    iconSize: { real: dmg.iconSize },
    scrollPositionX: { real: 0 },
    scrollPositionY: { real: 0 },
  };
  if (background) icvp.backgroundImageAlias = background.alias;
  const iloc = (p: DmgPoint): DsValue => {
    const b = new Uint8Array(16).fill(0xff);
    const dv = new DataView(b.buffer);
    dv.setUint32(0, p.x);
    dv.setUint32(4, p.y);
    b[14] = 0;
    b[15] = 0;
    return { type: "blob", value: b };
  };
  return [
    {
      name: ".",
      code: "bwsp",
      value: blob({
        ContainerShowSidebar: false,
        PreviewPaneVisibility: false,
        ShowPathbar: false,
        ShowSidebar: false,
        ShowStatusBar: false,
        ShowTabView: false,
        ShowToolbar: false,
        SidebarWidth: 0,
        WindowBounds: `{{${w.x}, ${w.y}}, {${w.width}, ${w.height}}}`,
      }),
    },
    { name: ".", code: "icvp", value: blob(icvp) },
    { name: ".", code: "vSrn", value: { type: "long", value: 1 } },
    { name: ".", code: "vstl", value: { type: "type", value: "icnv" } },
    { name: appName, code: "Iloc", value: iloc(dmg.appPosition) },
    { name: "Applications", code: "Iloc", value: iloc(dmg.applicationsPosition) },
  ];
}

// ---------------------------------------------------------------------------------------------------------------
// Commands.

export type NotaryCredentials =
  { keychainProfile: string; keychain: string | null } | { keyPath: string; keyId: string; issuer: string };

export function notaryAuthArgs(c: NotaryCredentials): string[] {
  if ("keychainProfile" in c) {
    return ["--keychain-profile", c.keychainProfile, ...(c.keychain ? ["--keychain", c.keychain] : [])];
  }
  return ["--key", c.keyPath, "--key-id", c.keyId, "--issuer", c.issuer];
}

export function notarySubmitArgs(path: string, c: NotaryCredentials): string[] {
  return ["notarytool", "submit", path, ...notaryAuthArgs(c), "--wait", "--output-format", "json"];
}

/**
 * Credentials from the config and the environment. An API key's variable holds the `.p8` path or its PEM text;
 * text is written to `keyFile` (0600) for notarytool, which takes only a path.
 */
export function notaryCredentials(
  n: DmgNotarize,
  env: Record<string, string | undefined>,
  keyFile: string,
): NotaryCredentials {
  if ("keychainProfile" in n) return { keychainProfile: n.keychainProfile, keychain: n.keychain };
  const read = (key: "apiKey" | "apiKeyId" | "apiIssuer", literalOk: boolean): string => {
    const value = n[key];
    const name = envName(value);
    if (!name) {
      if (literalOk && !value.includes("BEGIN")) return value;
      throw new Error(`package.macos.dmg.notarize.${key} must be "env:NAME", not the value itself`);
    }
    const v = env[name]?.trim();
    if (!v)
      throw new Error(`notarisation: the environment variable ${name} (package.macos.dmg.notarize.${key}) is not set`);
    return v;
  };
  const key = read("apiKey", false);
  let keyPath = key;
  if (key.includes("-----BEGIN")) {
    writeFileSync(keyFile, key.endsWith("\n") ? key : `${key}\n`, { mode: 0o600 });
    keyPath = keyFile;
  } else if (!existsSync(key)) {
    throw new Error(
      `notarisation: ${n.apiKey.slice(4)} names ${key}, which does not exist — give the .p8 path or its contents`,
    );
  }
  return { keyPath, keyId: read("apiKeyId", true), issuer: read("apiIssuer", true) };
}

export interface NotaryResult {
  id: string;
  status: string;
  message: string;
}

export function parseNotaryResult(stdout: string): NotaryResult {
  let v: unknown;
  try {
    v = JSON.parse(stdout);
  } catch {
    throw new Error(`notarytool printed something other than JSON:\n${stdout.trim()}`);
  }
  if (!isObject(v) || typeof v.id !== "string" || typeof v.status !== "string") {
    throw new Error(`notarytool's answer has no submission id or status:\n${stdout.trim()}`);
  }
  return { id: v.id, status: v.status, message: typeof v.message === "string" ? v.message : "" };
}

export function hdiutilCreateArgs(image: string, volumeName: string, megabytes: number): string[] {
  return ["create", "-ov", "-size", `${megabytes}m`, "-fs", "HFS+", "-volname", volumeName, "-type", "UDIF", image];
}

export function hdiutilConvertArgs(from: string, to: string, format: DmgImageFormat): string[] {
  return [
    "convert",
    from,
    "-format",
    format,
    ...(format === "UDZO" ? ["-imagekey", "zlib-level=9"] : []),
    "-ov",
    "-o",
    to,
  ];
}

export function mountPointFrom(plist: string): string {
  const m = /<key>mount-point<\/key>\s*<string>([^<]+)<\/string>/.exec(plist);
  if (!m) throw new Error(`hdiutil attach mounted nothing:\n${plist.trim()}`);
  return m[1]!.replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">");
}

/** Bytes an HFS+ copy of `path` takes, in 4 KiB blocks plus one per entry for its catalog record. */
export function diskUsage(path: string): number {
  const st = lstatSync(path);
  const own = 4096 + Math.ceil(st.size / 4096) * 4096;
  if (!st.isDirectory()) return own;
  return readdirSync(path).reduce((n, f) => n + diskUsage(join(path, f)), own);
}

async function exec(cmd: string, args: string[]): Promise<{ code: number; out: string; err: string }> {
  // Every tool used is in /usr/bin; a Homebrew coreutils or a shim earlier on PATH must not stand in for it.
  const p = Bun.spawn([`/usr/bin/${cmd}`, ...args], { stdout: "pipe", stderr: "pipe" });
  const [out, err, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
  return { code, out, err };
}

async function run(cmd: string, args: string[]): Promise<string> {
  const r = await exec(cmd, args);
  if (r.code !== 0) {
    const what = cmd === "xcrun" ? args.slice(0, 2).join(" ") : `${basename(cmd)} ${args[0] ?? ""}`;
    throw new Error(`${what} failed (exit ${r.code}):\n${(r.err.trim() || r.out.trim()).slice(0, 4000)}`);
  }
  return r.out;
}

async function notarise(path: string, creds: NotaryCredentials, log: PackageContext["log"]): Promise<void> {
  log.info(`notarising ${basename(path)} (notarytool waits for Apple's answer, usually a few minutes)`);
  // notarytool exits non-zero for a rejected submission too, with its JSON still on stdout.
  const submitted = await exec("xcrun", notarySubmitArgs(path, creds));
  if (submitted.code !== 0 && !submitted.out.trim().startsWith("{")) {
    throw new Error(
      `notarytool submit failed (exit ${submitted.code}):\n${submitted.err.trim() || submitted.out.trim()}`,
    );
  }
  const result = parseNotaryResult(submitted.out);
  if (result.status !== "Accepted") {
    let details = "";
    try {
      details = await run("xcrun", ["notarytool", "log", result.id, ...notaryAuthArgs(creds)]);
    } catch (e) {
      details = (e as Error).message;
    }
    throw new Error(
      `notarisation of ${basename(path)} was ${result.status.toLowerCase()} (submission ${result.id}): ${result.message}\n${details.trim()}`,
    );
  }
  await run("xcrun", ["stapler", "staple", path]);
  log.info(`notarised and stapled ${basename(path)}`);
}

async function detach(mount: string): Promise<void> {
  for (let attempt = 0; ; attempt++) {
    try {
      await run("hdiutil", ["detach", mount, ...(attempt ? ["-force"] : [])]);
      return;
    } catch (e) {
      if (attempt === 2) throw e;
      await Bun.sleep(1000 * (attempt + 1));
    }
  }
}

async function write(ctx: PackageContext): Promise<Artifact[]> {
  if (process.platform !== "darwin") throw new Error(NOT_ON_MAC);
  const blocking = validateDmg(ctx).filter((p) => p.severity === "error");
  if (blocking.length) throw new Error(blocking.map((p) => `${p.code} ${p.key}: ${p.message}`).join("\n"));
  const app = ctx.appBundle;
  if (!app || !existsSync(app)) throw new Error("sk build wrote no .app to put in the disk image");
  const dmg = resolved(ctx);
  const appName = basename(app);
  const work = join(ctx.workDir, "dmg");
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });

  let creds: NotaryCredentials | null = null;
  const keyFile = join(work, "AuthKey.p8");
  try {
    if (dmg.notarize) {
      creds = notaryCredentials(dmg.notarize, process.env, keyFile);
      const zip = join(work, `${appName}.zip`);
      await run("ditto", ["-c", "-k", "--keepParent", app, zip]);
      await notarise(zip, creds, ctx.log);
      await run("xcrun", ["stapler", "staple", app]);
    }

    const rw = join(work, "rw.dmg");
    const extra = (dmg.background ? statSync(dmg.background).size * 3 : 0) + (2 << 20);
    const megabytes = Math.ceil(((diskUsage(app) + extra) * 1.1) / (1 << 20)) + 16;
    await run("hdiutil", hdiutilCreateArgs(rw, dmg.volumeName, megabytes));
    const mount = mountPointFrom(
      await run("hdiutil", [
        "attach",
        rw,
        "-readwrite",
        "-nobrowse",
        "-noverify",
        "-noautoopen",
        "-mountrandom",
        work,
        "-plist",
      ]),
    );
    try {
      await run("ditto", [app, join(mount, appName)]);
      symlinkSync("/Applications", join(mount, "Applications"));
      let background: { alias: Uint8Array } | null = null;
      if (dmg.background) {
        const dir = join(mount, ".background");
        mkdirSync(dir);
        const retina = dmg.background.replace(/\.png$/i, "@2x.png");
        let file: string;
        if (extname(dmg.background).toLowerCase() === ".png" && existsSync(retina)) {
          file = join(dir, "background.tiff");
          await run("tiffutil", ["-cathidpicheck", dmg.background, retina, "-out", file]);
        } else {
          file = join(dir, `background${isTiff(dmg.background) ? ".tiff" : ".png"}`);
          writeFileSync(file, readFileSync(dmg.background));
        }
        await run("chflags", ["hidden", dir]);
        const st = statSync(file);
        background = {
          alias: aliasRecord({
            volumeName: dmg.volumeName,
            volumeCreated: statSync(mount).birthtime,
            path: `/.background/${basename(file)}`,
            cnid: st.ino,
            parentCnid: statSync(dir).ino,
            created: st.birthtime,
          }),
        };
      }
      const icns = join(app, "Contents", "Resources", `${ICON_NAME}.icns`);
      if (dmg.volumeIcon && existsSync(icns)) {
        writeFileSync(join(mount, ".VolumeIcon.icns"), readFileSync(icns));
        // kHasCustomIcon in the root folder's Finder flags.
        await run("xattr", ["-wx", "com.apple.FinderInfo", `${"0".repeat(16)}0400${"0".repeat(44)}`, mount]);
      }
      writeFileSync(join(mount, ".DS_Store"), dsStore(layoutRecords(dmg, appName, background)));
    } finally {
      await detach(mount);
    }

    mkdirSync(ctx.outDir, { recursive: true });
    const out = join(ctx.outDir, `${dmg.fileName.replace("{arch}", ctx.arch)}.dmg`);
    await run("hdiutil", hdiutilConvertArgs(rw, out, dmg.format));
    rmSync(rw, { force: true });

    let signed = false;
    const identity = signIdentity(ctx, dmg);
    if (ctx.sign) {
      await ctx.sign(out, "dmg");
      signed = true;
    } else if (identity) {
      await run("codesign", ["--force", "--timestamp", "--sign", identity, out]);
      signed = true;
    }
    if (creds) await notarise(out, creds, ctx.log);
    ctx.log.info(`wrote ${out}`);
    return [{ path: out, kind: "installer", format: "dmg", signed }];
  } finally {
    rmSync(keyFile, { force: true });
  }
}

export const format: PackageFormat<"dmg"> = {
  name: "dmg",
  platform: "macos",
  optionKeys: KEYS,
  parseOptions(raw, ctx, problems) {
    const parsed = parseOptions(raw, ctx, problems);
    return resolveDmg(parsed, ctx.identity.displayName, ctx.identity.version);
  },
  validate: (ctx) => validateDmg(ctx),
  write,
};
