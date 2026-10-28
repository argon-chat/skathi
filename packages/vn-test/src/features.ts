// Building the host under a NON-DEFAULT feature set, for the specs that are
// about the BUILD rather than about an app.
//
// Two claims need this and cannot be made any other way (docs/DEVTOOLS.md
// "Shape", research/reports/m22/devtools-release.md):
//
//   * `--features prod` — a SHIPPED app — carries the devtools and opens its
//     inspector on F12. Since m22 that is the default, and the only honest
//     proof is to run a prod binary and press the key.
//   * `--features no-devtools` still compiles, and the resulting image
//     contains none of it — no listener, no bundle, not even the inspector's
//     window title.
//
// ## Why this is allowed to run `cargo`
//
// It is the same bargain `ensureBuild` already makes for JS: a spec that
// asserts something about an artifact has to be able to produce that artifact,
// or it degrades into a spec that passes because somebody remembered to build
// by hand. The cost is stated rather than hidden — see `staleness` below.
//
// ## What it costs, and why it is not an isolated target dir
//
// The build happens in the WORKSPACE's own `target/`, so it recompiles exactly
// one crate (`vn-app`; no feature here propagates to a dependency) and then
// relinks — about a minute on the machine this was written on. A private
// `CARGO_TARGET_DIR` would avoid disturbing the dev tree, at the price of
// rebuilding every dependency including gpui and JavaScriptCore, which is tens
// of minutes and gigabytes. So instead the disturbance is UNDONE: the staged
// exe is copied out and `cargo build -p vn-app` runs again, leaving
// `target/.../debug/vn-app.exe` the plain dev host it was.
//
// `stagedExe()` is called before any of that, so the copy this run's other
// hosts are using is already pinned and cannot be replaced underneath them.

import { spawnSync } from "node:child_process";
import { cpSync, existsSync, linkSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";
import { DEFAULT_EXE, REPO, WORK_DIR, stagedExe } from "./dist";

export interface FeatureExeOptions {
  /**
   * Cargo features, e.g. `["prod"]` or `["prod", "no-devtools"]`. A `prod`
   * build is a BARE HOST since m25 — no app inside; give it one with
   * [`packagedExe`].
   */
  features: string[];
  log?: (line: string) => void;
}

/** Where a feature build is kept between runs. */
export const featureExeDir = (features: string[]) =>
  join(WORK_DIR, "exes", features.length ? [...features].sort().join("+") : "dev");

/** The sources whose mtime decides whether a staged exe is still the truth. */
const SOURCE_ROOTS = [
  join(REPO, "crates"),
  join(REPO, "apps", "devtools"),
  join(REPO, "packages", "vue-native", "src"),
];
const SOURCE_FILES = [
  join(REPO, "Cargo.toml"),
  join(REPO, "packages", "vue-native", "prod.ts"),
  join(REPO, "packages", "vue-native", "build.ts"),
  join(REPO, "packages", "vue-native", "tailwind.ts"),
];
const WATCHED = /\.(rs|toml|ts|vue|js|json)$/;

function newest(dir: string, out = 0): number {
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return out;
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
    if (st.isDirectory()) out = newest(abs, out);
    else if (WATCHED.test(name)) out = Math.max(out, st.mtimeMs);
  }
  return out;
}

/**
 * Is `exe` older than the sources it was built from?
 *
 * Deliberately a real staleness check rather than a presence check. A cached
 * binary that silently answers for code that has moved is the same failure
 * `crates/vn-app/build.rs` refuses to ship for the inspector bundle — "a
 * debugger that shows you last week's panels" — and a green spec proving a
 * property of last week's build is worse than a slow one.
 */
function stale(exe: string): boolean {
  if (!existsSync(exe)) return true;
  const built = statSync(exe).mtimeMs;
  let newestSource = 0;
  for (const root of SOURCE_ROOTS) newestSource = Math.max(newestSource, newest(root));
  for (const file of SOURCE_FILES) {
    try {
      newestSource = Math.max(newestSource, statSync(file).mtimeMs);
    } catch {}
  }
  return built < newestSource;
}

