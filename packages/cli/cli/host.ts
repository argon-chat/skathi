// Where `sk build` gets the prebuilt bare host it stamps: `cargo build --release -p sk-app
// --features prod,icu-root` (the whole engine, no app, the root ICU locale), built once per Skathi version and
// variant: `standard`, `no-devtools` with the devtools compiled out, or (Windows x64) `aot-nojit`
// without devtools or the JIT, which runs only apps built with `--aot`.
//
// Resolution, first hit wins: 1. `--workload <exe>` or `SK_HOST`; 2. in-repo
// target/x86_64-pc-windows-msvc/release/sk-app.exe, when that is a prod host of this variant and
// version; 3. the local cache (`sk workload add <exe>`, `sk workload build`, or a download from this
// version's GitHub release: host-fetch.ts); 4. an error naming them. Paths per OS are in platform.ts.
//
// A host for another OS cannot be run to ask what it is: only `--workload` / `SK_HOST` names one, its
// format is sniffed, and what it is comes from a `host.json` beside it or is taken on trust, with a warning.
//
// A host and a bundle must come from the same Skathi version (the bundle carries the JS half of
// the `__sk_*` protocol): the CLI refuses a host whose `--sk-host-info` disagrees
// (`--allow-workload-mismatch` is for maintainers).

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import type { CompilerInfo } from "../aot/pin";
import { cargoFeatureArgs, ENGINE_FEATURES, NO_JIT_VARIANT, variantList, VARIANTS, type Variant } from "./config";
import { appendedPayloadOf, elfMachineOf } from "./elf";
import { bundlePayloadPath, machOArchs } from "./macos-bundle";
import { readPe, RT } from "./pe";
import { currentPlatform, hostsFor, repoHostExe, type Platform } from "./platform";
import { TEMPLATE_NAME } from "./image-sections";
import { MACHO_TEMPLATE_NAME } from "./image-macho";
import { PAYLOAD_NAME } from "./stamp";

export const PKG_DIR = resolve(import.meta.dir, "..");
const PKG: { version: string; engines: { bun: string } } = JSON.parse(
  readFileSync(join(PKG_DIR, "package.json"), "utf8"),
);
export const CLI_VERSION = PKG.version;
export const BUN_RANGE = PKG.engines.bun;

export const REPO_ROOT: string | null = (() => {
  const root = resolve(PKG_DIR, "../..");
  return existsSync(join(root, "Cargo.toml")) && existsSync(join(root, "crates", "sk-app")) ? root : null;
})();

export const REPO_RELEASE_EXE = REPO_ROOT ? repoHostExe(REPO_ROOT, hostsFor(currentPlatform()), "release") : null;

export interface HostInfo {
  mode: "prod" | "dev";
  skathi: string;
  crate: string;
  variant: Variant;
  devtools: boolean;
  features: string[];
  jscPin: string;
  payloadFormat: number;
  profile: string;
  /** What a dev host reloads in place from `sk dev` besides JS modules; absent on older hosts. */
  devReload?: string[];
  /** A host built with the native AOT loader: what an app image must be compiled against. */
  aot?: HostAot | null;
  /** The ICU data it links; absent on older hosts, which link every locale. */
  icu?: HostIcu;
}

export interface HostIcu {
  /** `root`: the root locale only, the rest ships beside the app as icu.skpk (`sk.config.json` "intl"). */
  linked: "full" | "root" | "system";
  /** `icudt79l`; null with the system ICU. */
  package: string | null;
}

/** Where a host that links only the root ICU locale keeps the full data (`<package>.dat` beside it); null for any other host. */
export function icuDataPath(host: { exe: string; info: HostInfo }): string | null {
  const icu = host.info.icu;
  return icu?.linked === "root" && icu.package ? join(dirname(host.exe), `${icu.package}.dat`) : null;
}

/** The empty native image container beside a host (scripts/release-hosts.ts, `cargoBuildHost`). */
export const imageTemplatePath = (host: { exe: string }, platform = currentPlatform()): string =>
  join(dirname(host.exe), platform === "macos" ? MACHO_TEMPLATE_NAME : TEMPLATE_NAME);

