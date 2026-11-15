// `sk dev [appDir]`: the dev loop for one app, as one command: `bun build.ts <entry>
// <appRoot>/dist/.sk-dev --watch` (HMR producer) plus a dev host (`SK_DIST=… SK_APP_ROOT=… <host>`).
//
// The dev host, first hit wins: `--exe` or SK_DEV_HOST; in a checkout, its debug sk-app (`cargo build
// -p sk-app`); the cached dev host of this version; a download of it from the release (host-fetch.ts).
//
// An app with shaders/*.slang has them compiled into <dist>/effects, which the host reads through
// SK_EFFECTS_DIR, and recompiled on every change. Renderers build effect pipelines once, so a good
// rebuild restarts the host; a failed one prints the diagnostics and keeps the last good build running.

import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { compileAppEffects, EffectsError, formatProblems, hasAppEffects, writeEffectFiles } from "../effects/compile";
import { resolveCompiler } from "../effects/redslangc";
import { loadAppConfig } from "./config";
import { watchAppEffects, type EffectsWatch } from "./effects";
import { cachedHost, checkHost, DEV_HOST, HostError, hostInfo, PKG_DIR, REPO_ROOT, CLI_VERSION } from "./host";
import { fetchHost, type OnBytes } from "./host-fetch";
import { currentPlatform, hostsFor, repoHostExe } from "./platform";
import { colorEnabled, paint, progressFor } from "./term";

export async function devHost(
  options: { exe?: string | undefined; log?: (line: string) => void; onBytes?: OnBytes } = {},
): Promise<string> {
  const log = options.log ?? console.log;
  const named = options.exe ?? process.env.SK_DEV_HOST;
  if (named) {
    if (!existsSync(named)) throw new HostError(`no dev workload at ${named}`);
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
        `no dev workload: ${cached} is missing or unusable and SK_HOST_OFFLINE=1 forbids a download — ` +
          "pass --exe <dev workload>" +
          (REPO_ROOT
            ? `, or build one with \`${["cargo build -p sk-app", ...hostsFor(currentPlatform()).cargoTarget].join(" ")}\``
            : ""),
      );
    }
    await fetchHost({ variant: DEV_HOST, log, onBytes: options.onBytes });
  }
  return cached;
}

export async function dev(
  target: string,
  options: { exe?: string | undefined; shaderCompiler?: string | undefined; color?: boolean | undefined } = {},
): Promise<number> {
  const config = loadAppConfig(target);
  const c = paint(colorEnabled(process.stdout, options.color));
  const progress = progressFor(process.stderr, colorEnabled(process.stderr, options.color));
  progress.step("dev workload");
  const exe = await devHost({
    ...options,
    log: (line) => progress.detail(line),
    onBytes: (done, total) => progress.bytes(done, total),
  }).catch((e: unknown) => {
    progress.fail();
    throw e;
  });
  const dist = join(config.appRoot, "dist", ".sk-dev");
  mkdirSync(dist, { recursive: true });

  const effectsDir = join(dist, "effects");
  const overridden = !!process.env.SK_EFFECTS_DIR;
  let compiler: string | null = null;
  if (hasAppEffects(config.appRoot) && !overridden) {
    progress.step("effects");
    try {
      compiler = (
        await resolveCompiler({
          named: options.shaderCompiler,
          log: (line) => progress.info(line),
          onBytes: (done, total) => progress.bytes(done, total),
        })
      ).path;
      const effects = await compileAppEffects(config.appRoot, compiler);
      writeEffectFiles(effectsDir, effects.files);
      progress.detail(effects.compiled.map((e) => e.effect.name).join(", "));
    } catch (e) {
      progress.fail();
      const text = e instanceof EffectsError ? formatProblems(e.problems) : e instanceof Error ? e.message : String(e);
      console.error(`${c.red("effects")}: ${text}`);
      if (existsSync(join(effectsDir, "effects.json"))) console.error(c.dim("the app starts with the last good build"));
    }
  }
  progress.end();

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
  console.log(`${c.green("▶")} ${c.bold(config.name)} ${c.dim(`dist ${dist}, workload ${exe}`)}`);
  const start = (): ChildProcess =>
    spawn(exe, [], {
      stdio: "inherit",
      // SK_OWNER_PID: the host dies with this process however it ends (crates/sk-app/src/owner.rs).
      env: {
        ...process.env,
        SK_DIST: dist,
        SK_APP_ROOT: config.appRoot,
        SK_OWNER_PID: String(process.pid),
        ...(!overridden && existsSync(join(effectsDir, "effects.json")) ? { SK_EFFECTS_DIR: effectsDir } : {}),
      },
    });
  let host = start();
  let restarting = false;
  let effectsWatch: EffectsWatch | null = null;
  if (compiler) {
    effectsWatch = watchAppEffects({
      appRoot: config.appRoot,
      out: effectsDir,
      compiler,
      onBuilt: (effects) => {
        console.log(
          `${c.cyan("↻")} effects rebuilt (${effects.compiled.map((e) => e.effect.name).join(", ")}) — restarting the app`,
        );
        restarting = true;
        host.kill();
      },
      onError: (text) => {
        console.error(`${c.red("effects")}: ${text}`);
        console.error(c.dim("the app keeps running the last good build"));
      },
    });
  }
  const stop = () => {
    effectsWatch?.close();
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
  const code: number = await new Promise((done) => {
    const exited = (status: number | null) => {
      if (!restarting) return done(status ?? 0);
      restarting = false;
      host = start();
      host.on("exit", exited);
    };
    host.on("exit", exited);
  });
  stop();
  return code;
}
