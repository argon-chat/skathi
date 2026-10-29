// `vn build`: an app directory in, one exe out — no Rust toolchain.
//
//   vn.config.json ─┐
//   bun prod.ts ────┼─► app.prod.js + window.json ─┐
//   "assets" ───────┘                               ├─► VNPK ─► stamp(copy of the bare host)
//   vn/app.json (name, version, host) ──────────────┘             │
//                                                                 ▼
//                                                  <appRoot>/dist/<exeName>.exe
//
// Intermediates go to `dist/.vn/` (`app.prod.js`, `window.json`, `assets.json`,
// `app.vnpk`). `assets.json` is the builder's list of files the bundle's
// imports refer to (`?url`, image imports); they are packed beside "assets"
// and "publicDir".
// The `.vnpk` is the exact blob that was stamped, so a bare host can run it
// without a re-stamp while iterating: `VN_PAYLOAD=dist/.vn/app.vnpk <host>`.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { collectAssets, type AssetFile } from "./assets";
import { loadAppConfig, type AppConfig, type Variant } from "./config";
import { CLI_VERSION, PKG_DIR, resolveHost, type ResolvedHost } from "./host";
import { iconImagesFrom } from "./icon";
import { stamp, type StampResult } from "./stamp";
import { buildVersionInfo, numericVersion } from "./versioninfo";
import { FORMAT, KIND, NAMES, writeVnpk, type VnpkInput } from "./vnpk";

export const PROD_TS = join(PKG_DIR, "prod.ts");

/** A PE image is capped at 4 GiB; long before that, a sidecar is the right tool (design §8). */
export const WARN_PAYLOAD_BYTES = 512 * 1024 * 1024;
export const MAX_PAYLOAD_BYTES = 1536 * 1024 * 1024;

export interface BuildOptions {
  /** An app directory, its entry module, or its vn.config.json. */
  target: string;
  /** Output directory. Default `<appRoot>/dist`. */
  out?: string;
  host?: string;
  variant?: Variant;
  /** Keep the console subsystem (stderr in a console window). */
  console?: boolean;
  /** Write `dist/.vn/app.vnpk` and stop — no host needed. */
  payloadOnly?: boolean;
  allowHostMismatch?: boolean;
  /** `--define K=V`, each passed to prod.ts on top of vn.config.json `define`. */
  defines?: string[];
  log?: (line: string) => void;
}

/** One entry of the builder's `assets.json` (prod.ts): a file an import referred to. */
interface BuilderAsset {
  key: string;
  path: string;
}

function readBuilderAssets(path: string): BuilderAsset[] {
  if (!existsSync(path)) return [];
  const list = JSON.parse(readFileSync(path, "utf8")) as unknown;
  if (!Array.isArray(list)) throw new Error(`${path}: not a list`);
  return list.filter(
    (a): a is BuilderAsset => !!a && typeof (a as BuilderAsset).key === "string" && typeof (a as BuilderAsset).path === "string",
  );
}

export interface BuildResult {
  config: AppConfig;
  variant: Variant;
  exe: string | null;
  vnpk: string;
  host: ResolvedHost | null;
  stamp: StampResult | null;
  assets: AssetFile[];
  bundleBytes: number;
  payloadBytes: number;
  warnings: string[];
  ms: { bundle: number; assets: number; stamp: number; total: number };
}

const enc = new TextEncoder();

/** The META entry. No timestamps — two builds of one tree are byte-identical. */
export function appMeta(config: AppConfig, variant: Variant, host: ResolvedHost | null, assetCount: number): string {
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
    // Read by the host before anything else exists (docs/LAUNCH.md): the
    // single-instance check and the per-user protocol registration.
    launch: config.launch,
  });
}

