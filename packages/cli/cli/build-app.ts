// `sk build`: an app directory in, one exe out, no Rust toolchain.
//
//   sk.config.json + prod.ts (app.prod.js, window.json) + "assets" -> SKPK -> stamp(copy of the bare host)
//   -> <appRoot>/dist/<exeName>.exe
//
// Intermediates go to `dist/.sk/` (`app.prod.js`, `window.json`, `assets.json`, `app.skpk`). The
// `.skpk` is the exact blob that was stamped, so `SK_PAYLOAD=dist/.sk/app.skpk <host>` runs it
// without a re-stamp. On macOS (or `--platform macos` with `--host <mac sk-app>`) the last step is
// macos-bundle.ts: the host is copied into `<exeName>.app`, the SKPK goes in Contents/Resources,
// and the bundle is signed (ad hoc unless an identity is given). On Linux the SKPK is appended to a
// copy of the host ELF (elf.ts). `PACKAGERS` holds the three. With `--aot` (Windows x64), aot-build.ts adds
// the app's native image to the SKPK.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { aotCheck, aotPlatformProblem, buildNative, hostAot, type NativeBuild } from "./aot-build";
import { collectAssets, type AssetFile } from "./assets";
import { ConfigError, loadAppConfig, parseVariant, type AppConfig, type PackConfig, type Variant } from "./config";
import { stampElf, type ElfStampResult } from "./elf";
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
import { lineProgress, logTo, paint, progressFor, seconds, type Progress } from "./term";
import { buildVersionInfo, numericVersion } from "./versioninfo";
import { FORMAT, KIND, NAMES, readHeader, skewFor, writeSkpk, type SkpkInput } from "./skpk";

export const PROD_TS = join(PKG_DIR, "prod.ts");

export const WARN_PAYLOAD_BYTES = 512 * 1024 * 1024;
export const MAX_PAYLOAD_BYTES = 1536 * 1024 * 1024;

export interface BuildOptions {
  target: string;
  /** Read this config instead of <appRoot>/sk.config.json; paths still belong to appRoot. */
  configPath?: string | undefined;
  out?: string | undefined;
  host?: string | undefined;
  /** The host variant (standard or no-devtools); overrides the mode's. */
  variant?: Variant | undefined;
  console?: boolean;
  payloadOnly?: boolean;
  allowHostMismatch?: boolean;
  defines?: string[] | undefined;
  aotInputs?: boolean;
  /** Compile the app to a native image (over sk.config.json "aot"). */
  aot?: boolean | undefined;
  /** The native image compiler to use instead of the release's (SDK development). */
  aotCompiler?: string | undefined;
  /** Colors in a report that refuses the build. */
  color?: boolean;
  platform?: Platform | undefined;
  /** macOS: a codesign identity (`-` = ad hoc), or false for none; over `macos.sign`. */
  sign?: string | false | undefined;
  /**
   * `sk build` is "debug": devtools always in, the bundle not minified, <app>/dist/debug. `sk publish` is
   * "release": no devtools unless sk.config.json has "publishDevtools": true; <app>/dist/release. Unset:
   * the standard host into <app>/dist.
   */
  mode?: "debug" | "release" | undefined;
  /** Plain lines instead of the default progress on stderr. */
  log?: (line: string) => void;
  progress?: Progress;
}

/** The variant a mode builds when `--variant` names none. */
export function modeVariant(config: AppConfig, mode: BuildOptions["mode"]): Variant {
  return mode === "release" && !config.publishDevtools ? "no-devtools" : "standard";
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
  skpk: string;
  host: ResolvedHost | null;
  stamp: StampResult | null;
  bundle: MacBundle | null;
  elf: ElfStampResult | null;
  assets: AssetFile[];
  bundleBytes: number;
  payloadBytes: number;
  packs: { name: string; path: string; bytes: number; assets: number }[];
  aotInputs: string | null;
  native: {
    manifest: string;
    imageBytes: number;
    compressedBytes: number;
    entry: number;
    compiler: string;
    ms: number;
  } | null;
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
    skathi: CLI_VERSION,
    host: { version: host?.info.skathi ?? CLI_VERSION, variant },
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
  const inputs: SkpkInput[] = files.map((f) => ({
    name: f.key,
    kind: KIND.ASSET,
    data: new Uint8Array(readFileSync(f.path)),
  }));
  const unskewed = writeSkpk(inputs);
  const header = readHeader(unskewed);
  const skew = skewFor(0, header);
  const blob = skew ? writeSkpk(inputs, { skew }) : unskewed;
  const id = createHash("sha256")
    .update(blob.subarray(0, header.namesOff + header.namesLen))
    .digest("hex");
  return { name, file: `${name}.skpk`, id, blob, assets: files };
}

