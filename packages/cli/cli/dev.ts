// `sk dev [appDir]`: the dev loop for one app, as one command: `bun build.ts <entry>
// <appRoot>/dist/.sk-dev --watch` (HMR producer) plus a dev host (`SK_DIST=… SK_APP_ROOT=… <host>`).
//
// The dev host, first hit wins: `--exe` or SK_DEV_HOST; in a checkout, its debug sk-app (`cargo build
// -p sk-app`); the cached dev host of this version; a download of it from the release (host-fetch.ts).

import { spawn } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { loadAppConfig } from "./config";
import { cachedHost, checkHost, DEV_HOST, HostError, hostInfo, PKG_DIR, REPO_ROOT, CLI_VERSION } from "./host";
import { fetchHost } from "./host-fetch";
import { currentPlatform, hostsFor, repoHostExe } from "./platform";
import { colorEnabled, paint, progressFor } from "./term";

export async function devHost(
  options: { exe?: string | undefined; log?: (line: string) => void } = {},
): Promise<string> {
  const log = options.log ?? console.log;
  const named = options.exe ?? process.env.SK_DEV_HOST;
  if (named) {
    if (!existsSync(named)) throw new HostError(`no dev host at ${named}`);
    return named;
  }
  if (REPO_ROOT) {
    const debug = repoHostExe(REPO_ROOT, hostsFor(currentPlatform()), "debug");
    if (existsSync(debug)) return debug;
  }
  const cached = cachedHost(CLI_VERSION, DEV_HOST);
  const usable = () => {
    try {
      return existsSync(cached) && !checkHost(cached, hostInfo(cached), DEV_HOST, false);
    } catch {
      return false;
    }
  };
  if (!usable()) {
    if (process.env.SK_HOST_OFFLINE === "1") {
      throw new HostError(
        `no dev host: ${cached} is missing or unusable and SK_HOST_OFFLINE=1 forbids a download — ` +
          "pass --exe <dev host>" +
          (REPO_ROOT
            ? `, or build one with \`${["cargo build -p sk-app", ...hostsFor(currentPlatform()).cargoTarget].join(" ")}\``
            : ""),
      );
    }
    await fetchHost({ variant: DEV_HOST, log });
  }
  return cached;
}

export async function dev(target: string, options: { exe?: string | undefined } = {}): Promise<number> {
  const config = loadAppConfig(target);
  const c = paint(colorEnabled(process.stdout));
  const progress = progressFor(process.stderr);
  progress.step("dev host");
  const exe = await devHost({ ...options, log: (line) => progress.detail(line) }).finally(() => progress.end());
  const dist = join(config.appRoot, "dist", ".sk-dev");
  mkdirSync(dist, { recursive: true });
  const watcher = spawn(process.execPath, [join(PKG_DIR, "build.ts"), config.entry, dist, "--watch"], {
    cwd: PKG_DIR,
    stdio: "inherit",
    env: { ...process.env, SK_OWNER_PID: String(process.pid) },
  });
  const deadline = Date.now() + 120_000;
  while (!existsSync(join(dist, "manifest.json"))) {
    if (watcher.exitCode !== null) throw new Error(`bun build.ts exited with ${watcher.exitCode}`);
    if (Date.now() > deadline) throw new Error("bun build.ts produced no manifest within 2 minutes");
    await Bun.sleep(100);
  }
  console.log(`${c.green("▶")} ${c.bold(config.name)} ${c.dim(`dist ${dist}, host ${exe}`)}`);
  const host = spawn(exe, [], {
    stdio: "inherit",
    // SK_OWNER_PID: the host dies with this process however it ends (crates/sk-app/src/owner.rs).
    env: { ...process.env, SK_DIST: dist, SK_APP_ROOT: config.appRoot, SK_OWNER_PID: String(process.pid) },
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
