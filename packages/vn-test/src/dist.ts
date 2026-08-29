// Building the app under test, and handing each instance its own slot.
//
// Two separate jobs, deliberately not one:
//
//   1. BUILD, once per app per `bun test` process, into a CACHE dist. Rebuilt
//      only when a source file is newer than the manifest it produced.
//   2. SLOT, once per launch: a copy of the cache dist that this host owns.
//      A dist is not shareable — the host writes `devtools.json` into it at
//      boot, so two hosts on one dist would each claim to be the other.
//
// Splitting them is what makes N spec files cost one build and N copies
// (~5 MB, ~100 ms) instead of N builds.

import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";

/** …/packages/vn-test/src -> the workspace root. */
export const REPO = resolve(import.meta.dir, "../../..");
export const VUE_NATIVE_PKG = join(REPO, "packages/vue-native");
export const DEFAULT_EXE = join(REPO, "target/x86_64-pc-windows-msvc/debug/vn-app.exe");

/** Everything this harness writes that is not a user-facing artifact. */
export const WORK_DIR = join(REPO, "packages/vn-test/.vn-test");

const SOURCE_EXT = /\.(vue|js|ts|mjs|css|json)$/;

/** `hello` -> `apps/hello/src/main.js`; an explicit path is taken verbatim. */
export function resolveEntry(app: string): string {
  if (app.includes("/") || app.includes("\\") || app.endsWith(".js")) return resolve(REPO, app);
  const entry = join(REPO, "apps", app, "src", "main.js");
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

/**
 * The inputs a rebuild has to react to: the app's own sources, the runtime that
 * becomes vendor.js, and the config files that sit BESIDE `src/` (tailwind's
 * theme, vn.config.json, the app's package.json) and would otherwise be invisible.
 */
function newestInput(entry: string): number {
  const srcDir = dirname(entry);
  const appDir = srcDir.endsWith("src") ? dirname(srcDir) : srcDir;
  let newest = Math.max(newestMtime(srcDir), newestMtime(join(VUE_NATIVE_PKG, "src")));
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

/** Where `ensureBuild` puts (and `noBuild` looks for) an app's cached dist. */
export const cacheDistFor = (entry: string) => join(WORK_DIR, "dists", appNameOf(entry));

/** In-process build de-dup: two launches of the same app must not race the builder. */
const builds = new Map<string, Promise<string>>();

export type BuildLog = (line: string) => void;

/**
 * Builds `entry` into the shared cache dist if anything it is made of moved,
 * and returns that directory.
 *
 * The builder is `packages/vue-native/build.ts`, SHELLED TO rather than
 * imported: it is a script, not a module — it reads `process.argv` and runs a
 * full build at import time — so importing it twice in one process would build
 * twice into whatever the first call's globals said. Shelling out is also what
 * keeps the two honest, because it is the exact command a developer runs.
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
      log(`[vn-test] ${name}: dist is current (newest source ${new Date(newest).toISOString()})`);
      return cache;
    }
    log(`[vn-test] ${name}: building ${stamp ? "(sources moved)" : "(no dist yet)"} …`);
    const t0 = Date.now();
    const r = spawnSync("bun", ["build.ts", entry, cache], {
      cwd: VUE_NATIVE_PKG,
      encoding: "utf8",
    });
    if (r.status !== 0 || !existsSync(manifest)) {
      throw new Error(
        `build failed for ${entry}\n` +
          `  bun build.ts ${entry} ${cache}  (cwd ${VUE_NATIVE_PKG})\n` +
          `${r.stdout ?? ""}${r.stderr ?? ""}`,
      );
    }
    log(`[vn-test] ${name}: built in ${Date.now() - t0} ms`);
    return cache;
  })();

  builds.set(key, promise);
  return promise;
}

let slotSeq = 0;

/**
 * A private dist for one host: a copy of the cache, minus the handshake.
 *
 * `devtools.json` is deleted rather than left behind because the host NEVER
 * removes it (crates/vn-app/src/devtools/mod.rs: "the file is a 'who is live on
 * this dist slot right now' record, not history"). A copied-in stale one would
 * let the launcher connect to a port that belonged to a previous run — the
 * single most confusing failure this harness could have — so the slot starts
 * with no handshake at all and "the file exists" becomes a real readiness edge.
 */
export function allocateSlot(cacheDist: string, label: string): string {
  const slug = label.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 40) || "app";
  const dir = join(WORK_DIR, "slots", `${slug}-${process.pid}-${++slotSeq}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dirname(dir), { recursive: true });
  cpSync(cacheDist, dir, { recursive: true });
  rmSync(join(dir, "devtools.json"), { force: true });
  return dir;
}

/**
 * The executable this run drives, copied aside under its own name.
 *
 * The habit is older than this package (`vn-app-shadcn.exe` in m10,
 * `vn-app-m14.exe` in m14) and buys two things: `cargo build` can relink
 * `vn-app.exe` while a test run is in flight, and a `taskkill /IM` sweep aimed
 * at one of them cannot take down the other. One copy per PROCESS, not per
 * instance — the file is ~120 MB and every instance runs the same bytes.
 */
let exeCopy: string | null = null;
export function stagedExe(source = DEFAULT_EXE): string {
  if (exeCopy) return exeCopy;
  if (!existsSync(source)) {
    throw new Error(
      `${source} is missing — build the host first:\n  cargo build -p vn-app   (in ${REPO})`,
    );
  }
  const dir = join(WORK_DIR, "bin");
  mkdirSync(dir, { recursive: true });
  // Leftovers from runs that died before their exit hook: anything still locked
  // belongs to a live process and simply refuses to delete, which is the test.
  for (const name of readdirSync(dir)) {
    if (!/^vn-app-vntest-\d+\.exe$/.test(name)) continue;
    try {
      rmSync(join(dir, name));
    } catch {}
  }
  const staged = join(dir, `vn-app-vntest-${process.pid}.exe`);
  cpSync(source, staged);
  exeCopy = staged;
  return staged;
}

export function releaseStagedExe() {
  if (!exeCopy) return;
  try {
    rmSync(exeCopy);
  } catch {
    // Still locked: a host outlived its kill. The next run's sweep gets it.
  }
  exeCopy = null;
}

/** `.vn-test/` holds only derived files; a stray `.gitignore` keeps it invisible. */
export function ensureWorkDir() {
  mkdirSync(WORK_DIR, { recursive: true });
  const marker = join(WORK_DIR, ".gitignore");
  if (!existsSync(marker)) writeFileSync(marker, "*\n");
}
