#!/usr/bin/env bun
/**
 * Opens the vue-native inspector on a running app, from a shell.
 *
 *   bun devtools/ui.ts [--target <slot|app>] [--window WxH] [--wait]
 *
 * Turns a friendly `--target` into `vn-app.exe --devtools --target <absolute slot>`, which is what
 * F12 spawns. The inspector's sources do not hot-reload: the bundle in the executable is the
 * inspector (`cargo build -p vn-app` rebuilds it; `VN_DEVTOOLS_BUNDLE=<app.prod.js>` skips the bun
 * invocation). If the target is running, the handshake is validated here, so a typo is a message.
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

const exe = typeof flags.exe === "string" && flags.exe ? resolve(flags.exe) : EXE;
if (!existsSync(exe)) {
  die(
    `${exe} is missing — build the host first:\n  cargo build -p vn-app\n` +
      `(every build has the inspector; only \`--features no-devtools\` has none)`,
  );
}

const env: Record<string, string> = { ...(process.env as Record<string, string>) };
// VN_DIST names the slot a host serves; an inspector gets a private slot under %TEMP%, so an
// inherited VN_DIST would put its handshake on top of some app's: drop it.
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
