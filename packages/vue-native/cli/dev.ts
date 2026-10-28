// `vn dev [appDir]` — the in-repo dev loop for one app, as one command.
//
// It is the two things a developer already runs by hand, wired together:
//
//   bun build.ts <entry> <appRoot>/dist/.vn-dev --watch      (HMR producer)
//   VN_DIST=… VN_APP_ROOT=… target/…/debug/vn-app.exe        (the dev host)
//
// The dev host's compiled-in `DEV_DIST` path points into this checkout and
// `VN_DIST` / `VN_APP_ROOT` override it — but `build.ts` names every module by
// its path RELATIVE TO THE CHECKOUT (`toId`), so the app has to live inside
// it: an app on another drive has no relative path at all. Projects outside
// the repo (`--root`) and a prebuilt dev host are phase 2 / LATER, with
// distribution. `vn build` has no such limit — `prod.ts` emits one bundle.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { isAbsolute, join, relative } from "node:path";
import { ConfigError, loadAppConfig } from "./config";
import { PKG_DIR, REPO_ROOT } from "./host";

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
  const exe =
    options.exe ??
    process.env.VN_DEV_HOST ??
    (REPO_ROOT ? join(REPO_ROOT, "target", "x86_64-pc-windows-msvc", "debug", "vn-app.exe") : "");
  if (!exe || !existsSync(exe)) {
    throw new Error(
      `no dev host at ${exe || "(no checkout)"} — build one with \`cargo build -p vn-app\` in the vue-native ` +
        "checkout, or pass --exe <dev vn-app.exe>",
    );
  }
  const dist = join(config.appRoot, "dist", ".vn-dev");
  mkdirSync(dist, { recursive: true });
  const watcher = spawn(process.execPath, [join(PKG_DIR, "build.ts"), config.entry, dist, "--watch"], {
    cwd: PKG_DIR,
    stdio: "inherit",
  });
  // The first build has written its manifest before the host may read it.
  const deadline = Date.now() + 120_000;
  while (!existsSync(join(dist, "manifest.json"))) {
    if (watcher.exitCode !== null) throw new Error(`bun build.ts exited with ${watcher.exitCode}`);
    if (Date.now() > deadline) throw new Error("bun build.ts produced no manifest within 2 minutes");
    await Bun.sleep(100);
  }
  console.log(`[vn] dev: ${config.name} — dist ${dist}`);
  const host = spawn(exe, [], {
    stdio: "inherit",
    // VN_OWNER_PID: the host dies with this process however it ends
    // (crates/vn-app/src/owner.rs) — a killed `vn dev` leaves no window behind.
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