/**
 * `cargo`, wherever rustup put it.
 *
 * A shell spawned by a long-lived process inherits the PATH that process was
 * started with, so a machine where `cargo` was added to the user PATH *after*
 * the terminal (or the editor, or an agent's shell) opened runs these specs
 * with no `cargo` on PATH at all — while the same command works by hand in a
 * fresh window. rustup's location is fixed, so ask it before giving up.
 */
function cargoCommand(): string {
  if (spawnSync("cargo", ["--version"], { shell: true, encoding: "utf8" }).status === 0) {
    return "cargo";
  }
  const home = process.env.CARGO_HOME ?? join(process.env.USERPROFILE ?? homedir(), ".cargo");
  const exe = join(home, "bin", process.platform === "win32" ? "cargo.exe" : "cargo");
  return existsSync(exe) ? exe : "cargo";
}

function run(cmd: string, args: string[], log: (l: string) => void) {
  if (cmd === "cargo") cmd = cargoCommand();
  log(`[vn-test] ${cmd} ${args.join(" ")}`);
  // `shell: true` because on Windows neither `bun` nor `cargo` is necessarily
  // an `.exe` on PATH (bun ships a `.cmd` shim), and `spawnSync` applies
  // PATHEXT only through a shell. Which means the arguments are re-parsed by
  // cmd, so anything with a space is quoted here.
  const quoted = args.map((a) => (/[\s&|<>^]/.test(a) ? `"${a}"` : a));
  const out = spawnSync(cmd, quoted, { cwd: REPO, encoding: "utf8", shell: true });
  if (out.error) {
    throw new Error(
      `${cmd} could not be run (${out.error.message}). This spec builds the host under a ` +
        `non-default feature set; ${cmd} has to be on PATH.`,
    );
  }
  if (out.status !== 0) {
    throw new Error(
      `${cmd} ${args.join(" ")} failed with ${out.status}\n${out.stdout ?? ""}\n${out.stderr ?? ""}`,
    );
  }
}

const inFlight = new Map<string, Promise<string>>();

/**
 * A `vn-app.exe` built with `features`, staged where a spec can run it.
 *
 * Cached across runs and de-duplicated within one, so several specs asking for
 * the same feature set pay for one build.
 */
export function featureExe(options: FeatureExeOptions): Promise<string> {
  const dir = featureExeDir(options.features);
  const key = dir;
  const existing = inFlight.get(key);
  if (existing) return existing;
  const task = build(options, dir);
  inFlight.set(key, task);
  return task;
}

async function build(options: FeatureExeOptions, dir: string): Promise<string> {
  const log = options.log ?? (() => {});
  const exe = join(dir, "vn-app.exe");
  if (!stale(exe)) {
    log(`[vn-test] reusing ${exe}`);
    return exe;
  }

  // Pin the DEV bytes for whatever else this run launches, before the shared
  // target directory is touched at all.
  stagedExe();

  console.log(
    `[vn-test] building vn-app --features ${options.features.join(",")} — this recompiles one ` +
      `crate and relinks (~1 min), then restores the dev exe. It is cached at ${exe}.`,
  );

  // No JS step: a `prod` build carries no app any more (it is stamped in
  // afterwards — `packagedExe`), so the host is the same whatever runs on it.
  const args = ["build", "-p", "vn-app"];
  if (options.features.length) args.push("--features", options.features.join(","));
  run("cargo", args, log);

  mkdirSync(dirname(exe), { recursive: true });
  cpSync(DEFAULT_EXE, exe);

  // Put the tree back: the next `cargo build` in somebody's editor must not
  // find a prod host where the dev host belongs.
  if (options.features.length) run("cargo", ["build", "-p", "vn-app"], log);
  return exe;
}

// ---------------------------------------------------------------------------
// packaged apps (`vn build`, docs/PACKAGING.md)
// ---------------------------------------------------------------------------

export type HostVariant = "standard" | "no-devtools";