/** The native inspector container a host with devtools expects beside itself and beside the apps it runs; null for a host that embeds its inspector image. */
export function inspectorContainerPath(host: { exe: string; info: HostInfo }): string | null {
  const name = host.info.aot?.inspectorContainer;
  return name ? join(dirname(host.exe), name) : null;
}

export interface HostAot extends CompilerInfo {
  /** The file name of the native inspector container beside the host. */
  inspectorContainer?: string;
  closedTransform?: string;
  transform: string;
  preludeSha256: string;
  preludeScripts?: readonly [string, string];
}

export interface ResolvedHost {
  exe: string;
  info: HostInfo;
  source: "--workload" | "SK_HOST" | "repo" | "cache";
  unverified?: string;
}

export class HostError extends Error {}

/** True when `exe` already carries an app (asking it for host info would launch it): a stamped resource on Windows, `Contents/Resources/app.skpk` on macOS, an appended payload on Linux. */
export function isStamped(exe: string): boolean {
  const bundled = bundlePayloadPath(exe);
  if (bundled && existsSync(bundled)) return true;
  if (appendedPayloadOf(exe)) return true;
  try {
    return readPe(exe).resources.some((r) => r.type === RT.RCDATA && r.name === PAYLOAD_NAME);
  } catch {
    return false;
  }
}

function formatProblem(exe: string, platform: Platform): string | null {
  if (platform === "windows") return null;
  if (platform === "linux") {
    const machine = elfMachineOf(exe);
    if (!machine) return `${exe} is not an ELF executable — a Linux build needs a Linux workload (sk-app)`;
    if (machine !== "x86_64" && machine !== "aarch64") return `${exe} is an ELF for ${machine}, not x86_64 or aarch64`;
    return null;
  }
  const archs = machOArchs(exe);
  if (!archs) return `${exe} is not a Mach-O executable — a macOS bundle needs a macOS workload (sk-app)`;
  if (!archs.some((a) => a === "arm64" || a === "x86_64"))
    return `${exe} is a Mach-O for ${archs.join(", ")}, not arm64 or x86_64`;
  return null;
}

export function crossHostInfo(
  exe: string,
  variant: Variant,
  platform: Platform,
): { info: HostInfo; unverified: string } {
  const problem = formatProblem(exe, platform);
  if (problem) throw new HostError(problem);
  const sidecar = join(dirname(exe), "host.json");
  if (existsSync(sidecar)) {
    try {
      const info = JSON.parse(readFileSync(sidecar, "utf8")) as HostInfo;
      if (info.mode && info.skathi && info.variant) {
        return {
          info,
          unverified: `${platform} workload described by ${sidecar}, not run (this is ${process.platform})`,
        };
      }
    } catch {}
  }
  const devtools = variant !== "no-devtools" && variant !== NO_JIT_VARIANT;
  const info: HostInfo = {
    mode: "prod",
    skathi: CLI_VERSION,
    crate: "unknown",
    variant,
    devtools,
    features: ["prod", ...(devtools ? [] : ["no-devtools"]), ...ENGINE_FEATURES],
    jscPin: "unknown",
    payloadFormat: 1,
    profile: "unknown",
  };
  return {
    info,
    unverified:
      `a ${platform} workload cannot be run on ${process.platform} to check it — ${exe} is taken to be a prod ${variant} workload ` +
      `of Skathi ${CLI_VERSION}. Put its \`--sk-host-info\` output in ${sidecar} to have it checked.`,
  };
}

export function hostInfo(exe: string): HostInfo {
  if (!existsSync(exe)) throw new HostError(`${exe} does not exist`);
  if (isStamped(exe)) {
    throw new HostError(`${exe} is a packaged app, not a bare workload (it already carries a SKPAYLOAD)`);
  }
  const r = spawnSync(exe, ["--sk-host-info"], { encoding: "utf8", timeout: 30_000, windowsHide: true });
  if (r.error) throw new HostError(`${exe} could not be run: ${r.error.message}`);
  const line = (r.stdout ?? "").trim().split(/\r?\n/).pop() ?? "";
  try {
    const info = JSON.parse(line) as HostInfo;
    if (!info.mode || !info.skathi) throw new Error("missing fields");
    return info;
  } catch {
    throw new HostError(
      `${exe} did not answer --sk-host-info (exit ${r.status}). Workloads older than this CLI do not know the flag — ` +
        `rebuild it.\n${(r.stderr ?? "").trim().split(/\r?\n/).slice(-5).join("\n")}`,
    );
  }
}

