// `vn dev [appDir]` — the in-repo dev loop for one app, as one command.
//
// It is the two things a developer already runs by hand, wired together:
//
//   bun build.ts <entry> <appRoot>/dist/.vn-dev --watch      (HMR producer)
//   VN_DIST=… VN_APP_ROOT=… target/…/debug/vn-app.exe        (the dev host)
//
// The dev host's compiled-in `DEV_DIST` path points into this checkout, and `build.ts` names every module
// by its path relative to the checkout (`toId`), so the app has to live inside it. `vn build` has no such
// limit: `prod.ts` emits one bundle.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { isAbsolute, join, relative } from "node:path";
import { ConfigError, loadAppConfig } from "./config";
import { PKG_DIR, REPO_ROOT } from "./host";
import { currentPlatform, hostsFor, repoHostExe } from "./platform";

export async function dev(target: string, options: { exe?: string } = {}): Promise<number> {
  const config = loadAppConfig(target);
  const rel = REPO_ROOT ? relative(REPO_ROOT, config.appRoot) : "";
  if (!REPO_ROOT || rel.startsWith("..") || isAbsolute(rel)) {
    throw new ConfigError(
      `vn dev runs apps inside the vue-native checkout (${REPO_ROOT ?? "none found"}); ${config.appRoot} is not. ` +
        "The dev builder names modules by their path in the checkout — an outside project is phase 2. " +
        "`vn build` works from anywhere.",
    );
  }
  const hosts = hostsFor(currentPlatform());
  const exe = options.exe ?? process.env.VN_DEV_HOST ?? (REPO_ROOT ? repoHostExe(REPO_ROOT, hosts, "debug") : "");
  if (!exe || !existsSync(exe)) {
    throw new Error(
      `no dev host at ${exe || "(no checkout)"} — build one with \`${["cargo build -p vn-app", ...hosts.cargoTarget].join(" ")}\` ` +
        `in the vue-native checkout, or pass --exe <dev ${hosts.exeFile}>`,
    );
  }
  const dist = join(config.appRoot, "dist", ".vn-dev");
  mkdirSync(dist, { recursive: true });
  const watcher = spawn(process.execPath, [join(PKG_DIR, "build.ts"), config.entry, dist, "--watch"], {
    cwd: PKG_DIR,
    stdio: "inherit",
    // The watcher, too, dies with this process however it ends (build.ts).
    env: { ...process.env, VN_OWNER_PID: String(process.pid) },
  });
  const deadline = Date.now() + 120_000;
  while (!existsSync(join(dist, "manifest.json"))) {
    if (watcher.exitCode !== null) throw new Error(`bun build.ts exited with ${watcher.exitCode}`);
    if (Date.now() > deadline) throw new Error("bun build.ts produced no manifest within 2 minutes");
    await Bun.sleep(100);
  }
  console.log(`[vn] dev: ${config.name} — dist ${dist}`);
  const host = spawn(exe, [], {
    stdio: "inherit",
    // VN_OWNER_PID: the host dies with this process however it ends (crates/vn-app/src/owner.rs).
    env: { ...process.env, VN_DIST: dist, VN_APP_ROOT: config.appRoot, VN_OWNER_PID: String(process.pid) },
  });
  const stop = () => {
    try {
      watcher.kill();
    } catch {}
    try {
      host.kill();
    } catch {}
  };
  process.on("SIGINT", () => {
    stop();
    process.exit(130);
  });
  const code: number = await new Promise((done) => host.on("exit", (c) => done(c ?? 0)));
  stop();
  return code;
}
