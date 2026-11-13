// `vn build`: an app directory in, one exe out, no Rust toolchain.
//
//   vn.config.json + prod.ts (app.prod.js, window.json) + "assets" -> VNPK -> stamp(copy of the bare host)
//   -> <appRoot>/dist/<exeName>.exe
//
// Intermediates go to `dist/.vn/` (`app.prod.js`, `window.json`, `assets.json`, `app.vnpk`). The
// `.vnpk` is the exact blob that was stamped, so `VN_PAYLOAD=dist/.vn/app.vnpk <host>` runs it
// without a re-stamp. On macOS (or `--platform macos` with `--host <mac vn-app>`) the last step is
// macos-bundle.ts: the host is copied into `<exeName>.app`, the VNPK goes in Contents/Resources,
// and the bundle is signed (ad hoc unless an identity is given). `PACKAGERS` holds the two.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { collectAssets, type AssetFile } from "./assets";
import {
  loadAppConfig,
  partsLeftOut,
  variantFor,
  variantWarnings,
  type AppConfig,
  type PackConfig,
  type Variant,
} from "./config";
import { CLI_VERSION, PKG_DIR, type ResolvedHost } from "./host";
import { resolveOrFetchHost } from "./host-fetch";
import { iconImagesFrom } from "./icon";
import {
  entitlements,
  icnsFrom,
  ICON_NAME,
  infoPlist,
  plistXml,
  signBundle,
  signPlan,
  writeAppBundle,
  type AppBundleResult,
} from "./macos-bundle";
import { currentPlatform, type Platform } from "./platform";
import { stamp, type StampResult } from "./stamp";
import { buildVersionInfo, numericVersion } from "./versioninfo";
import { FORMAT, KIND, NAMES, readHeader, skewFor, writeVnpk, type VnpkInput } from "./vnpk";

export const PROD_TS = join(PKG_DIR, "prod.ts");

export const WARN_PAYLOAD_BYTES = 512 * 1024 * 1024;
export const MAX_PAYLOAD_BYTES = 1536 * 1024 * 1024;

export interface BuildOptions {
  target: string;
  /** Read this config instead of <appRoot>/vn.config.json; paths still belong to appRoot. */
  configPath?: string | undefined;
  out?: string | undefined;
  host?: string | undefined;
  /** The host variant; overrides vn.config.json's `windows` selection. */
  variant?: Variant | undefined;
  console?: boolean;
  payloadOnly?: boolean;
  allowHostMismatch?: boolean;
  defines?: string[] | undefined;
  aotInputs?: boolean;
  platform?: Platform | undefined;
  /** macOS: a codesign identity (`-` = ad hoc), or false for none; over `macos.sign`. */
  sign?: string | false | undefined;
  /**
   * `vn build` is "debug": devtools always in, the bundle not minified, <app>/dist/debug. `vn publish` is
   * "release": vn.config.json's engine selection, and without one, no devtools; <app>/dist/release. Unset:
   * the config's selection into <app>/dist, as the API always did.
   */
  mode?: "debug" | "release" | undefined;
  log?: (line: string) => void;
}

/** The variant a mode builds when neither `--variant` nor a selection flag names one. */
export function modeVariant(config: AppConfig, mode: BuildOptions["mode"]): Variant {
  const selected = config.windows.variant;
  if (mode === "debug") return variantFor(partsLeftOut(selected).filter((p) => p !== "devtools"));
  if (mode === "release" && !config.windows.selected) return "no-devtools";
  return selected;
}

interface BuilderAsset {
  key: string;
  path: string;
}

function readBuilderAssets(path: string): BuilderAsset[] {
  if (!existsSync(path)) return [];
  const list = JSON.parse(readFileSync(path, "utf8")) as unknown;
  if (!Array.isArray(list)) throw new Error(`${path}: not a list`);
  return list.filter(
    (a): a is BuilderAsset =>
      !!a && typeof (a as BuilderAsset).key === "string" && typeof (a as BuilderAsset).path === "string",
  );
}

export interface MacBundle extends AppBundleResult {
  bundleId: string;
  iconSizes: number[];
  signed: string | null;
  signNote: string | null;
  entitlements: string;
}

