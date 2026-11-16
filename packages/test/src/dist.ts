// Two separate jobs, deliberately not one: 1. Rebuilt only when a source file is newer than the
// manifest it produced. 2.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { reapOrphans } from "./reap";

/** …/packages/test/src -> the workspace root. */
export const REPO = resolve(import.meta.dir, "../../..");
export const CLI_PKG = join(REPO, "packages/cli");
const RUNTIME_SRC = join(REPO, "packages/runtime/src");
export const DEFAULT_EXE = join(
  REPO,
  {
    win32: "target/x86_64-pc-windows-msvc/debug/sk-app.exe",
    darwin: `target/${process.arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin/debug/sk-app`,
  }[process.platform as string] ?? "target/x86_64-unknown-linux-gnu/debug/sk-app",
);

/** Everything this harness writes that is not a user-facing artifact. */
export const WORK_DIR = join(REPO, "packages/test/.sk-test");

const SOURCE_EXT = /\.(vue|svelte|js|ts|mjs|css|json)$/;

/** `hello` -> `apps/hello/src/main.ts`; an explicit path is taken verbatim. */
export function resolveEntry(app: string): string {
  const explicit = app.includes("/") || app.includes("\\") || app.endsWith(".ts") || app.endsWith(".js");
  if (explicit) return resolve(REPO, app);
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

  const promise = (async () => {
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
    });
    if (r.status !== 0 || !existsSync(manifest)) {
      throw new Error(
        `build failed for ${entry}\n` +
          `  bun build.ts ${entry} ${cache}  (cwd ${CLI_PKG})\n` +
          `${r.stdout ?? ""}${r.stderr ?? ""}`,
      );
    }
    log(`[sk-test] ${name}: built in ${Date.now() - t0} ms`);
    return cache;
  })();

  builds.set(key, promise);
  return promise;
}

let slotSeq = 0;

/** A private dist for one host: a copy of the cache, minus the handshake. */
export function allocateSlot(cacheDist: string, label: string): string {
  const slug = label.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 40) || "app";
  const dir = join(WORK_DIR, "slots", `${slug}-${process.pid}-${++slotSeq}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dirname(dir), { recursive: true });
  cpSync(cacheDist, dir, { recursive: true });
  rmSync(join(dir, "devtools.json"), { force: true });
  return dir;
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
