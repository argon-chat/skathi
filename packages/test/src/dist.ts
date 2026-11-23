// Two separate jobs, deliberately not one: 1. Rebuilt only when a source file is newer than the
// manifest it produced. 2.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { loadAppConfig } from "@skathi-native/cli/cli/config";
import { reapOrphans } from "./reap";

/** The Skathi checkout this harness runs from, or null when it is installed from npm. */
export const CHECKOUT: string | null = (() => {
  const root = resolve(import.meta.dir, "../../..");
  return existsSync(join(root, "Cargo.toml")) && existsSync(join(root, "crates", "sk-app")) ? root : null;
})();
/** The checkout's root, or the project directory `bun test` runs in. */
export const REPO = CHECKOUT ?? process.cwd();
export const CLI_PKG = dirname(createRequire(import.meta.url).resolve("@skathi-native/cli/package.json"));
const RUNTIME_SRC = join(
  dirname(createRequire(join(CLI_PKG, "package.json")).resolve("@skathi-native/runtime/package.json")),
  "src",
);
/** The cargo target this machine's host is built for (`.cargo/config.toml` defaults to Windows). */
export const TARGET =
  {
    win32: "x86_64-pc-windows-msvc",
    darwin: `${process.arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin`,
  }[process.platform as string] ?? "x86_64-unknown-linux-gnu";
export const EXE_SUFFIX = process.platform === "win32" ? ".exe" : "";
export const DEFAULT_EXE = join(REPO, "target", TARGET, "debug", `sk-app${EXE_SUFFIX}`);

/** Where the host keeps an app's own files (sk-storage's `LOCALAPPDATA` folder). */
export function appDataDir(app: string): string {
  if (process.platform === "win32") return join(process.env.LOCALAPPDATA ?? "", app);
  const home = process.env.HOME ?? "";
  if (process.platform === "darwin") return join(home, "Library", "Application Support", app);
  return join(process.env.XDG_DATA_HOME || join(home, ".local", "share"), app);
}

/** Everything this harness writes that is not a user-facing artifact. */
export const WORK_DIR = CHECKOUT ? join(CHECKOUT, "packages/test/.sk-test") : join(REPO, ".sk-test");

const SOURCE_EXT = /\.(vue|svelte|js|ts|mjs|css|json)$/;

/**
 * An app directory (its `sk.config.json` names the entry), an entry module, or in a Skathi checkout a
 * name under `apps/` (`hello` -> `apps/hello/src/main.ts`); paths are relative to {@link REPO}.
 */
export function resolveEntry(app: string): string {
  const dir = resolve(REPO, app);
  if (existsSync(join(dir, "sk.config.json"))) return loadAppConfig(dir).entry;
  const explicit = app.includes("/") || app.includes("\\") || app.endsWith(".ts") || app.endsWith(".js");
  if (explicit || !CHECKOUT) return dir;
  const entry = join(REPO, "apps", app, "src", "main.ts");
  if (!existsSync(entry)) {
    const available = existsSync(join(REPO, "apps"))
      ? readdirSync(join(REPO, "apps")).join(", ")
      : "(no apps/ directory)";
    throw new Error(`no app named ${JSON.stringify(app)} — ${entry} does not exist. Have: ${available}`);
  }
  return entry;
}

/** Newest mtime under `dir`, skipping the directories a build never reads. */
function newestMtime(dir: string, newest = 0): number {
  let out = newest;
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of names) {
    if (name === "node_modules" || name === "dist" || name.startsWith(".")) continue;
    const abs = join(dir, name);
    let st;
    try {
      st = statSync(abs);
    } catch {
      continue;
    }
    if (st.isDirectory()) out = newestMtime(abs, out);
    else if (SOURCE_EXT.test(name)) out = Math.max(out, st.mtimeMs);
  }
  return out;
}