/**
 * A BARE prod host of `variant`, the way `vn build` finds one: `VN_HOST`, the
 * in-repo release exe, or the local host cache (`vn host build` fills it).
 * When none exists, a DEBUG feature build stands in — minutes rather than the
 * release build's tens of minutes, and the same host for a spec's purposes.
 */
export async function bareHost(variant: HostVariant, log: (l: string) => void = () => {}): Promise<string> {
  const { resolveHost, HostError } = await import("../../vue-native/cli/host");
  try {
    const h = resolveHost({ variant, log });
    log(`[vn-test] ${variant} host: ${h.exe} (${h.source}, ${h.info.profile})`);
    return h.exe;
  } catch (e) {
    if (!(e instanceof HostError)) throw e;
    log(`[vn-test] no prebuilt ${variant} host (${e.message.split("\n")[0]}) — building a debug one`);
    return featureExe({ features: variant === "standard" ? ["prod"] : ["prod", "no-devtools"], log });
  }
}

export interface PackagedExeOptions {
  /** A name under `apps/`, or a path to an app directory. */
  app: string;
  /** The bare host to stamp. Default: [`bareHost`] of `variant`. */
  host?: string;
  variant?: HostVariant;
  /** Names the output directory and the exe. Default: the app. */
  label?: string;
  log?: (line: string) => void;
}

export interface PackagedExe {
  /** The stamped exe, renamed `vn-app-vntest-<runner pid>-<label>.exe` so the reaper knows it. */
  exe: string;
  /** What `vn build` wrote, under its own name (`Hello.exe`). */
  built: string;
  /** `vn.config.json` `"name"` — what `launch({ packaged })` takes. */
  name: string;
  result: import("../../vue-native/cli/build-app").BuildResult;
}

/**
 * `vn build <app> --host <bare host>` into `.vn-test/packaged/<label>/`, then a
 * copy under a reaper-visible name. In-process (the CLI is TypeScript in this
 * repo), so a spec gets the build's own result back rather than parsing text.
 */
export async function packagedExe(options: PackagedExeOptions): Promise<PackagedExe> {
  const log = options.log ?? (() => {});
  const label = (options.label ?? options.app).replace(/[^\w.-]+/g, "-");
  const variant = options.variant ?? "standard";
  const host = options.host ?? (await bareHost(variant, log));
  const target = existsSync(join(REPO, "apps", options.app)) ? join(REPO, "apps", options.app) : options.app;
  const out = join(WORK_DIR, "packaged", label);
  const { buildApp } = await import("../../vue-native/cli/build-app");
  const result = await buildApp({ target, out, host, variant, log });
  if (!result.exe) throw new Error(`vn build ${target} produced no exe`);
  const exe = join(out, `vn-app-vntest-${process.pid}-${label}.exe`);
  // A hard link, not a copy: the same ~100 MB image under a second name.
  rmSync(exe, { force: true });
  try {
    linkSync(result.exe, exe);
  } catch {
    cpSync(result.exe, exe);
  }
  return { exe, built: result.exe, name: result.config.name, result };
}

/**
 * Does `path` contain `needle` anywhere in its bytes?
 *
 * Streamed in chunks with an overlap, because these images are ~100 MB and a
 * spec that reads one into a string would be measured in seconds and hundreds
 * of megabytes. ASCII only — every marker this is used for is a literal from
 * the source.
 */
export async function fileContains(path: string, needle: string): Promise<boolean> {
  const bytes = new TextEncoder().encode(needle);
  const overlap = bytes.length - 1;
  const stream = Bun.file(path).stream();
  let tail = new Uint8Array(0);
  for await (const chunk of stream as unknown as AsyncIterable<Uint8Array>) {
    const buf = new Uint8Array(tail.length + chunk.length);
    buf.set(tail, 0);
    buf.set(chunk, tail.length);
    if (indexOfBytes(buf, bytes) >= 0) return true;
    tail = overlap > 0 ? buf.slice(Math.max(0, buf.length - overlap)) : new Uint8Array(0);
  }
  return false;
}

function indexOfBytes(haystack: Uint8Array, needle: Uint8Array): number {
  const last = haystack.length - needle.length;
  outer: for (let i = 0; i <= last; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}
