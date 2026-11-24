// `sk package`: `sk publish` (unless --no-build or --from), then every format the platform lists.

import { existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { buildApp } from "../build-app";
import { ConfigError, loadAppConfig, type AppConfig } from "../config";
import { currentPlatform, type Platform } from "../platform";
import { paint, quietProgress, type Progress } from "../term";
import { packageProblems } from "./config";
import { iconAt, iconSources } from "./icons";
import { FORMATS } from "./index";
import {
  PLATFORM_FORMATS,
  type Artifact,
  type FormatName,
  type PackageConfig,
  type PackageContext,
  type Problem,
  type SignHook,
} from "./types";

export interface PackageOptions {
  target: string;
  configPath?: string | undefined;
  platform?: Platform | undefined;
  /** Over `package.<platform>.formats`. */
  formats?: string[] | undefined;
  out?: string | undefined;
  /** False: package what the last `sk publish` wrote. */
  build?: boolean;
  /** Package this folder instead of building: an embedder's output. */
  from?: string | undefined;
  /** The program inside `from`, relative to it. */
  exe?: string | undefined;
  aot?: boolean | undefined;
  sign?: SignHook | null;
  /** The system sk runs on; default: this one. */
  host?: Platform;
  progress?: Progress;
  color?: boolean;
}

export interface PackageResult {
  config: PackageConfig;
  platform: Platform;
  artifacts: Artifact[];
  warnings: Problem[];
}

function chosenFormats(list: readonly string[], platform: Platform): FormatName[] {
  const allowed = PLATFORM_FORMATS[platform];
  return list.map((raw) => {
    const name = allowed.find((f) => f === raw.trim());
    if (!name) {
      throw new ConfigError(
        `--format: ${JSON.stringify(raw)} is not a ${platform} format; ${platform} has ${allowed.join(", ")}`,
      );
    }
    if (!FORMATS[name]) throw new ConfigError(`--format: ${name} is not available in this version of sk`);
    return name;
  });
}

const describe = (p: Problem) => `${p.key}: ${p.message}`;

function locate(app: AppConfig, platform: Platform, options: PackageOptions): { buildDir: string; exe: string } {
  const fromDir = options.from === undefined ? join(app.appRoot, "dist", "release") : resolve(options.from);
  if (!existsSync(fromDir) || !statSync(fromDir).isDirectory()) {
    throw new ConfigError(
      options.from === undefined
        ? `${fromDir} does not exist: run sk publish first, or sk package without --no-build`
        : `--from ${options.from}: not a directory`,
    );
  }
  const name =
    options.exe ??
    (platform === "windows" ? `${app.exeName}.exe` : platform === "macos" ? `${app.macos.exeName}.app` : app.exeName);
  const exe = resolve(fromDir, name);
  if (!existsSync(exe)) {
    throw new ConfigError(
      `${exe} does not exist: ${options.exe === undefined ? "pass --exe <file> to name the program in the folder" : "check --exe"}`,
    );
  }
  return { buildDir: fromDir, exe };
}

export async function packageApp(options: PackageOptions): Promise<PackageResult> {
  const progress = options.progress ?? quietProgress;
  const platform = options.platform ?? currentPlatform();
  const app = loadAppConfig(options.target, { configPath: options.configPath, platform });
  const only = options.formats?.length ? chosenFormats(options.formats, platform) : undefined;
  const { config, problems, formats } = packageProblems(app, [platform], only);
  const errors = problems.filter((p) => p.severity === "error");
  if (errors.length) {
    throw new ConfigError(
      `sk.config.json "package" has ${errors.length} problem${errors.length === 1 ? "" : "s"}:\n${errors.map((p) => `  ${describe(p)}`).join("\n")}`,
    );
  }
  if (!formats.length) {
    throw new ConfigError(
      `nothing to package for ${platform}: list formats in "package.${platform}.formats" (${PLATFORM_FORMATS[platform].join(", ")})`,
    );
  }
  const host = options.host ?? currentPlatform();
  const elsewhere = formats.filter((f) => !(f.hosts ?? [f.platform]).includes(host));
  if (elsewhere.length) {
    throw new ConfigError(
      elsewhere
        .map((f) => `a ${f.name} package is written on ${(f.hosts ?? [f.platform]).join(" or ")}, not on ${host}`)
        .join("; ") + `: run sk package --platform ${platform} there`,
    );
  }
  const warnings = problems.filter((p) => p.severity === "warning");
  for (const w of warnings) progress.warn(describe(w));

  let buildDir: string;
  let exe: string;
  let appBundle: string | null = null;
  if (options.from !== undefined || options.build === false) {
    ({ buildDir, exe } = locate(app, platform, options));
    if (platform === "macos") {
      appBundle = exe;
      exe = join(exe, "Contents", "MacOS", app.macos.exeName);
    }
  } else {
    const result = await buildApp({
      target: options.target,
      configPath: options.configPath,
      platform,
      mode: "release",
      aot: options.aot,
      color: options.color ?? false,
      progress,
    });
    if (!result.exe) throw new ConfigError("sk publish wrote no program to package");
    exe = result.exe;
    appBundle = result.bundle?.app ?? null;
    buildDir = dirname(appBundle ?? exe);
  }

  const outDir = resolve(options.out ?? config.outDir);
  const workDir = join(outDir, ".sk-package");
  rmSync(workDir, { recursive: true, force: true });
  mkdirSync(workDir, { recursive: true });
  const sources = iconSources(config.icons);
  const ctx: PackageContext = {
    appRoot: app.appRoot,
    app,
    config,
    platform,
    arch: process.arch,
    buildDir,
    exe,
    appBundle,
    outDir,
    workDir,
    log: { info: (line) => progress.info(line), warn: (line) => progress.warn(line) },
    icon: (size) => iconAt(sources, size),
    sign: options.sign ?? null,
  };
  const artifacts: Artifact[] = [];
  try {
    for (const format of formats) {
      progress.step(`${format.name} package`);
      const written = await format.write(ctx);
      progress.detail(written.map((a) => a.path).join(", "));
      artifacts.push(...written);
    }
  } finally {
    rmSync(workDir, { recursive: true, force: true });
  }
  return { config, platform, artifacts, warnings };
}

export function packageSummary(result: PackageResult, color = false): string {
  const p = paint(color);
  const lines = result.artifacts.map(
    (a) => `${p.green(a.kind.padEnd(9))} ${a.path}${a.kind === "installer" && !a.signed ? p.dim(" (unsigned)") : ""}`,
  );
  return lines.join("\n");
}