interface PackageInput {
  config: AppConfig;
  host: ResolvedHost;
  inputs: SkpkInput[];
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
  elf?: ElfStampResult;
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
    payload: (skew) => writeSkpk(inputs, { skew }),
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
      `the bundle identifier is ${JSON.stringify(m.bundleId)} (from "id") — set sk.config.json "macos.bundleId" ` +
        'to a reverse-DNS id such as "com.example.app"; signing and notarisation expect one',
    );
  }
  // The host maps app.skpk from offset 0, so the data region must be page-aligned in the FILE.
  const unskewed = writeSkpk(inputs);
  const skew = skewFor(0, readHeader(unskewed));
  const payload = skew ? writeSkpk(inputs, { skew }) : unskewed;

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

function linuxExe({ config, host, inputs, packs, outDir }: PackageInput): Packaged {
  for (const p of packs) writeFileSync(join(outDir, p.file), p.blob);
  const elf = stampElf({
    host: host.exe,
    out: join(outDir, config.exeName),
    payload: (skew) => writeSkpk(inputs, { skew }),
  });
  return { exe: elf.out, payload: elf.payload, stamp: null, bundle: null, elf };
}

const PACKAGERS: Record<Platform, (input: PackageInput) => Packaged> = {
  windows: stampExe,
  macos: macBundle,
  linux: linuxExe,
};

export async function buildApp(options: BuildOptions): Promise<BuildResult> {
  const progress = options.progress ?? (options.log ? lineProgress(options.log, false) : progressFor(process.stderr));
  try {
    return await build(options, progress);
  } finally {
    if (!options.progress) progress.end();
  }
}

const kib = (n: number) => `${(n / 1024).toFixed(0)} KiB`;
const mib = (n: number) => `${(n / 2 ** 20).toFixed(1)} MiB`;

