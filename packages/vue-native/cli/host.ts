// Where `vn build` gets the prebuilt BARE host it stamps.
//
// A bare host is `cargo build --release -p vn-app --features prod` (or
// `prod,no-devtools`): the whole engine, no app. It is built once per
// vue-native version and VARIANT, never per app — the app arrives later as a
// payload (research/reports/m25/packaging-design.md §4.9). A variant names the
// engine parts left out (config.ts `OPTIONAL`): `no-voice` is
// `--no-default-features --features prod,audio,clipboard,storages,indexeddb`.
//
// Resolution, first hit wins:
//   1. `--host <exe>` or `VN_HOST`;
//   2. IN-REPO: target/x86_64-pc-windows-msvc/release/vn-app.exe, when this CLI
//      runs from a vue-native checkout and that exe says it is a prod host of
//      the requested variant and of THIS version;
//   3. the local cache, %LOCALAPPDATA%\vue-native\hosts\<version>\<variant>\,
//      filled by `vn host add <exe>` (or `vn host build`, in a checkout);
//   4. otherwise an error naming the three.
//
// Fetching a host from a release channel is deliberately not here: where hosts
// are published is an open owner decision (design Q1, deferred with
// distribution). Nothing in this file uploads or downloads anything.
//
// Version coupling: the bundle carries `src/runtime.ts`, the JS half of the
// `__vn_*` host protocol, so a host and a bundle must come from the same
// vue-native version. The CLI refuses a host whose `--vn-host-info` disagrees
// (`--allow-host-mismatch` exists for maintainers).

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { cargoFeatureArgs, type Variant } from "./config";
import { readPe, RT } from "./pe";
import { PAYLOAD_NAME } from "./stamp";

export const PKG_DIR = resolve(import.meta.dir, "..");
export const CLI_VERSION: string = JSON.parse(readFileSync(join(PKG_DIR, "package.json"), "utf8")).version;

/** The checkout this CLI lives in, or `null` when it runs from somewhere else. */
export const REPO_ROOT: string | null = (() => {
  const root = resolve(PKG_DIR, "../..");
  return existsSync(join(root, "Cargo.toml")) && existsSync(join(root, "crates", "vn-app")) ? root : null;
})();

export const REPO_RELEASE_EXE = REPO_ROOT ? join(REPO_ROOT, "target", "x86_64-pc-windows-msvc", "release", "vn-app.exe") : null;

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
}

export class HostError extends Error {}

/** True when `exe` already carries an app — asking it for host info would launch that app. */
export function isStamped(exe: string): boolean {
  try {
    return readPe(exe).resources.some((r) => r.type === RT.RCDATA && r.name === PAYLOAD_NAME);
  } catch {
    return false;
  }
}

/** `<exe> --vn-host-info`: the host describes itself and exits before any window exists. */
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

export function cacheRoot(): string {
  const base = process.env.LOCALAPPDATA || join(homedir(), "AppData", "Local");
  return join(base, "vue-native", "hosts");
}

export const cachedHost = (version: string, variant: Variant) => join(cacheRoot(), version, variant, "vn-host.exe");

function check(exe: string, info: HostInfo, variant: Variant | null, allowMismatch: boolean): string | null {
  if (info.mode !== "prod") return `${exe} is a ${info.mode} host — vn build needs one built with --features prod`;
  if (variant && info.variant !== variant) return `${exe} is the ${info.variant} variant, not ${variant}`;
  if (info.vueNative !== CLI_VERSION && !allowMismatch) {
    return (
      `${exe} is vue-native ${info.vueNative}, this CLI is ${CLI_VERSION} — the bundle carries the JS half of the ` +
      `host protocol, so the two must match (--allow-host-mismatch overrides, for maintainers)`
    );
  }
  return null;
}

/** Newest source mtime that a host build depends on — for the in-repo staleness warning. */
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
  host?: string;
  variant: Variant;
  allowMismatch?: boolean;
  log?: (line: string) => void;
}

export function resolveHost(options: ResolveOptions): ResolvedHost {
  const log = options.log ?? (() => {});
  const allow = options.allowMismatch ?? false;
  const explicit = options.host ?? process.env.VN_HOST;
  if (explicit) {
    const exe = resolve(explicit);
    const info = hostInfo(exe);
    const problem = check(exe, info, options.variant, allow);
    if (problem) throw new HostError(problem);
    return { exe, info, source: options.host ? "--host" : "VN_HOST" };
  }

  const tried: string[] = [];
  if (REPO_RELEASE_EXE && existsSync(REPO_RELEASE_EXE)) {
    try {
      const info = hostInfo(REPO_RELEASE_EXE);
      const problem = check(REPO_RELEASE_EXE, info, options.variant, allow);
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
    const problem = check(cached, info, options.variant, allow);
    if (!problem) return { exe: cached, info, source: "cache" };
    tried.push(`cache: ${problem}`);
  } else {
    tried.push(`cache: ${cached} does not exist`);
  }

  throw new HostError(
    `no ${options.variant} vue-native ${CLI_VERSION} host found.\n` +
      tried.map((t) => `  - ${t}`).join("\n") +
      "\nSupply one of:\n" +
      "  --host <vn-app.exe>  (or VN_HOST=<exe>)   a prod host built from this version\n" +
      "  vn host add <exe>                          import one into the local cache\n" +
      (REPO_ROOT
        ? options.variant === "standard" || options.variant === "no-devtools"
          ? "  vn host build                              build both variants from this checkout (cargo)\n"
          : `  vn host build --variant ${options.variant}   build this one from this checkout (cargo)\n`
        : ""),
  );
}

/** Imports a verified host into the cache. Returns its cached path. */
export function addHost(exe: string, log: (line: string) => void = () => {}): { path: string; info: HostInfo } {
  const src = resolve(exe);
  const info = hostInfo(src);
  const problem = check(src, info, null, true);
  if (problem) throw new HostError(problem);
  const dest = cachedHost(info.vueNative, info.variant);
  mkdirSync(dirname(dest), { recursive: true });
  const tmp = `${dest}.partial`;
  copyFileSync(src, tmp);
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

/**
 * MAINTAINERS ONLY, and only in a checkout: builds the bare host for `variant`
 * and imports it into the cache. Both variants are the same output path in
 * `target/`, which is why each one is copied out as soon as it exists.
 */
export function buildHost(variant: Variant, log: (line: string) => void = console.log): { path: string; info: HostInfo; ms: number } {
  if (!REPO_ROOT || !REPO_RELEASE_EXE) {
    throw new HostError("vn host build runs from a vue-native checkout; elsewhere, use vn host add <exe>");
  }
  const args = ["build", "--release", "-p", "vn-app", ...cargoFeatureArgs(variant)];
  log(`[vn] cargo ${args.join(" ")}`);
  const t0 = performance.now();
  const r = spawnSync(cargo(), args, { cwd: REPO_ROOT, stdio: "inherit" });
  if (r.status !== 0) throw new HostError(`cargo ${args.join(" ")} failed (exit ${r.status})`);
  const ms = Math.round(performance.now() - t0);
  const added = addHost(REPO_RELEASE_EXE, log);
  if (added.info.variant !== variant) {
    throw new HostError(`built ${args.slice(4).join(" ")} but the exe reports variant ${added.info.variant}`);
  }
  return { ...added, ms };
}