export function cacheRoot(platform: Platform = currentPlatform()): string {
  return hostsFor(platform).cacheRoot();
}

export const cachedHost = (version: string, variant: Variant, platform: Platform = currentPlatform()) =>
  join(cacheRoot(platform), version, variant, hostsFor(platform).cachedFile);

/** The host `sk dev` runs: every part, built without `prod`, so it reads a dev dist and hot-reloads. Cached and released beside the prod variants under this name. */
export const DEV_HOST: Variant = "dev";

export function checkHost(exe: string, info: HostInfo, variant: Variant | null, allowMismatch: boolean): string | null {
  const mismatch = versionMismatch(exe, info, allowMismatch);
  if (variant === DEV_HOST) {
    return info.mode === "dev"
      ? mismatch
      : `${exe} is a ${info.mode} workload — sk dev needs one built without --features prod`;
  }
  if (info.mode !== "prod") return `${exe} is a ${info.mode} workload — sk build needs one built with --features prod`;
  if (variant && info.variant !== variant) return `${exe} is the ${info.variant} variant, not ${variant}`;
  if (!VARIANTS.includes(info.variant)) {
    return `${exe} is the ${info.variant} variant, which leaves engine parts out — sk build stamps only ${variantList(VARIANTS)} workloads`;
  }
  const missing = ENGINE_FEATURES.filter((f) => !info.features?.includes(f));
  if (missing.length) return `${exe} lacks ${missing.join(", ")} — rebuild or replace the cached workload`;
  return mismatch;
}

function versionMismatch(exe: string, info: HostInfo, allowMismatch: boolean): string | null {
  return info.skathi !== CLI_VERSION && !allowMismatch
    ? `${exe} is Skathi ${info.skathi}, this CLI is ${CLI_VERSION} — the bundle carries the JS half of the ` +
        `workload protocol, so the two must match (--allow-workload-mismatch overrides, for maintainers)`
    : null;
}

export function resolveEmbedderHost(path: string, allowMismatch = false): ResolvedHost {
  const exe = resolve(path);
  const info = hostInfo(exe);
  if (info.mode !== "prod") throw new HostError(`${exe} must be built with the Skathi prod feature`);
  const mismatch = versionMismatch(exe, info, allowMismatch);
  if (mismatch) throw new HostError(mismatch);
  return { exe, info, source: "--workload" };
}

function newestHostSource(): number {
  if (!REPO_ROOT) return 0;
  let newest = 0;
  const visit = (dir: string) => {
    let names: string[];
    try {
      names = readdirSync(dir);
    } catch {
      return;
    }
    for (const name of names) {
      if (name === "node_modules" || name === "target" || name.startsWith(".")) continue;
      const abs = join(dir, name);
      let st;
      try {
        st = statSync(abs);
      } catch {
        continue;
      }
      if (st.isDirectory()) visit(abs);
      else if (/\.(rs|toml|ts|vue|json)$/.test(name)) newest = Math.max(newest, st.mtimeMs);
    }
  };
  visit(join(REPO_ROOT, "crates"));
  visit(join(REPO_ROOT, "apps", "devtools"));
  visit(join(REPO_ROOT, "packages", "runtime", "src"));
  for (const f of ["Cargo.toml", "Cargo.lock"]) {
    try {
      newest = Math.max(newest, statSync(join(REPO_ROOT, f)).mtimeMs);
    } catch {}
  }
  return newest;
}

export interface ResolveOptions {
  host?: string | undefined;
  variant: Variant;
  allowMismatch?: boolean | undefined;
  platform?: Platform | undefined;
  log?: (line: string) => void;
  onBytes?: ((done: number, total: number) => void) | undefined;
}