function newestInput(entry: string): number {
  const srcDir = dirname(entry);
  const appDir = srcDir.endsWith("src") ? dirname(srcDir) : srcDir;
  let newest = Math.max(newestMtime(srcDir), newestMtime(RUNTIME_SRC), statSync(join(CLI_PKG, "build.ts")).mtimeMs);
  for (const name of readdirSync(appDir)) {
    if (!SOURCE_EXT.test(name)) continue;
    try {
      newest = Math.max(newest, statSync(join(appDir, name)).mtimeMs);
    } catch {}
  }
  return newest;
}

/** The app's own directory name — `apps/hello/src/main.js` -> `hello`. */
export function appNameOf(entry: string): string {
  const srcDir = dirname(entry);
  const appDir = srcDir.endsWith("src") ? dirname(srcDir) : srcDir;
  return appDir.split(/[\\/]/).pop()!;
}

/** Where `ensureBuild` puts (and `noBuild` looks for) an entry's cached dist. */
export const cacheDistFor = (entry: string) => {
  const key = createHash("sha256").update(resolve(entry).toLowerCase()).digest("hex").slice(0, 10);
  return join(WORK_DIR, "dists", `${appNameOf(entry)}-${key}`);
};

const BUILD_TIMEOUT_MS = 10 * 60_000;

/** In-process build de-dup: two launches of the same app must not race the builder. */
const builds = new Map<string, Promise<string>>();

export type BuildLog = (line: string) => void;

/**
 * Builds `entry` into the shared cache dist if anything it is made of moved, and returns that
 * directory.
 */
export function ensureBuild(entry: string, log: BuildLog = () => {}): Promise<string> {
  const key = entry.toLowerCase();
  const existing = builds.get(key);
  if (existing) return existing;

  const promise = (async () => withCacheLock(cacheDistFor(entry), () => build(entry, log)))();

  builds.set(key, promise);
  return promise;
}

function build(entry: string, log: BuildLog): string {
  const name = appNameOf(entry);
  const cache = cacheDistFor(entry);
  const manifest = join(cache, "manifest.json");
  const stamp = existsSync(manifest) ? statSync(manifest).mtimeMs : 0;
  const newest = newestInput(entry);
  if (stamp && newest <= stamp) {
    log(`[sk-test] ${name}: dist is current (newest source ${new Date(newest).toISOString()})`);
    return cache;
  }
  log(`[sk-test] ${name}: building ${stamp ? "(sources moved)" : "(no dist yet)"} …`);
  const t0 = Date.now();
  const r = spawnSync("bun", ["--no-install", "build.ts", entry, cache], {
    cwd: CLI_PKG,
    encoding: "utf8",
    timeout: BUILD_TIMEOUT_MS,
  });
  if (r.status !== 0 || !existsSync(manifest)) {
    throw new Error(
      `build failed for ${entry}${r.error ? ` (${r.error.message})` : ""}\n` +
        `  bun build.ts ${entry} ${cache}  (cwd ${CLI_PKG})\n` +
        `${r.stdout ?? ""}${r.stderr ?? ""}`,
    );
  }
  log(`[sk-test] ${name}: built in ${Date.now() - t0} ms`);
  return cache;
}

/**
 * Runs `fn` holding `<dir>.lock`, for a cache dist under {@link WORK_DIR}: runs in other processes
 * build into and copy out of the same one.
 */
function withCacheLock<T>(dir: string, fn: () => T): T {
  if (!resolve(dir).toLowerCase().startsWith(resolve(WORK_DIR).toLowerCase())) return fn();
  const lock = `${dir}.lock`;
  const owner = join(lock, "pid");
  mkdirSync(dirname(lock), { recursive: true });
  const deadline = Date.now() + BUILD_TIMEOUT_MS;
  for (;;) {
    try {
      mkdirSync(lock);
      break;
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== "EEXIST") throw e;
    }
    let holder = 0;
    try {
      holder = Number(readFileSync(owner, "utf8"));
    } catch {}
    if ((holder && !runnerAlive(holder)) || Date.now() > deadline) {
      rmSync(lock, { recursive: true, force: true });
      continue;
    }
    Bun.sleepSync(50);
  }
  try {
    writeFileSync(owner, String(process.pid));
    return fn();
  } finally {
    rmSync(lock, { recursive: true, force: true });
  }
}