export interface BuildResult {
  config: AppConfig;
  platform: Platform;
  variant: Variant;
  exe: string | null;
  vnpk: string;
  host: ResolvedHost | null;
  stamp: StampResult | null;
  bundle: MacBundle | null;
  assets: AssetFile[];
  bundleBytes: number;
  payloadBytes: number;
  packs: { name: string; path: string; bytes: number; assets: number }[];
  aotInputs: string | null;
  warnings: string[];
  ms: { bundle: number; assets: number; stamp: number; total: number };
}

const enc = new TextEncoder();

/** The META entry. No timestamps — two builds of one tree are byte-identical. */
export function appMeta(
  config: AppConfig,
  variant: Variant,
  host: ResolvedHost | null,
  assetCount: number,
  packs: BuiltPack[] = [],
): string {
  const entry = relative(config.appRoot, config.entry).split(sep).join("/");
  return JSON.stringify({
    format: FORMAT,
    name: config.name,
    productName: config.productName,
    version: config.version,
    vueNative: CLI_VERSION,
    host: { version: host?.info.vueNative ?? CLI_VERSION, variant },
    entry,
    assets: assetCount,
    launch: config.launch,
    ...(packs.length ? { packs: packs.map((p) => ({ name: p.name, file: p.file, id: p.id })) } : {}),
  });
}

export interface BuiltPack {
  name: string;
  file: string;
  id: string;
  blob: Uint8Array;
  assets: AssetFile[];
}

/** Deals the assets out to the packs in order; what no pack takes stays in the payload. */
export function splitPacks(
  files: AssetFile[],
  packs: PackConfig[],
): { own: AssetFile[]; packs: { name: string; files: AssetFile[] }[] } {
  let left = files;
  const dealt = packs.map((p) => {
    const globs = p.match?.map((m) => new Bun.Glob(m));
    const taken = globs ? left.filter((f) => globs.some((g) => g.match(f.key))) : left;
    left = left.filter((f) => !taken.includes(f));
    return { name: p.name, files: taken };
  });
  return { own: left, packs: dealt };
}

/** An asset pack's blob: assets only, data page-aligned in the file (the host maps it from offset 0), and its id. */
export function writePack(name: string, files: AssetFile[]): BuiltPack {
  const inputs: VnpkInput[] = files.map((f) => ({
    name: f.key,
    kind: KIND.ASSET,
    data: new Uint8Array(readFileSync(f.path)),
  }));
  const unskewed = writeVnpk(inputs);
  const header = readHeader(unskewed);
  const skew = skewFor(0, header);
  const blob = skew ? writeVnpk(inputs, { skew }) : unskewed;
  const id = createHash("sha256")
    .update(blob.subarray(0, header.namesOff + header.namesLen))
    .digest("hex");
  return { name, file: `${name}.vnpk`, id, blob, assets: files };
}

interface PackageInput {
  config: AppConfig;
  host: ResolvedHost;
  inputs: VnpkInput[];
  packs: BuiltPack[];
  outDir: string;
  work: string;
  options: BuildOptions;
  warnings: string[];
  log: (line: string) => void;
}

interface Packaged {
  exe: string;
  payload: Uint8Array;
  stamp: StampResult | null;
  bundle: MacBundle | null;
}

function stampExe({ config, host, inputs, packs, outDir, options, log }: PackageInput): Packaged {
  for (const p of packs) writeFileSync(join(outDir, p.file), p.blob);
  const fv = numericVersion(config.version);
  const exeFile = `${config.exeName}.exe`;
  const exe = join(outDir, exeFile);
  const w = config.windows;
  const stamped = stamp({
    host: host.exe,
    out: exe,
    payload: (skew) => writeVnpk(inputs, { skew }),
    icons: config.iconPath ? iconImagesFrom(config.iconPath) : undefined,
    versionInfo: buildVersionInfo({
      fileVersion: [fv[0], fv[1], fv[2], 0],
      strings: {
        // Task Manager names a process by its FileDescription, so it is the product name.
        FileDescription: config.productName,
        ProductName: config.productName,
        ProductVersion: config.version,
        FileVersion: `${fv[0]}.${fv[1]}.${fv[2]}.0`,
        InternalName: config.name,
        OriginalFilename: exeFile,
        CompanyName: w.company,
        LegalCopyright: w.copyright,
        Comments: w.description,
      },
    }),
    subsystem: options.console || w.console ? "console" : "gui",
    log,
  });
  return { exe, payload: stamped.payload, stamp: stamped, bundle: null };
}

