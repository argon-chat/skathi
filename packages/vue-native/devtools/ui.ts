#!/usr/bin/env bun
/**
 * Launches the vue-native inspector (apps/devtools) against a running app.
 *
 *   bun devtools/ui.ts [--target <slot|app>] [--window WxH] [--no-build] [--wait]
 *
 * What it does, in order:
 *   1. resolves the TARGET's dist slot — a directory, or the name of one beside
 *      `packages/vue-native` (`dist`, `dist-shadcn`, …), or an app name whose
 *      conventional slot is `dist` / `dist-<app>`;
 *   2. builds `apps/devtools` into `packages/vue-native/dist-devtools` (skipped
 *      when nothing moved, like every other build here);
 *   3. starts a dev host on that slot with `VN_DEVTOOLS_TARGET` set, which is
 *      what makes the inspector attach to that instance rather than guessing.
 *
 * The inspector does NOT need the target to be running at launch: the target
 * list refreshes every few seconds and `--target` stays the preference. But if
 * one IS running the handshake is validated here, so a typo in the slot name is
 * a message rather than an empty window.
 *
 * See docs/DEVTOOLS-UI.md.
 */

import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG = resolve(HERE, ".."); // packages/vue-native
const REPO = resolve(PKG, "../..");
const ENTRY = join(REPO, "apps/devtools/src/main.js");
const UI_DIST = join(PKG, "dist-devtools");
const EXE = join(REPO, "target/x86_64-pc-windows-msvc/debug/vn-app.exe");

const USAGE = `vue-native devtools UI

  bun packages/vue-native/devtools/ui.ts [options]

options
  --target <slot>   the dist slot of the app to inspect: a path, a slot name
                    beside packages/vue-native (dist, dist-shadcn, …), or an
                    app name (hello -> dist, shadcn -> dist-shadcn).
                    Default: dist
  --window WxH      the inspector's window size (default: its vn.config.json)
  --no-build        use packages/vue-native/dist-devtools as it is
  --wait            stay in the foreground and forward the host's output
  --exe <path>      a different vn-app.exe
`;

function die(message: string): never {
  console.error(message);
  process.exit(1);
}

// ---- argv -----------------------------------------------------------------

const argv = process.argv.slice(2);
const flags: Record<string, string | boolean> = {};
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (!a.startsWith("--")) continue;
  const name = a.slice(2);
  if (name === "no-build" || name === "wait" || name === "help") flags[name] = true;
  else flags[name] = argv[++i] ?? "";
}
if (flags.help) {
  console.log(USAGE);
  process.exit(0);
}

// ---- the target ------------------------------------------------------------

/** `dist-shadcn` | `shadcn` | `Y:\…\slots\hello-1-1` -> an absolute slot dir. */
function resolveTargetSlot(spec: string): string {
  if (spec.includes("/") || spec.includes("\\")) return resolve(spec);
  const beside = join(PKG, spec);
  if (existsSync(beside)) return beside;
  const asApp = join(PKG, spec === "hello" ? "dist" : `dist-${spec}`);
  if (existsSync(asApp)) return asApp;
  const slots = readdirSync(PKG).filter((n) => n === "dist" || n.startsWith("dist-"));
  die(
    `no dist slot named ${JSON.stringify(spec)} under ${PKG}\n` +
      `  have: ${slots.join(", ")}\n` +
      `  or pass a path: --target packages/vn-test/.vn-test/slots/hello-123-1`,
  );
}

const targetSlot = resolveTargetSlot(String(flags.target || "dist"));
const handshake = join(targetSlot, "devtools.json");
if (existsSync(handshake)) {
  try {
    const h = JSON.parse(readFileSync(handshake, "utf8"));
    console.log(`[devtools:ui] target ${targetSlot} — port ${h.port}, pid ${h.pid}`);
  } catch {
    console.warn(`[devtools:ui] ${handshake} is not a valid handshake — attaching anyway`);
  }
} else {
  console.warn(
    `[devtools:ui] no devtools.json in ${targetSlot} yet — start the app and the inspector will ` +
      `pick it up (it rescans every few seconds)`,
  );
}

// ---- build -----------------------------------------------------------------

/** Newest mtime of anything the inspector is built from. */
function newestSource(dir: string, newest = 0): number {
  let out = newest;
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name.startsWith(".")) continue;
    const abs = join(dir, name);
    const st = statSync(abs);
    if (st.isDirectory()) out = newestSource(abs, out);
    else if (/\.(vue|js|ts|json|css)$/.test(name)) out = Math.max(out, st.mtimeMs);
  }
  return out;
}

if (!flags["no-build"]) {
  const manifest = join(UI_DIST, "manifest.json");
  const built = existsSync(manifest) ? statSync(manifest).mtimeMs : 0;
  const newest = Math.max(
    newestSource(join(REPO, "apps/devtools")),
    newestSource(join(PKG, "src")),
  );
  if (!built || newest > built) {
    console.log(`[devtools:ui] building apps/devtools -> ${UI_DIST} …`);
    const r = spawnSync("bun", ["build.ts", ENTRY, UI_DIST], { cwd: PKG, stdio: "inherit" });
    if (r.status !== 0) die("[devtools:ui] build failed");
  } else {
    console.log("[devtools:ui] dist-devtools is current");
  }
}

// ---- launch ----------------------------------------------------------------

const exe = typeof flags.exe === "string" && flags.exe ? resolve(flags.exe) : EXE;
if (!existsSync(exe)) die(`${exe} is missing — build the host first:\n  cargo build -p vn-app`);

const env: Record<string, string> = {
  ...(process.env as Record<string, string>),
  VN_DIST: UI_DIST,
  // Read by `__vn_devtools_targets()` in the host: which instance the inspector
  // should prefer when it boots. Everything else on the machine is still
  // listed — this is a preference, not a restriction.
  VN_DEVTOOLS_TARGET: targetSlot,
};
if (typeof flags.window === "string" && flags.window) env.VN_WINDOW = flags.window;

console.log(`[devtools:ui] starting inspector on ${UI_DIST}`);
const child = spawn(exe, [], {
  env,
  stdio: flags.wait ? "inherit" : "ignore",
  detached: !flags.wait,
});
if (flags.wait) {
  child.on("exit", (code) => process.exit(code ?? 0));
} else {
  child.unref();
  console.log(`[devtools:ui] pid ${child.pid} — it keeps running after this script exits`);
}