export async function buildApp(options: BuildOptions): Promise<BuildResult> {
  const log = options.log ?? console.log;
  const t0 = performance.now();
  const config = loadAppConfig(options.target);
  const warnings = [...config.warnings];
  const variant: Variant = options.variant ?? config.windows.host ?? "standard";
  const outDir = resolve(options.out ?? join(config.appRoot, "dist"));
  const work = join(outDir, ".vn");
  mkdirSync(work, { recursive: true });

  // The host first: a missing or mismatched host should fail before a bundle
  // is spent on it.
  const host = options.payloadOnly
    ? null
    : resolveHost({ host: options.host, variant, allowMismatch: options.allowHostMismatch, log });
  if (host) {
    log(`[vn] host: ${host.exe} (${host.source}; ${host.info.variant}, vue-native ${host.info.vueNative}, ${host.info.profile})`);
  }

  // 1. The bundle — the same `prod.ts` every prod build in this repo uses.
  const jsOut = join(work, "app.prod.js");
  const assetsJson = join(work, "assets.json");
  // A stale list from an older build must not survive a builder that wrote none.
  rmSync(assetsJson, { force: true });
  const tb = performance.now();
  const defineArgs = (options.defines ?? []).map((d) => `--define=${d}`);
  const r = spawnSync(process.execPath, [PROD_TS, config.entry, jsOut, ...defineArgs], { cwd: PKG_DIR, encoding: "utf8" });
  if (r.status !== 0 || !existsSync(jsOut)) {
    throw new Error(`bun prod.ts failed for ${config.entry}\n${r.stdout ?? ""}${r.stderr ?? ""}`);
  }
  for (const line of `${r.stdout ?? ""}${r.stderr ?? ""}`.trim().split(/\r?\n/)) if (line) log(`[vn] ${line}`);
  const script = readFileSync(jsOut);
  const windowJson = readFileSync(join(work, "window.json"));
  const msBundle = performance.now() - tb;

  // 2. The assets.
  const ta = performance.now();
  const collected = collectAssets(config.appRoot, config.assets, config.assetsDeclared, config.publicDir);
  warnings.push(...collected.warnings);
  // What the bundle's imports referred to (`?url`, `import logo from "./a.png"`),
  // keyed the way the bundle spells the URL. They live under src/ or in a
  // package, which the "assets" walk never packages, so they are added here.
  const known = new Map(collected.files.map((f) => [f.key, f]));
  for (const a of readBuilderAssets(assetsJson)) {
    const prev = known.get(a.key);
    if (prev) {
      if (resolve(prev.path) !== resolve(a.path)) {
        warnings.push(`${JSON.stringify(a.key)}: an import refers to ${a.path}, but "assets" packages ${prev.path} under that key — the import's file wins`);
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

  // 3. The payload.
  const inputs: VnpkInput[] = [
    { name: NAMES.META, kind: KIND.META, data: enc.encode(appMeta(config, variant, host, collected.files.length)) },
    { name: NAMES.WINDOW, kind: KIND.WINDOW, data: windowJson },
    { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: script },
    ...collected.files.map((f) => ({ name: f.key, kind: KIND.ASSET, data: new Uint8Array(readFileSync(f.path)) })),
  ];
  const vnpk = join(work, "app.vnpk");

  let stamped: StampResult | null = null;
  let exe: string | null = null;
  const ts = performance.now();
  if (options.payloadOnly || !host) {
    const blob = writeVnpk(inputs);
    writeFileSync(vnpk, blob);
  } else {
    const fv = numericVersion(config.version);
    const exeFile = `${config.exeName}.exe`;
    exe = join(outDir, exeFile);
    const w = config.windows;
    stamped = stamp({
      host: host.exe,
      out: exe,
      payload: (skew) => writeVnpk(inputs, { skew }),
      icons: config.iconPath ? iconImagesFrom(config.iconPath) : undefined,
      versionInfo: buildVersionInfo({
        fileVersion: [fv[0], fv[1], fv[2], 0],
        strings: {
          // Task Manager names a process by its FileDescription, so it is the
          // product name and not a sentence — what Electron does too.
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
    writeFileSync(vnpk, stamped.payload);
  }
  const msStamp = performance.now() - ts;

  for (const w of warnings) log(`[vn] warning: ${w}`);
  return {
    config,
    variant,
    exe,
    vnpk,
    host,
    stamp: stamped,
    assets: collected.files,
    bundleBytes: script.length,
    payloadBytes: stamped?.payloadBytes ?? readFileSync(vnpk).length,
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
  lines.push(`${c.productName} ${c.version} — name "${c.name}" (storage: %LOCALAPPDATA%\\${c.name}\\storage)`);
  lines.push(`  bundle   ${kib(r.bundleBytes)} in ${r.ms.bundle} ms`);
  lines.push(`  assets   ${r.assets.length} file(s), ${kib(r.assets.reduce((n, a) => n + a.size, 0))}`);
  lines.push(`  payload  ${kib(r.payloadBytes)} → ${r.vnpk}`);
  if (r.stamp && r.exe) {
    const s = r.stamp;
    lines.push(
      `  exe      ${r.exe} — ${mib(s.bytes)}, ${r.variant} host, subsystem ${s.subsystem === 2 ? "GUI" : "console"}, ` +
        `${s.iconCount ? `${s.iconCount} icon` : "no icon"}, ${s.passes} pass(es), data ${s.dataAligned ? "page-aligned" : "NOT aligned"}`,
    );
    lines.push(`  stamp    ${s.ms.total} ms (copy ${s.ms.copy}, resources ${s.ms.resources}, verify ${s.ms.verify})`);
  }
  lines.push(`  total    ${r.ms.total} ms`);
  return lines.join("\n");
}