async function build(options: BuildOptions, progress: Progress): Promise<BuildResult> {
  const log = logTo(progress);
  const t0 = performance.now();
  const platform = options.platform ?? currentPlatform();
  const config = loadAppConfig(options.target, { configPath: options.configPath, platform });
  const warnings = [...config.warnings];
  const variant: Variant =
    options.variant === undefined ? modeVariant(config, options.mode) : parseVariant(options.variant, "--variant");
  const aot = options.aot ?? config.aot;
  if (aot) {
    const problem = aotPlatformProblem(platform);
    if (problem) throw new ConfigError(problem);
    if (options.payloadOnly)
      throw new ConfigError("--aot compiles against the host it stamps, so it cannot be combined with --payload-only");
  }
  const outDir = resolve(options.out ?? join(config.appRoot, "dist", ...(options.mode ? [options.mode] : [])));
  const work = join(outDir, ".sk");
  mkdirSync(work, { recursive: true });

  // The host first: a missing or mismatched host should fail before a bundle is spent on it.
  if (!options.payloadOnly) progress.step("host");
  const host = options.payloadOnly
    ? null
    : await resolveOrFetchHost({
        host: options.host,
        variant,
        allowMismatch: options.allowHostMismatch,
        platform,
        log,
      });
  if (host) {
    progress.detail(`${host.info.variant} · Skathi ${host.info.skathi} ${host.info.profile} · ${host.source}`);
    if (host.unverified) warnings.push(host.unverified);
  }
  const hostNative = aot && host ? hostAot(host) : null;

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
  progress.step("bundle");
  const child = Bun.spawn([process.execPath, PROD_TS, config.entry, jsOut, ...pathArgs, ...defineArgs], {
    cwd: PKG_DIR,
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr, status] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  if (status !== 0 || !existsSync(jsOut)) {
    throw new Error(`the production bundle of ${config.entry} failed\n${stdout}${stderr}`.trimEnd());
  }
  for (const line of `${stdout}${stderr}`.trim().split(/\r?\n/)) {
    if (!line) continue;
    const text = line.replace(/^\[prod\] /, "");
    const made = /^prod build: (.*?) -> /.exec(text);
    if (made) progress.detail(made[1]!);
    else if (text.startsWith("advisory: ")) progress.warn(text.slice("advisory: ".length));
    else progress.info(text);
  }
  const script = readFileSync(jsOut);
  const windowJson = readFileSync(join(work, "window.json"));
  const msBundle = performance.now() - tb;
  if (aot) await aotCheck(options.target, options.configPath, progress, options.color ?? false);

  progress.step("assets");
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
      warnings.push(`sk.config.json "packs": ${JSON.stringify(p.name)} takes no asset — no ${p.name}.skpk is written`);
      continue;
    }
    packs.push(writePack(p.name, p.files));
  }
  const inputs: SkpkInput[] = [
    {
      name: NAMES.META,
      kind: KIND.META,
      data: enc.encode(appMeta(config, variant, host, collected.files.length, packs)),
    },
    { name: NAMES.WINDOW, kind: KIND.WINDOW, data: windowJson },
    { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: script },
    ...split.own.map((f) => ({ name: f.key, kind: KIND.ASSET, data: new Uint8Array(readFileSync(f.path)) })),
  ];
  const skpk = join(work, "app.skpk");
  for (const p of packs) writeFileSync(join(work, p.file), p.blob);
  progress.detail(`${collected.files.length} files · ${kib(collected.totalBytes)}`);

  let native: NativeBuild | null = null;
  const tn = performance.now();
  if (hostNative) {
    native = await buildNative({
      source: script,
      aot: hostNative,
      compilerPath: options.aotCompiler,
      work: join(work, "aot"),
      progress,
      log,
    });
    inputs.push(...native.inputs);
  }
  const msNative = performance.now() - tn;

  let packaged: Packaged | null = null;
  const ts = performance.now();
  progress.step(options.payloadOnly || !host ? "payload" : platform === "macos" ? "app bundle" : "exe");
  if (options.payloadOnly || !host) {
    // An app split into packs ships app.skpk as a file mapped from offset 0: page-align its data.
    const unskewed = writeSkpk(inputs);
    const skew = packs.length ? skewFor(0, readHeader(unskewed)) : 0;
    writeFileSync(skpk, skew ? writeSkpk(inputs, { skew }) : unskewed);
  } else {
    packaged = PACKAGERS[platform]({ config, host, inputs, packs, outDir, work, options, warnings, log });
    writeFileSync(skpk, packaged.payload);
  }
  const msStamp = performance.now() - ts;
  progress.detail(mib(packaged?.payload.length ?? readFileSync(skpk).length));
  if (packaged?.bundle) progress.detail(mib(packaged.bundle.bytes));
  else if (packaged?.stamp) progress.detail(mib(packaged.stamp.bytes));
  else if (packaged?.elf) progress.detail(mib(packaged.elf.bytes));
  progress.end();

  for (const w of warnings) progress.warn(w);
  return {
    config,
    platform,
    variant,
    exe: packaged?.exe ?? null,
    skpk,
    host,
    stamp: packaged?.stamp ?? null,
    bundle: packaged?.bundle ?? null,
    elf: packaged?.elf ?? null,
    assets: collected.files,
    bundleBytes: script.length,
    aotInputs: options.aotInputs ? join(work, "aot-inputs", "manifest.json") : null,
    native: native && {
      manifest: native.manifestPath,
      imageBytes: native.imageBytes,
      compressedBytes: native.compressedBytes,
      entry: native.entry,
      compiler: native.compiler.path,
      ms: Math.round(msNative),
    },
    payloadBytes: packaged?.payload.length ?? readFileSync(skpk).length,
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

export function summary(r: BuildResult, color = false): string {
  const p = paint(color);
  const c = r.config;
  const storage =
    r.platform === "macos"
      ? `~/Library/Application Support/${c.name}/storage`
      : r.platform === "linux"
        ? `~/.local/share/${c.name}/storage`
        : `%LOCALAPPDATA%\\${c.name}\\storage`;
  const row = (label: string, text: string) => `  ${p.dim(label.padEnd(8))} ${text}`;
  const bad = (ok: boolean, yes: string, no: string) => (ok ? yes : p.yellow(no));
  const lines = [`${p.bold(`${c.productName} ${c.version}`)} ${p.dim(`name "${c.name}", storage ${storage}`)}`];
  if (r.stamp && r.exe) {
    const s = r.stamp;
    lines.push(
      row("exe", p.cyan(r.exe)),
      row(
        "",
        `${mib(s.bytes)}, ${r.variant} host, subsystem ${s.subsystem === 2 ? "GUI" : "console"}, ` +
          `${s.iconCount ? `${s.iconCount} icon` : "no icon"}, ` +
          bad(s.dataAligned, "data page-aligned", "data NOT aligned"),
      ),
    );
  }
  if (r.elf) {
    const e = r.elf;
    lines.push(
      row("exe", p.cyan(e.out)),
      row(
        "",
        `${mib(e.bytes)}, ${r.variant} host (${e.machine}), payload appended at ${e.payloadOffset}, ` +
          bad(e.dataAligned, "data page-aligned", "data NOT aligned"),
      ),
    );
  }
  if (r.bundle) {
    const b = r.bundle;
    const icon = b.iconSizes.length ? `icon ${b.iconSizes.join("/")} px` : "no icon";
    const signed =
      b.signed === "-" ? "signed ad hoc" : b.signed ? `signed as ${JSON.stringify(b.signed)}` : p.yellow("NOT signed");
    lines.push(
      row("app", p.cyan(b.app)),
      row("", `${mib(b.bytes)}, ${r.variant} host, ${b.bundleId}, ${icon}, ${signed}`),
    );
    if (b.signNote) lines.push(row("", p.dim(b.signNote)));
  }
  lines.push(
    row(
      "bundle",
      `${kib(r.bundleBytes)} script, ${r.assets.length} asset(s) ${kib(r.assets.reduce((n, a) => n + a.size, 0))}`,
    ),
    row("payload", `${kib(r.payloadBytes)} ${p.dim(r.skpk)}`),
  );
  if (r.native) {
    lines.push(
      row(
        "native",
        `${mib(r.native.imageBytes)} image (${mib(r.native.compressedBytes)} in the payload), entry ${r.native.entry} ` +
          p.dim(r.native.manifest),
      ),
    );
  }
  for (const pk of r.packs)
    lines.push(row("pack", `${pk.name}: ${pk.assets} asset(s), ${kib(pk.bytes)} ${p.dim(pk.path)}`));
  lines.push(`${p.green("done")} in ${seconds(r.ms.total)}`);
  return lines.join("\n");
}