export function macInfoFor(config: AppConfig, hasIcon: boolean) {
  const m = config.macos;
  const fv = numericVersion(config.version);
  return {
    bundleId: m.bundleId,
    name: config.productName,
    executable: m.exeName,
    // CFBundleShortVersionString is three integers; a prerelease tag has no place in it.
    version: `${fv[0]}.${fv[1]}.${fv[2]}`,
    iconFile: hasIcon ? ICON_NAME : null,
    minimumSystemVersion: m.minimumSystemVersion,
    category: m.category,
    copyright: m.copyright,
    microphoneUsageDescription: m.microphoneUsageDescription,
    cameraUsageDescription: m.cameraUsageDescription,
    urlSchemes: config.launch.protocols,
  };
}

function macBundle({ config, host, inputs, packs, outDir, work, options, warnings, log }: PackageInput): Packaged {
  const m = config.macos;
  if (m.bundleIdFrom === "id" && !m.bundleId.includes(".")) {
    warnings.push(
      `the bundle identifier is ${JSON.stringify(m.bundleId)} (from "id") — set vn.config.json "macos.bundleId" ` +
        'to a reverse-DNS id such as "com.example.app"; signing and notarisation expect one',
    );
  }
  // The host maps app.vnpk from offset 0, so the data region must be page-aligned in the FILE.
  const unskewed = writeVnpk(inputs);
  const skew = skewFor(0, readHeader(unskewed));
  const payload = skew ? writeVnpk(inputs, { skew }) : unskewed;

  const icon = m.iconPath ? icnsFrom(m.iconPath) : null;
  if (icon && !icon.verbatim && Math.max(...icon.sizes) < 512) {
    warnings.push(
      `${m.iconPath} is ${Math.max(...icon.sizes)} px at most — the Dock and Finder draw up to 1024 px (512@2x); a 1024 px PNG stays sharp`,
    );
  }
  const info = macInfoFor(config, !!icon);
  const entitlementsPath = join(work, "entitlements.plist");
  writeFileSync(entitlementsPath, plistXml(entitlements(info)));

  const written = writeAppBundle({
    host: host.exe,
    out: join(outDir, `${m.exeName}.app`),
    executable: m.exeName,
    payload,
    icns: icon?.bytes ?? null,
    infoPlist: plistXml(infoPlist(info)),
    log,
  });
  for (const p of packs) writeFileSync(join(written.app, "Contents", "Resources", p.file), p.blob);

  const onMac = process.platform === "darwin";
  const plan = signPlan({
    want: options.sign ?? m.sign,
    app: written.app,
    entitlements: entitlementsPath,
    codesign: onMac ? Bun.which("codesign") : null,
    onMac,
  });
  if (plan.identity) signBundle(Bun.which("codesign")!, written.app, plan.identity, entitlementsPath, log);
  return {
    exe: written.executable,
    payload,
    stamp: null,
    bundle: {
      ...written,
      bundleId: m.bundleId,
      iconSizes: icon?.sizes ?? [],
      signed: plan.identity,
      signNote: plan.note,
      entitlements: entitlementsPath,
    },
  };
}

const PACKAGERS: Record<Platform, (input: PackageInput) => Packaged> = { windows: stampExe, macos: macBundle };

