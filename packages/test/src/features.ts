import { spawnSync } from "node:child_process";
import { cpSync, existsSync, linkSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";
import { DEFAULT_EXE, REPO, TARGET, WORK_DIR, stagedExe } from "./dist";

export interface FeatureExeOptions {
  /** Cargo features, e.g. */
  features: string[];
  log?: (line: string) => void;
}

/** Where a feature build is kept between runs. */
export const featureExeDir = (features: string[]) =>
  join(WORK_DIR, "exes", features.length ? [...features].sort().join("+") : "dev");

/** The sources whose mtime decides whether a staged exe is still the truth. */
const SOURCE_ROOTS = [join(REPO, "crates"), join(REPO, "apps", "devtools"), join(REPO, "packages", "runtime", "src")];
const SOURCE_FILES = [
  join(REPO, "Cargo.toml"),
  join(REPO, "packages", "cli", "prod.ts"),
  join(REPO, "packages", "cli", "build.ts"),
  join(REPO, "packages", "cli", "tailwind.ts"),
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

/** Is `exe` older than the sources it was built from? */
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

/** `cargo`, wherever rustup put it. */
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
  log(`[sk-test] ${cmd} ${args.join(" ")}`);
  // `shell: true` because on Windows neither `bun` nor `cargo` is necessarily an `.exe` on PATH
  // (bun ships a `.cmd` shim), and `spawnSync` applies PATHEXT only through a shell.
  const quoted = args.map((a) => (/[\s&|<>^]/.test(a) ? `"${a}"` : a));
  const out = spawnSync(cmd, quoted, { cwd: REPO, encoding: "utf8", shell: true });
  if (out.error) {
    throw new Error(
      `${cmd} could not be run (${out.error.message}). This spec builds the host under a ` +
        `non-default feature set; ${cmd} has to be on PATH.`,
    );
  }
  if (out.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} failed with ${out.status}\n${out.stdout ?? ""}\n${out.stderr ?? ""}`);
  }
}

const inFlight = new Map<string, Promise<string>>();

/** A `sk-app.exe` built with `features`, staged where a spec can run it. */
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
  const exe = join(dir, "sk-app.exe");
  if (!stale(exe)) {
    log(`[sk-test] reusing ${exe}`);
    return exe;
  }

  // Pin the DEV bytes for whatever else this run launches, before the shared target directory is
  // touched at all.
  stagedExe();

  console.log(
    `[sk-test] building sk-app --features ${options.features.join(",")} — this recompiles one ` +
      `crate and relinks (~1 min), then restores the dev exe. It is cached at ${exe}.`,
  );

  // No JS step: a `prod` build carries no app any more (it is stamped in afterwards —
  // `packagedExe`), so the host is the same whatever runs on it.
  const args = ["build", "-p", "sk-app", "--target", TARGET];
  if (options.features.length) args.push("--features", options.features.join(","));
  run("cargo", args, log);

  mkdirSync(dirname(exe), { recursive: true });
  cpSync(DEFAULT_EXE, exe);
  // `icu-root`: the full ICU data cargo writes beside the exe, which `sk build` ships as icu.skpk.
  for (const name of readdirSync(dirname(DEFAULT_EXE)).filter((n) => /^icudt\w+\.dat$/.test(n))) {
    cpSync(join(dirname(DEFAULT_EXE), name), join(dirname(exe), name));
  }

  // Put the tree back: the next `cargo build` in somebody's editor must not find a prod host where
  // the dev host belongs.
  if (options.features.length) run("cargo", ["build", "-p", "sk-app", "--target", TARGET], log);
  return exe;
}

export type HostVariant = "standard" | "no-devtools";

/**
 * A BARE prod host of `variant`, the way `sk build` finds one: `SK_HOST`, the in-repo release exe,
 * or the local host cache (`sk workload build` fills it).
 */
export async function bareHost(variant: HostVariant, log: (l: string) => void = () => {}): Promise<string> {
  const { resolveHost, HostError } = await import("../../cli/cli/host");
  try {
    const h = resolveHost({ variant, log });
    log(`[sk-test] ${variant} host: ${h.exe} (${h.source}, ${h.info.profile})`);
    return h.exe;
  } catch (e) {
    if (!(e instanceof HostError)) throw e;
    log(`[sk-test] no prebuilt ${variant} host (${e.message.split("\n")[0]}) — building a debug one`);
    return featureExe({ features: variant === "standard" ? ["prod"] : ["prod", "no-devtools"], log });
  }
}

export interface PackagedExeOptions {
  /** A name under `apps/`, or a path to an app directory. */
  app: string;
  /** The bare host to stamp. */
  host?: string;
  variant?: HostVariant;
  /** Names the output directory and the exe. */
  label?: string;
  log?: (line: string) => void;
}

export interface PackagedExe {
  /** The stamped exe, renamed `sk-app-sktest-<runner pid>-<label>.exe` so the reaper knows it. */
  exe: string;
  /** What `sk build` wrote, under its own name (`Hello.exe`). */
  built: string;
  /** `sk.config.json` `"name"` — what `launch({ packaged })` takes. */
  name: string;
  result: import("../../cli/cli/build-app").BuildResult;
}

/**
 * `sk build <app> --host <bare host>` into `.sk-test/packaged/<label>/`, then a copy under a
 * reaper-visible name.
 */
export async function packagedExe(options: PackagedExeOptions): Promise<PackagedExe> {
  const log = options.log ?? (() => {});
  const label = (options.label ?? options.app).replace(/[^\w.-]+/g, "-");
  const variant = options.variant ?? "standard";
  const host = options.host ?? (await bareHost(variant, log));
  const target = existsSync(join(REPO, "apps", options.app)) ? join(REPO, "apps", options.app) : options.app;
  const out = join(WORK_DIR, "packaged", label);
  const { buildApp } = await import("../../cli/cli/build-app");
  const result = await buildApp({ target, out, host, variant, log });
  if (!result.exe) throw new Error(`sk build ${target} produced no exe`);
  // A macOS payload is Contents/Resources/app.skpk: the binary only finds it inside its bundle.
  if (process.platform === "darwin") return { exe: result.exe, built: result.exe, name: result.config.name, result };
  const exe = join(out, `sk-app-sktest-${process.pid}-${label}.exe`);
  rmSync(exe, { force: true });
  try {
    linkSync(result.exe, exe);
  } catch {
    cpSync(result.exe, exe);
  }
  return { exe, built: result.exe, name: result.config.name, result };
}

/** Does `path` contain `needle` anywhere in its bytes? */
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
