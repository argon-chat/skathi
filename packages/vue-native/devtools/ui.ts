#!/usr/bin/env bun
/**
 * Opens the vue-native inspector on a running app, from a shell.
 *
 *   bun devtools/ui.ts [--target <slot|app>] [--window WxH] [--wait]
 *
 * It is a thin wrapper — deliberately. All it does is turn a friendly
 * `--target` (a slot name, an app name, a path) into the one command the host
 * already understands:
 *
 *   vn-app.exe --devtools --target <absolute slot>
 *
 * …which is EXACTLY what F12 spawns (`crates/vn-app/src/inspector.rs`,
 * `open_for_window`). One code path, one inspector.
 *
 * It did not used to be. Until m21 this script built `apps/devtools` into
 * `packages/vue-native/dist-devtools` and started an ordinary dev host on that
 * slot, while F12 evaluated the bundle `build.rs` had baked into the binary. So
 * there were two inspectors — with two build states, two ways to be stale, and
 * two sets of boot arguments (a hand-launched one never got
 * `__VN_DEVTOOLS_ARGS__` at all, which is why every reader of it guards). The
 * dist-devtools half is gone; the bundle in the executable is the inspector.
 *
 * WHAT THAT COSTS, stated rather than discovered: the inspector's own sources
 * no longer hot-reload while you edit them, because they are no longer served
 * from a dist slot. Re-running `cargo build -p vn-app` rebuilds the bundle
 * (build.rs re-runs when anything under `apps/devtools` moves), and
 * `VN_DEVTOOLS_BUNDLE=<path to app.prod.js>` skips the bun invocation inside it
 * when you want to iterate on the bundle alone.
 *
 * The inspector does NOT need the target to be running at launch: its target
 * list refreshes every few seconds and `--target` stays the preference. But if
 * one IS running the handshake is validated here, so a typo in the slot name is
 * a message rather than an empty window.
 *
 * See docs/DEVTOOLS-UI.md.
 */

import { spawn } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG = resolve(HERE, ".."); // packages/vue-native
const REPO = resolve(PKG, "../..");
const EXE = join(REPO, "target/x86_64-pc-windows-msvc/debug/vn-app.exe");

const USAGE = `vue-native devtools UI

  bun packages/vue-native/devtools/ui.ts [options]

Opens the inspector that is embedded in the host binary — the same one F12
opens, launched the same way (\`vn-app --devtools --target <slot>\`).

options
  --target <slot>   the dist slot of the app to inspect: a path, a slot name
                    beside packages/vue-native (dist, dist-shadcn, …), or an
                    app name (hello -> dist, shadcn -> dist-shadcn).
                    Default: dist
  --window WxH      the inspector's window size (default: its vn.config.json)
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
  if (name === "wait" || name === "help") flags[name] = true;
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

// ---- launch ----------------------------------------------------------------

const exe = typeof flags.exe === "string" && flags.exe ? resolve(flags.exe) : EXE;
if (!existsSync(exe)) {
  die(
    `${exe} is missing — build the host first:\n  cargo build -p vn-app\n` +
      `(a \`--features prod\` build has no inspector at all; \`prod,devtools\` has one)`,
  );
}

const env: Record<string, string> = { ...(process.env as Record<string, string>) };
// VN_DIST names the slot a host SERVES. An inspector serves nothing — it
// evaluates the bundle inside the executable — and `inspector::init` gives it a
// private slot under %TEMP% to publish its own handshake into. Inheriting the
// shell's VN_DIST would put that handshake on top of some app's, so drop it.
delete env.VN_DIST;
// Likewise `--target` is what sets VN_DEVTOOLS_TARGET in the child.
delete env.VN_DEVTOOLS_TARGET;
if (typeof flags.window === "string" && flags.window) env.VN_WINDOW = flags.window;

const args = ["--devtools", "--target", targetSlot];
console.log(`[devtools:ui] ${exe} ${args.join(" ")}`);
const child = spawn(exe, args, {
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
