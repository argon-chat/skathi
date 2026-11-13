// Where `vn build` gets the prebuilt bare host it stamps: `cargo build --release -p vn-app
// --features prod` (the whole engine, no app), built once per vue-native version and variant. A
// variant names the engine parts left out (config.ts `OPTIONAL`).
//
// Resolution, first hit wins: 1. `--host <exe>` or `VN_HOST`; 2. in-repo
// target/x86_64-pc-windows-msvc/release/vn-app.exe, when that is a prod host of this variant and
// version; 3. the local cache (`vn host add <exe>`, `vn host build`, or a download from this
// version's GitHub release: host-fetch.ts); 4. an error naming them. Paths for macOS are in platform.ts.
//
// A host for another OS cannot be run to ask what it is: only `--host` / `VN_HOST` names one, its
// format is sniffed, and what it is comes from a `host.json` beside it or is taken on trust, with a warning.
//
// A host and a bundle must come from the same vue-native version (the bundle carries the JS half of
// the `__vn_*` protocol): the CLI refuses a host whose `--vn-host-info` disagrees
// (`--allow-host-mismatch` is for maintainers).

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
import { cargoFeatureArgs, OPTIONAL, partsLeftOut, type Variant } from "./config";
import { bundlePayloadPath, machOArchs } from "./macos-bundle";
import { readPe, RT } from "./pe";
import { currentPlatform, hostsFor, repoHostExe, type Platform } from "./platform";
import { PAYLOAD_NAME } from "./stamp";

export const PKG_DIR = resolve(import.meta.dir, "..");
export const CLI_VERSION: string = JSON.parse(readFileSync(join(PKG_DIR, "package.json"), "utf8")).version;

export const REPO_ROOT: string | null = (() => {
  const root = resolve(PKG_DIR, "../..");
  return existsSync(join(root, "Cargo.toml")) && existsSync(join(root, "crates", "vn-app")) ? root : null;
})();

export const REPO_RELEASE_EXE = REPO_ROOT ? repoHostExe(REPO_ROOT, hostsFor(currentPlatform()), "release") : null;

export interface HostInfo {
  mode: "prod" | "dev";
  vueNative: string;
  crate: string;
  variant: Variant;
  devtools: boolean;
  features: string[];
  jscPin: string;
  payloadFormat: number;
  profile: string;
}

export interface ResolvedHost {
  exe: string;
  info: HostInfo;
  source: "--host" | "VN_HOST" | "repo" | "cache";
  unverified?: string;
}

export class HostError extends Error {}

/** True when `exe` already carries an app (asking it for host info would launch it): a stamped resource on Windows, `Contents/Resources/app.vnpk` on macOS. */
export function isStamped(exe: string): boolean {
  const bundled = bundlePayloadPath(exe);
  if (bundled && existsSync(bundled)) return true;
  try {
    return readPe(exe).resources.some((r) => r.type === RT.RCDATA && r.name === PAYLOAD_NAME);
  } catch {
    return false;
  }
}

function formatProblem(exe: string, platform: Platform): string | null {
  if (platform !== "macos") return null;
  const archs = machOArchs(exe);
  if (!archs) return `${exe} is not a Mach-O executable — a macOS bundle needs a macOS vn-app`;
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
      if (info.mode && info.vueNative && info.variant) {
        return { info, unverified: `${platform} host described by ${sidecar}, not run (this is ${process.platform})` };
      }
    } catch {}
  }
  const off = partsLeftOut(variant);
  const info: HostInfo = {
    mode: "prod",
    vueNative: CLI_VERSION,
    crate: "unknown",
    variant,
    devtools: !off.includes("devtools"),
    features: ["prod", ...OPTIONAL.filter((p) => p !== "devtools" && !off.includes(p))],
    jscPin: "unknown",
    payloadFormat: 1,
    profile: "unknown",
  };
  return {
    info,
    unverified:
      `a ${platform} host cannot be run on ${process.platform} to check it — ${exe} is taken to be a prod ${variant} host ` +
      `of vue-native ${CLI_VERSION}. Put its \`--vn-host-info\` output in ${sidecar} to have it checked.`,
  };
}

export function hostInfo(exe: string): HostInfo {
  if (!existsSync(exe)) throw new HostError(`${exe} does not exist`);
  if (isStamped(exe)) {
    throw new HostError(`${exe} is a packaged app, not a bare host (it already carries a VNPAYLOAD)`);
  }
  const r = spawnSync(exe, ["--vn-host-info"], { encoding: "utf8", timeout: 30_000, windowsHide: true });
  if (r.error) throw new HostError(`${exe} could not be run: ${r.error.message}`);
  const line = (r.stdout ?? "").trim().split(/\r?\n/).pop() ?? "";
  try {
    const info = JSON.parse(line) as HostInfo;
    if (!info.mode || !info.vueNative) throw new Error("missing fields");
    return info;
  } catch {
    throw new HostError(
      `${exe} did not answer --vn-host-info (exit ${r.status}). Hosts older than this CLI do not know the flag — ` +
        `rebuild it.\n${(r.stderr ?? "").trim().split(/\r?\n/).slice(-5).join("\n")}`,
    );
  }
}

export function cacheRoot(platform: Platform = currentPlatform()): string {
  return hostsFor(platform).cacheRoot();
}