export async function buildApp(options: BuildOptions): Promise<BuildResult> {
  const log = options.log ?? console.log;
  const t0 = performance.now();
  const platform = options.platform ?? currentPlatform();
  const config = loadAppConfig(options.target, { configPath: options.configPath, platform });
  const warnings = [...config.warnings];
  const asked: Variant = options.variant ?? modeVariant(config, options.mode);
  partsLeftOut(asked);
  warnings.push(...variantWarnings(asked));
  const outDir = resolve(options.out ?? join(config.appRoot, "dist", ...(options.mode ? [options.mode] : [])));
  const work = join(outDir, ".vn");
  mkdirSync(work, { recursive: true });

  // The host first: a missing or mismatched host should fail before a bundle is spent on it.
  const host = options.payloadOnly
    ? null
    : await resolveOrFetchHost({
        host: options.host,
        variant: asked,
        allowMismatch: options.allowHostMismatch,
        platform,
        log,
      });
  if (host) {
    log(
      `[vn] host: ${host.exe} (${host.source}; ${host.info.variant}, vue-native ${host.info.vueNative}, ${host.info.profile})`,
    );
    if (host.unverified) warnings.push(host.unverified);
    if (host.substitute) warnings.push(host.substitute);
  }
  // The payload names the host it runs on, which a stand-in changes.
  const variant: Variant = host?.substitute ? host.info.variant : asked;

  const jsOut = join(work, "app.prod.js");
  const assetsJson = join(work, "assets.json");
  // A stale list from an older build must not survive a builder that wrote none.
  rmSync(assetsJson, { force: true });
  const tb = performance.now();
  const defineArgs = (options.defines ?? []).map((d) => `--define=${d}`);
  const pathArgs = [
    `--app-root=${config.appRoot}`,
    ...(config.configPath ? [`--config=${config.configPath}`] : []),
    ...(options.aotInputs ? [`--aot-inputs=${join(work, "aot-inputs")}`] : []),
    ...(options.mode === "debug" ? ["--no-minify"] : []),
  ];
  const r = spawnSync(process.execPath, [PROD_TS, config.entry, jsOut, ...pathArgs, ...defineArgs], {
    cwd: PKG_DIR,
    encoding: "utf8",
  });
  if (r.status !== 0 || !existsSync(jsOut)) {
    throw new Error(`bun prod.ts failed for ${config.entry}\n${r.stdout ?? ""}${r.stderr ?? ""}`);
  }
  for (const line of `${r.stdout ?? ""}${r.stderr ?? ""}`.trim().split(/\r?\n/)) if (line) log(`[vn] ${line}`);
  const script = readFileSync(jsOut);
  const windowJson = readFileSync(join(work, "window.json"));
  const msBundle = performance.now() - tb;

  const ta = performance.now();
  const collected = collectAssets(config.appRoot, config.assets, config.assetsDeclared, config.publicDir);
  warnings.push(...collected.warnings);
  // What the bundle's imports referred to (`?url`, image imports): they live under src/ or in a
  // package, which the "assets" walk never packages, so they are added here.
  const known = new Map(collected.files.map((f) => [f.key, f]));
  for (const a of readBuilderAssets(assetsJson)) {
    const prev = known.get(a.key);
    if (prev) {
      if (resolve(prev.path) !== resolve(a.path)) {
        warnings.push(
          `${JSON.stringify(a.key)}: an import refers to ${a.path}, but "assets" packages ${prev.path} under that key — the import's file wins`,
        );
        prev.path = a.path;
        prev.size = statSync(a.path).size;
      }
      continue;
    }
    const file: AssetFile = { key: a.key, path: a.path, size: statSync(a.path).size };
    known.set(a.key, file);
    collected.files.push(file);
  }
  collected.files.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  collected.totalBytes = collected.files.reduce((n, f) => n + f.size, 0);
  const msAssets = performance.now() - ta;
  if (collected.totalBytes > MAX_PAYLOAD_BYTES) {
    throw new Error(
      `the assets are ${(collected.totalBytes / 2 ** 20).toFixed(0)} MiB — more than a single exe should carry ` +
        `(limit ${MAX_PAYLOAD_BYTES / 2 ** 20} MiB). Ship large media beside the exe instead.`,
    );
  }
  if (collected.totalBytes > WARN_PAYLOAD_BYTES) {
    warnings.push(`the assets are ${(collected.totalBytes / 2 ** 20).toFixed(0)} MiB — every byte is inside the exe`);
  }

  const split = splitPacks(collected.files, config.packs);
  const packs: BuiltPack[] = [];
  for (const p of split.packs) {
    if (!p.files.length) {
      warnings.push(`vn.config.json "packs": ${JSON.stringify(p.name)} takes no asset — no ${p.name}.vnpk is written`);
      continue;
    }
    packs.push(writePack(p.name, p.files));
  }
  const inputs: VnpkInput[] = [
    {
      name: NAMES.META,
      kind: KIND.META,
      data: enc.encode(appMeta(config, variant, host, collected.files.length, packs)),
    },
    { name: NAMES.WINDOW, kind: KIND.WINDOW, data: windowJson },
    { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: script },
    ...split.own.map((f) => ({ name: f.key, kind: KIND.ASSET, data: new Uint8Array(readFileSync(f.path)) })),
  ];
  const vnpk = join(work, "app.vnpk");
  for (const p of packs) writeFileSync(join(work, p.file), p.blob);

  let packaged: Packaged | null = null;
  const ts = performance.now();
  if (options.payloadOnly || !host) {
    // An app split into packs ships app.vnpk as a file mapped from offset 0: page-align its data.
    const unskewed = writeVnpk(inputs);
    const skew = packs.length ? skewFor(0, readHeader(unskewed)) : 0;
    writeFileSync(vnpk, skew ? writeVnpk(inputs, { skew }) : unskewed);
  } else {
    packaged = PACKAGERS[platform]({ config, host, inputs, packs, outDir, work, options, warnings, log });
    writeFileSync(vnpk, packaged.payload);
  }
  const msStamp = performance.now() - ts;

  for (const w of warnings) log(`[vn] warning: ${w}`);
  return {
    config,
    platform,
    variant,
    exe: packaged?.exe ?? null,
    vnpk,
    host,
    stamp: packaged?.stamp ?? null,
    bundle: packaged?.bundle ?? null,
    assets: collected.files,
    bundleBytes: script.length,
    aotInputs: options.aotInputs ? join(work, "aot-inputs", "manifest.json") : null,
    payloadBytes: packaged?.payload.length ?? readFileSync(vnpk).length,
    packs: packs.map((p) => ({
      name: p.name,
      path: join(work, p.file),
      bytes: p.blob.length,
      assets: p.assets.length,
    })),
    warnings,
    ms: {
      bundle: Math.round(msBundle),
      assets: Math.round(msAssets),
      stamp: Math.round(msStamp),
      total: Math.round(performance.now() - t0),
    },
  };
}