export function resolveHost(options: ResolveOptions): ResolvedHost {
  const log = options.log ?? (() => {});
  const allow = options.allowMismatch ?? false;
  const platform = options.platform ?? currentPlatform();
  const cross = platform !== currentPlatform();
  const explicit = options.host ?? process.env.SK_HOST;
  if (explicit) {
    const exe = resolve(explicit);
    const source = options.host ? "--workload" : "SK_HOST";
    if (cross) {
      if (!existsSync(exe)) throw new HostError(`${exe} does not exist`);
      if (isStamped(exe))
        throw new HostError(`${exe} is a packaged app, not a bare workload (it already carries an app payload)`);
      const { info, unverified } = crossHostInfo(exe, options.variant, platform);
      const problem = checkHost(exe, info, options.variant, allow);
      if (problem) throw new HostError(problem);
      return { exe, info, source, unverified };
    }
    const format = existsSync(exe) ? formatProblem(exe, platform) : null;
    if (format) throw new HostError(format);
    const info = hostInfo(exe);
    const problem = checkHost(exe, info, options.variant, allow);
    if (problem) throw new HostError(problem);
    return { exe, info, source };
  }
  if (cross) {
    throw new HostError(
      `a ${platform} build on ${process.platform} needs the workload named: --workload <path to a ${platform} sk-app> ` +
        `(or SK_HOST). This machine's checkout and cache hold ${currentPlatform()} workloads.`,
    );
  }

  const tried: string[] = [];
  if (REPO_RELEASE_EXE && existsSync(REPO_RELEASE_EXE)) {
    try {
      const info = hostInfo(REPO_RELEASE_EXE);
      const problem = checkHost(REPO_RELEASE_EXE, info, options.variant, allow);
      if (!problem) {
        if (statSync(REPO_RELEASE_EXE).mtimeMs < newestHostSource()) {
          log(
            `warning: ${REPO_RELEASE_EXE} is older than the sources it is built from — ` +
              "`bun packages/cli/cli/sk.ts workload build` rebuilds it",
          );
        }
        return { exe: REPO_RELEASE_EXE, info, source: "repo" };
      }
      tried.push(`in-repo: ${problem}`);
    } catch (e) {
      tried.push(`in-repo: ${(e as Error).message}`);
    }
  } else if (REPO_RELEASE_EXE) {
    tried.push(`in-repo: ${REPO_RELEASE_EXE} has not been built`);
  }

  const cached = cachedHost(CLI_VERSION, options.variant);
  if (existsSync(cached)) {
    const info = hostInfo(cached);
    const problem = checkHost(cached, info, options.variant, allow);
    if (!problem) return { exe: cached, info, source: "cache" };
    tried.push(`cache: ${problem}`);
  } else {
    tried.push(`cache: ${cached} does not exist`);
  }

  const remedies: [string, string][] = [
    [`--workload <${hostsFor(platform).exeFile}>`, "(or SK_HOST=<exe>) a prod workload built from this version"],
    [`sk workload install ${options.variant}`, `download it from the Skathi ${CLI_VERSION} release`],
    ["sk workload add <exe>", "import one into the local cache"],
    ...(REPO_ROOT
      ? [[`sk workload build ${options.variant}`, "build it from this checkout (cargo)"] as [string, string]]
      : []),
  ];
  const width = Math.max(...remedies.map(([how]) => how.length));
  throw new HostError(
    `no ${options.variant} Skathi ${CLI_VERSION} workload found.\n` +
      tried.map((t) => `  - ${t}`).join("\n") +
      "\nSupply one of:\n" +
      remedies.map(([how, what]) => `  ${how.padEnd(width)}  ${what}\n`).join(""),
  );
}

export function addHost(exe: string, log: (line: string) => void = () => {}): { path: string; info: HostInfo } {
  const src = resolve(exe);
  const info = hostInfo(src);
  const problem = checkHost(src, info, null, true);
  if (problem) throw new HostError(problem);
  const dest = cachedHost(info.skathi, info.variant);
  mkdirSync(dirname(dest), { recursive: true });
  const tmp = `${dest}.partial`;
  copyFileSync(src, tmp);
  // A POSIX host must stay runnable whatever mode the source had.
  if (process.platform !== "win32") chmodSync(tmp, 0o755);
  rmSync(dest, { force: true });
  renameSync(tmp, dest);
  const icu = icuDataPath({ exe: src, info });
  if (icu && existsSync(icu)) copyFileSync(icu, icuDataPath({ exe: dest, info })!);
  for (const [from, to] of [
    [imageTemplatePath({ exe: src }), imageTemplatePath({ exe: dest })],
    [inspectorContainerPath({ exe: src, info }), inspectorContainerPath({ exe: dest, info })],
  ] as const) {
    if (from && to && existsSync(from)) copyFileSync(from, to);
  }
  writeFileSync(join(dirname(dest), "host.json"), JSON.stringify(info, null, 2) + "\n");
  log(`cached ${info.variant} workload ${info.skathi} (${info.profile}) → ${dest}`);
  return { path: dest, info };
}

