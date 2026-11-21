// `sk build --aot`: the app's production bundle compiled to a native image. Its manifest travels in the payload
// beside app.prod.js (which stays: the host checks the image against it, and stacks name it); the image goes into a
// container beside the exe, or into the exe's own sections (image-sections.ts).
//
//   sk check (errors refuse the build) -> sk-aot matching the host (--aot-compiler, the checkout's
//   build, or the release asset, cached) -> image + manifest
//
// The host checks the manifest against itself before it loads the image (crates/sk-app/src/native_aot.rs).

import { createHash } from "node:crypto";
import { existsSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  AotError,
  compileNative,
  compilerInfo,
  compilerMismatch,
  nativeManifest,
  removedFeatureProblem,
  repoSkAot,
} from "../aot/native";
import type { CompilerInfo } from "../aot/pin";
import { CLOSED_PRELUDES_TRANSFORM, TRANSFORM } from "../aot/transform";
import { diagnosticLine } from "../check/finding";
import { exitCode, formatPretty, runCheck, type CheckReport } from "../check/index";
import { ConfigError } from "./config";
import { CLI_VERSION, HostError, REPO_ROOT, type HostAot, type ResolvedHost } from "./host";
import { cachedAotCompiler, fetchAotCompiler, type OnBytes } from "./host-fetch";
import type { Platform } from "./platform";
import { KIND, NAMES, type SkpkInput } from "./skpk";
import type { Progress } from "./term";

export const AOT_PLATFORM_ONLY = "AOT builds require Windows x64 or macOS arm64";

/** `--aot`, `--no-aot` and `--aot-compiler <exe>`; an unset `aot` leaves sk.config.json "aot" in charge. */
export function aotFlags(flags: Record<string, string | boolean>): {
  aot: boolean | undefined;
  aotCompiler: string | undefined;
} {
  if (flags.aot === true && flags["no-aot"] === true) throw new ConfigError("--aot and --no-aot contradict each other");
  const compiler = flags["aot-compiler"];
  if (compiler !== undefined && (typeof compiler !== "string" || !compiler.trim()))
    throw new ConfigError("--aot-compiler needs the path to the sk-aot executable");
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
  if (target === "macos" && here.platform === "darwin" && here.arch === "arm64") return null;
  if (target === "windows" && here.platform === "win32" && here.arch === "x64") return null;
  if (target === "linux")
    return `${AOT_PLATFORM_ONLY}: build ${target} with --no-aot (or without "aot": true in sk.config.json)`;
  const required = target === "macos" ? "macOS on Apple Silicon" : "Windows x64";
  return `The ${target} AOT compiler runs on ${required}; this is ${here.platform}-${here.arch}. Publish on ${required} or build here with --no-aot.`;
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
  if (aot.engine !== host.info.jscPin) {
    throw new HostError(`${host.exe} reports different JavaScriptCore pins in jscPin and its native AOT identity`);
  }
  const scripts = aot.preludeScripts;
  if (
    scripts !== undefined &&
    (!Array.isArray(scripts) ||
      scripts.length !== 2 ||
      !scripts.every((s) => typeof s === "string") ||
      createHash("sha256").update(scripts.join("\0")).digest("hex") !== aot.preludeSha256)
  )
    throw new HostError(`${host.exe} reports invalid AOT prelude scripts or a mismatched preludeSha256`);
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

/** Where the image goes: a container file beside the payload, or sections of the exe ("publishSingleFile"). */
export type ImagePlace = { place: "beside"; file: string } | { place: "executable" };

export interface NativeBuild {
  /** The payload entry that makes an app native: the manifest. The image itself is never in the payload. */
  inputs: SkpkInput[];
  image: Uint8Array;
  manifestPath: string;
  imageBytes: number;
  entry: number;
  compiler: CompilerChoice;
}

/** The app's image and its manifest, written also to `work` for inspection. */
export async function buildNative(options: {
  source: Uint8Array;
  aot: HostAot;
  compilerPath: string | undefined;
  work: string;
  /** The engine features the workload leaves out, and its name. */
  removed: readonly string[];
  workload: string;
  place: ImagePlace;
  progress: Progress;
  log: (line: string) => void;
}): Promise<NativeBuild> {
  const { progress, log, aot, work } = options;
  const closed = options.removed.includes("Interpreter");
  if (closed && aot.closedTransform !== CLOSED_PRELUDES_TRANSFORM)
    throw new AotError("this strict host predates closed application images; rebuild the workload with this SDK");
  progress.step("AOT toolchain");
  const compiler = await resolveAotCompiler(aot, options.compilerPath, log, (done, total) =>
    progress.bytes(done, total),
  );
  progress.detail(`sk-aot (${compiler.source})`);
  progress.step("native image");
  const source = new TextDecoder().decode(options.source);
  const preludes = aot.preludeScripts;
  const native = await compileNative({ compiler: compiler.path, source, work, preludes, closed });
  const problem = removedFeatureProblem(native, source, options.removed, options.workload, preludes);
  if (problem) throw new AotError(problem);
  progress.detail(`${(native.image.length / 2 ** 20).toFixed(1)} MiB`);
  const manifest = {
    ...nativeManifest("application", compiler.info, native, {
      image: options.place.place === "beside" ? options.place.file : "",
      preludeSha256: aot.preludeSha256,
      includesPreludes: !!preludes,
    }),
    place: options.place.place,
  };
  const text = JSON.stringify(manifest, null, 2);
  const manifestPath = join(work, "app.native.json");
  writeFileSync(manifestPath, text);
  return {
    inputs: [{ name: NAMES.NATIVE_MANIFEST, kind: KIND.NATIVE_MANIFEST, data: new TextEncoder().encode(text) }],
    image: native.image,
    manifestPath,
    imageBytes: native.image.length,
    entry: native.entry,
    compiler,
  };
}