let slotSeq = 0;

/** A private dist for one host: a copy of the cache, minus the handshake. */
export function allocateSlot(cacheDist: string, label: string): string {
  const slug = label.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 40) || "app";
  const dir = join(WORK_DIR, "slots", `${slug}-${process.pid}-${++slotSeq}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dirname(dir), { recursive: true });
  withCacheLock(cacheDist, () => cpSync(cacheDist, dir, { recursive: true }));
  rmSync(join(dir, "devtools.json"), { force: true });
  return dir;
}

/**
 * The dev host a launch runs: `SK_TEST_HOST`, else the checkout's debug build, else the one `sk dev`
 * uses (cached or downloaded).
 */
export async function devHostExe(log: BuildLog = () => {}): Promise<string> {
  const named = process.env.SK_TEST_HOST;
  if (named) {
    const exe = resolve(named);
    if (!existsSync(exe)) throw new Error(`SK_TEST_HOST=${named}: no such file`);
    await requireCurrentHost(exe, "rebuild the host SK_TEST_HOST names, or unset SK_TEST_HOST");
    return exe;
  }
  if (CHECKOUT) {
    if (existsSync(DEFAULT_EXE)) await requireCurrentHost(DEFAULT_EXE, `cargo build -p sk-app --target ${TARGET}`);
    return DEFAULT_EXE;
  }
  const { devHost } = await import("@skathi-native/cli/cli/dev");
  return devHost({ log });
}

const checkedHosts = new Map<string, string | null>();

/** Throws, with `rebuild` as the remedy, when `exe` was built against another runtime protocol than the specs run. */
export async function requireCurrentHost(exe: string, rebuild: string): Promise<void> {
  if (!checkedHosts.has(exe)) checkedHosts.set(exe, await hostStaleness(exe));
  const stale = checkedHosts.get(exe);
  if (stale) throw new Error(`${stale}. The specs do not run against an out-of-date host:\n  ${rebuild}`);
}

/** Why `exe` does not match the runtime the specs bundle, or null when it does or cannot tell. */
export async function hostStaleness(exe: string): Promise<string | null> {
  const { hostInfo, protocolMismatch } = await import("@skathi-native/cli/cli/host");
  return protocolMismatch(exe, hostInfo(exe));
}

/** The executable this run drives, copied aside under its own name. */
let exeCopy: string | null = null;
export function stagedExe(source = DEFAULT_EXE): string {
  if (exeCopy) return exeCopy;
  if (!existsSync(source)) {
    throw new Error(`${source} is missing — build the host first:\n  cargo build -p sk-app   (in ${REPO})`);
  }
  const dir = join(WORK_DIR, "bin");
  mkdirSync(dir, { recursive: true });
  // A host still running from a dead run is what keeps its copy locked, so the reap goes first and
  // the sweep below can then delete what it freed.
  reapOrphans();
  // Leftovers from runs that died before their exit hook: anything still locked belongs to a live
  // process and simply refuses to delete, which is the test.
  for (const name of readdirSync(dir)) {
    const owner = /^sk-app-sktest-(\d+)\.exe$/.exec(name);
    // Between specs a live runner's copy is unlocked, so a nested runner must not take it.
    if (!owner || runnerAlive(Number(owner[1]))) continue;
    try {
      rmSync(join(dir, name));
    } catch {}
  }
  const staged = join(dir, `sk-app-sktest-${process.pid}.exe`);
  cpSync(source, staged);
  exeCopy = staged;
  return staged;
}

function runnerAlive(pid: number): boolean {
  if (pid === process.pid) return true;
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === "EPERM";
  }
}

export function releaseStagedExe() {
  if (!exeCopy) return;
  try {
    rmSync(exeCopy);
  } catch {}
  exeCopy = null;
}

/** `.sk-test/` holds only derived files; a stray `.gitignore` keeps it invisible. */
export function ensureWorkDir() {
  mkdirSync(WORK_DIR, { recursive: true });
  const marker = join(WORK_DIR, ".gitignore");
  if (!existsSync(marker)) writeFileSync(marker, "*\n");
}