export const cachedHost = (version: string, variant: Variant, platform: Platform = currentPlatform()) =>
  join(cacheRoot(platform), version, variant, hostsFor(platform).cachedFile);

export function checkHost(exe: string, info: HostInfo, variant: Variant | null, allowMismatch: boolean): string | null {
  if (info.mode !== "prod") return `${exe} is a ${info.mode} host — vn build needs one built with --features prod`;
  if (variant && info.variant !== variant) return `${exe} is the ${info.variant} variant, not ${variant}`;
  if (!partsLeftOut(variant ?? info.variant).includes("webgpu") && !info.features?.includes("webgpu")) {
    return `${exe} lacks native WebGPU required by ${variant} — rebuild or replace the cached host`;
  }
  if (info.vueNative !== CLI_VERSION && !allowMismatch) {
    return (
      `${exe} is vue-native ${info.vueNative}, this CLI is ${CLI_VERSION} — the bundle carries the JS half of the ` +
      `host protocol, so the two must match (--allow-host-mismatch overrides, for maintainers)`
    );
  }
  return null;
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
  visit(join(REPO_ROOT, "packages", "vue-native", "src"));
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
}

export function resolveHost(options: ResolveOptions): ResolvedHost {
  const log = options.log ?? (() => {});
  const allow = options.allowMismatch ?? false;
  const platform = options.platform ?? currentPlatform();
  const cross = platform !== currentPlatform();
  const explicit = options.host ?? process.env.VN_HOST;
  if (explicit) {
    const exe = resolve(explicit);
    const source = options.host ? "--host" : "VN_HOST";
    if (cross) {
      if (!existsSync(exe)) throw new HostError(`${exe} does not exist`);
      if (isStamped(exe))
        throw new HostError(`${exe} is a packaged app, not a bare host (its bundle already carries an app.vnpk)`);
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
      `a ${platform} build on ${process.platform} needs the host named: --host <path to a ${platform} vn-app> ` +
        `(or VN_HOST). This machine's checkout and cache hold ${currentPlatform()} hosts.`,
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
            `[vn] warning: ${REPO_RELEASE_EXE} is older than the sources it is built from — ` +
              "`bun packages/vue-native/cli/vn.ts host build` rebuilds it",
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

  throw new HostError(
    `no ${options.variant} vue-native ${CLI_VERSION} host found.\n` +
      tried.map((t) => `  - ${t}`).join("\n") +
      "\nSupply one of:\n" +
      `  ${`--host <${hostsFor(platform).exeFile}>`.padEnd(21)}(or VN_HOST=<exe>)   a prod host built from this version\n` +
      `  vn host fetch                              download it from the vue-native ${CLI_VERSION} release\n` +
      "  vn host add <exe>                          import one into the local cache\n" +
      (REPO_ROOT
        ? options.variant === "standard" || options.variant === "no-devtools"
          ? "  vn host build                              build both variants from this checkout (cargo)\n"
          : `  vn host build --variant ${options.variant}   build this one from this checkout (cargo)\n`
        : ""),
  );
}

export function addHost(exe: string, log: (line: string) => void = () => {}): { path: string; info: HostInfo } {
  const src = resolve(exe);
  const info = hostInfo(src);
  const problem = checkHost(src, info, null, true);
  if (problem) throw new HostError(problem);
  const dest = cachedHost(info.vueNative, info.variant);
  mkdirSync(dirname(dest), { recursive: true });
  const tmp = `${dest}.partial`;
  copyFileSync(src, tmp);
  // A POSIX host must stay runnable whatever mode the source had.
  if (process.platform !== "win32") chmodSync(tmp, 0o755);
  rmSync(dest, { force: true });
  renameSync(tmp, dest);
  writeFileSync(join(dirname(dest), "host.json"), JSON.stringify(info, null, 2) + "\n");
  log(`[vn] cached ${info.variant} host ${info.vueNative} (${info.profile}) → ${dest}`);
  return { path: dest, info };
}

function cargo(): string {
  const which = Bun.which("cargo");
  if (which) return which;
  const home = process.env.CARGO_HOME ?? join(process.env.USERPROFILE ?? homedir(), ".cargo");
  const exe = join(home, "bin", process.platform === "win32" ? "cargo.exe" : "cargo");
  if (existsSync(exe)) return exe;
  throw new HostError("cargo is not on PATH — building a host needs the Rust toolchain (vn build does not)");
}

export const hostCargoArgs = (variant: Variant): string[] => [
  "build",
  "--release",
  "-p",
  "vn-app",
  ...hostsFor(currentPlatform()).cargoTarget,
  ...cargoFeatureArgs(variant),
];

/** MAINTAINERS ONLY, in a checkout: cargo-builds the bare host for `variant` into the checkout's target/; returns its path. */
export function cargoBuildHost(
  variant: Variant,
  log: (line: string) => void = console.log,
): { exe: string; ms: number } {
  if (!REPO_ROOT || !REPO_RELEASE_EXE) {
    throw new HostError(
      "vn host build runs from a vue-native checkout; elsewhere, use vn host fetch or vn host add <exe>",
    );
  }
  const args = hostCargoArgs(variant);
  log(`[vn] cargo ${args.join(" ")}`);
  const t0 = performance.now();
  const r = spawnSync(cargo(), args, { cwd: REPO_ROOT, stdio: "inherit" });
  if (r.status !== 0) throw new HostError(`cargo ${args.join(" ")} failed (exit ${r.status})`);
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