function cargo(): string {
  const which = Bun.which("cargo");
  if (which) return which;
  const home = process.env.CARGO_HOME ?? join(process.env.USERPROFILE ?? homedir(), ".cargo");
  const exe = join(home, "bin", process.platform === "win32" ? "cargo.exe" : "cargo");
  if (existsSync(exe)) return exe;
  throw new HostError("cargo is not on PATH — building a workload needs the Rust toolchain (sk build does not)");
}

/** The cargo command line for `variant`, plus `extra` features (`native-inspector` for a release host). */
export function hostCargoArgs(variant: Variant, extra: string[] = []): string[] {
  const args = variant === DEV_HOST ? [] : cargoFeatureArgs(variant);
  const at = args.indexOf("--features");
  if (extra.length) {
    if (at < 0) args.push("--features", extra.join(","));
    else args[at + 1] = [args[at + 1], ...extra].join(",");
  }
  const platform = currentPlatform();
  // rust-lld ships with rustc; sk-app/build.rs gives it /OPT:SAFEICF, which link.exe lacks (~5 MB).
  const linker = platform === "windows" ? ["--config", 'target.x86_64-pc-windows-msvc.linker="rust-lld"'] : [];
  return ["build", "--release", "-p", "sk-app", ...hostsFor(platform).cargoTarget, ...linker, ...args];
}

export interface CargoExtras {
  features?: string[];
  env?: Record<string, string>;
}

/** MAINTAINERS ONLY, in a checkout: cargo-builds the bare host for `variant` into the checkout's target/; returns its path. */
export function cargoBuildHost(
  variant: Variant,
  log: (line: string) => void = console.log,
  extras: CargoExtras = {},
): { exe: string; ms: number } {
  if (!REPO_ROOT || !REPO_RELEASE_EXE) {
    throw new HostError(
      "sk workload build runs from a Skathi checkout; elsewhere, use sk workload install or sk workload add <exe>",
    );
  }
  const args = hostCargoArgs(variant, extras.features);
  log(`cargo ${args.join(" ")}`);
  const t0 = performance.now();
  const r = spawnSync(cargo(), args, {
    cwd: REPO_ROOT,
    stdio: "inherit",
    env: { ...process.env, ...extras.env },
  });
  if (r.status !== 0) throw new HostError(`cargo ${args.join(" ")} failed (exit ${r.status})`);
  const platform = currentPlatform();
  if (platform === "windows" || (platform === "macos" && extras.features?.includes("native-aot"))) {
    // Its own workspace (it needs panic = "abort"), built into the same folder as the host.
    const template = [
      "build",
      "--release",
      "--manifest-path",
      join(REPO_ROOT, "crates", "sk-image", "Cargo.toml"),
      "--target",
      hostsFor(platform).triple,
      "--target-dir",
      dirname(dirname(dirname(REPO_RELEASE_EXE))),
    ];
    log(`cargo ${template.join(" ")}`);
    const t = spawnSync(cargo(), template, {
      cwd: REPO_ROOT,
      stdio: "inherit",
      env: { ...process.env, ...extras.env },
    });
    if (t.status !== 0) throw new HostError(`cargo ${template.join(" ")} failed (exit ${t.status})`);
  }
  return { exe: REPO_RELEASE_EXE, ms: Math.round(performance.now() - t0) };
}

/** MAINTAINERS ONLY, in a checkout: builds the bare host for `variant` and imports it into the cache. */
export function buildHost(
  variant: Variant,
  log: (line: string) => void = console.log,
): { path: string; info: HostInfo; ms: number } {
  const { exe, ms } = cargoBuildHost(variant, log);
  const added = addHost(exe, log);
  if (added.info.variant !== variant) {
    throw new HostError(`built the ${variant} variant but the exe reports ${added.info.variant}`);
  }
  return { ...added, ms };
}
