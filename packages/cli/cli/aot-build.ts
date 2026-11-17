// `sk build --aot`: the app's production bundle compiled to a native image that travels in the
// payload beside app.prod.js (which stays: the host checks the image against it, and stacks name it).
//
//   sk check (errors refuse the build) -> sk-aot matching the host (--aot-compiler, the checkout's
//   build, or the release asset, cached) -> image + manifest
//
// The host checks the manifest against itself before it loads the image (crates/sk-app/src/native_aot.rs).

import { deflateRawSync } from "node:zlib";
import { existsSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { AotError, compileNative, compilerInfo, compilerMismatch, nativeManifest, repoSkAot } from "../aot/native";
import type { CompilerInfo } from "../aot/pin";
import { TRANSFORM } from "../aot/transform";
import { diagnosticLine } from "../check/finding";
import { exitCode, formatPretty, runCheck, type CheckReport } from "../check/index";
import { ConfigError } from "./config";
import { CLI_VERSION, HostError, REPO_ROOT, type HostAot, type ResolvedHost } from "./host";
import { cachedAotCompiler, fetchAotCompiler, type OnBytes } from "./host-fetch";
import type { Platform } from "./platform";
import { KIND, NAMES, type SkpkInput } from "./skpk";
import type { Progress } from "./term";

export const AOT_PLATFORM_ONLY = "AOT builds are Windows x64 only for now";

/** `--aot`, `--no-aot` and `--aot-compiler <exe>`; an unset `aot` leaves sk.config.json "aot" in charge. */
export function aotFlags(flags: Record<string, string | boolean>): {
  aot: boolean | undefined;
  aotCompiler: string | undefined;
} {
  if (flags.aot === true && flags["no-aot"] === true) throw new ConfigError("--aot and --no-aot contradict each other");
  const compiler = flags["aot-compiler"];
  if (compiler !== undefined && (typeof compiler !== "string" || !compiler.trim()))
    throw new ConfigError("--aot-compiler needs the path of sk-aot.exe");
  return {
    aot: flags["no-aot"] === true ? false : flags.aot === true ? true : undefined,
    aotCompiler: typeof compiler === "string" ? resolve(compiler.trim()) : undefined,
  };
}

/** Why this build cannot be AOT-compiled here, or null. */
export function aotPlatformProblem(
  target: Platform,
  here: { platform: string; arch: string } = { platform: process.platform, arch: process.arch },
): string | null {
  if (target !== "windows")
    return `${AOT_PLATFORM_ONLY}: build ${target} with --no-aot (or without "aot": true in sk.config.json)`;
  if (here.platform !== "win32" || here.arch !== "x64")
    return `${AOT_PLATFORM_ONLY}: the native compiler runs on Windows x64 and this is ${here.platform}-${here.arch}; build here with --no-aot`;
  return null;
}

/** The host's native AOT identity; an error when the host cannot load app images. */
export function hostAot(host: ResolvedHost): HostAot {
  const aot = host.info.aot;
  if (!host.info.features.includes("native-aot") || !aot) {
    throw new HostError(
      `${host.exe} cannot load native images (it was built without the native AOT loader) — ` +
        `use a Skathi ${CLI_VERSION} release workload (sk workload install), or build one with the native-aot cargo feature`,
    );
  }
  if (aot.transform !== TRANSFORM) {
    throw new HostError(
      `${host.exe} expects native images made with the ${aot.transform} transform and this sk makes ${TRANSFORM} — ` +
        "use the sk that matches the workload",
    );
  }
  return aot;
}

export interface CompilerChoice {
  path: string;
  info: CompilerInfo;
  source: "--aot-compiler" | "SK_AOT_COMPILER" | "repo" | "cache" | "release";
}

/**
 * The sk-aot matching `aot`, first hit wins: --aot-compiler or SK_AOT_COMPILER (must match);
 * the checkout's own build; the cache; a download from this version's release.
 */
export async function resolveAotCompiler(
  aot: HostAot,
  named: string | undefined,
  log: (line: string) => void,
  onBytes?: OnBytes,
): Promise<CompilerChoice> {
  const explicit = named ?? process.env.SK_AOT_COMPILER;
  if (explicit) {
    const info = await compilerInfo(explicit);
    const mismatch = compilerMismatch(info, aot);
    if (mismatch) throw new AotError(`${explicit} compiles for another workload: ${mismatch}`);
    return { path: explicit, info, source: named ? "--aot-compiler" : "SK_AOT_COMPILER" };
  }
  const tried: string[] = [];
  const candidates: { path: string; source: "repo" | "cache" }[] = [];
  if (REPO_ROOT) candidates.push({ path: repoSkAot(REPO_ROOT), source: "repo" });
  candidates.push({ path: cachedAotCompiler(CLI_VERSION), source: "cache" });
  for (const c of candidates) {
    if (!existsSync(c.path)) continue;
    const info = await compilerInfo(c.path);
    const mismatch = compilerMismatch(info, aot);
    if (!mismatch) return { ...c, info };
    tried.push(`${c.path}: ${mismatch}`);
  }
  if (process.env.SK_HOST_OFFLINE === "1") {
    throw new AotError(
      `no sk-aot matches the workload${tried.length ? `:\n  ${tried.join("\n  ")}` : ""}\n` +
        "SK_HOST_OFFLINE=1 forbids downloading it; pass --aot-compiler <sk-aot.exe>",
    );
  }
  const fetched = await fetchAotCompiler({ log, onBytes });
  const mismatch = compilerMismatch(fetched.info, aot);
  if (mismatch) throw new AotError(`the Skathi ${CLI_VERSION} release's sk-aot: ${mismatch}`);
  return { path: fetched.path, info: await compilerInfo(fetched.path), source: "release" };
}

/** `sk check` over the app: errors (and fatal findings) refuse the build with the report; warnings are printed. */
export async function aotCheck(
  target: string,
  configPath: string | undefined,
  progress: Progress,
  color: boolean,
): Promise<CheckReport> {
  const report = await runCheck({ target, configPath, toolVersion: CLI_VERSION, progress });
  const code = exitCode(report, false);
  if (code !== 0) {
    throw new ConfigError(
      `sk check found what a native image cannot run, so the AOT build stops here:\n\n${formatPretty(report, { color })}\n\n` +
        "Fix the errors (sk explain <CODE> says how), or build with --no-aot.",
    );
  }
  for (const d of report.diagnostics) progress.warn(diagnosticLine(d));
  return report;
}

export interface NativeBuild {
  inputs: SkpkInput[];
  manifestPath: string;
  imageBytes: number;
  compressedBytes: number;
  entry: number;
  compiler: CompilerChoice;
}

/** The two payload entries that make an app native, written also to `work` for inspection. */
export async function buildNative(options: {
  source: Uint8Array;
  aot: HostAot;
  compilerPath: string | undefined;
  work: string;
  progress: Progress;
  log: (line: string) => void;
}): Promise<NativeBuild> {
  const { progress, log, aot, work } = options;
  progress.step("AOT toolchain");
  const compiler = await resolveAotCompiler(aot, options.compilerPath, log, (done, total) =>
    progress.bytes(done, total),
  );
  progress.detail(`sk-aot (${compiler.source})`);
  progress.step("native image");
  const native = await compileNative({
    compiler: compiler.path,
    source: new TextDecoder().decode(options.source),
    work,
  });
  const compressed = deflateRawSync(native.image, { level: 9 });
  progress.detail(
    `${(native.image.length / 2 ** 20).toFixed(1)} MiB, ${(compressed.length / 2 ** 20).toFixed(1)} MiB packed`,
  );
  const manifest = nativeManifest("application", compiler.info, native, {
    image: NAMES.NATIVE_IMAGE,
    preludeSha256: aot.preludeSha256,
  });
  const text = JSON.stringify(manifest, null, 2);
  const manifestPath = join(work, "app.native.json");
  writeFileSync(manifestPath, text);
  return {
    inputs: [
      { name: NAMES.NATIVE_MANIFEST, kind: KIND.NATIVE_MANIFEST, data: new TextEncoder().encode(text) },
      { name: NAMES.NATIVE_IMAGE, kind: KIND.NATIVE_IMAGE, data: new Uint8Array(compressed) },
    ],
    manifestPath,
    imageBytes: native.image.length,
    compressedBytes: compressed.length,
    entry: native.entry,
    compiler,
  };
}