const mib = (n: number) => `${(n / 2 ** 20).toFixed(1)} MiB`;
const kib = (n: number) => `${(n / 1024).toFixed(0)} KiB`;

export function summary(r: BuildResult): string {
  const lines: string[] = [];
  const c = r.config;
  const storage =
    r.platform === "macos" ? `~/Library/Application Support/${c.name}/storage` : `%LOCALAPPDATA%\\${c.name}\\storage`;
  lines.push(`${c.productName} ${c.version} — name "${c.name}" (storage: ${storage})`);
  lines.push(`  bundle   ${kib(r.bundleBytes)} in ${r.ms.bundle} ms`);
  lines.push(`  assets   ${r.assets.length} file(s), ${kib(r.assets.reduce((n, a) => n + a.size, 0))}`);
  lines.push(`  payload  ${kib(r.payloadBytes)} → ${r.vnpk}`);
  for (const p of r.packs) lines.push(`  pack     ${p.name}: ${p.assets} asset(s), ${kib(p.bytes)} → ${p.path}`);
  if (r.stamp && r.exe) {
    const s = r.stamp;
    lines.push(
      `  exe      ${r.exe} — ${mib(s.bytes)}, ${r.variant} host, subsystem ${s.subsystem === 2 ? "GUI" : "console"}, ` +
        `${s.iconCount ? `${s.iconCount} icon` : "no icon"}, ${s.passes} pass(es), data ${s.dataAligned ? "page-aligned" : "NOT aligned"}`,
    );
    lines.push(`  stamp    ${s.ms.total} ms (copy ${s.ms.copy}, resources ${s.ms.resources}, verify ${s.ms.verify})`);
  }
  if (r.bundle) {
    const b = r.bundle;
    const icon = b.iconSizes.length ? `icon ${b.iconSizes.join("/")} px` : "no icon";
    const signed =
      b.signed === "-" ? "signed ad hoc" : b.signed ? `signed as ${JSON.stringify(b.signed)}` : "NOT signed";
    lines.push(`  app      ${b.app} — ${mib(b.bytes)}, ${r.variant} host, ${b.bundleId}, ${icon}, ${signed}`);
    if (b.signNote) lines.push(`           ${b.signNote}`);
    lines.push(`  package  ${b.ms.total} ms (copy ${b.ms.copy})`);
  }
  lines.push(`  total    ${r.ms.total} ms`);
  return lines.join("\n");
}
