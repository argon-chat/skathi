// `sk build --aot`: apps/hello compiled to a native image, stamped into an AOT-capable host and run
// through the devtools protocol. Proves the app mounted from the image, not from app.prod.js, and
// compares startup with the same exe told to run its JavaScript (SK_APP_ENGINE=jit).
//
// Needs a standard host with the native AOT loader (a release host, or `cargo build --release -p sk-app
// --features prod,native-aot` with REDJS_AOT_BUILTIN_TABLE_DIR) and a matching image compiler (the
// checkout's aot-image example, SK_AOT_COMPILER, or the release's). Without them it skips and says why.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { bareHost, expect, launch, REPO, WORK_DIR, type SkApp } from "../src/index";
import { KIND, NAMES, readSkpk } from "../../cli/cli/skpk";
import { hostInfo } from "../../cli/cli/host";
import type { BuildResult } from "../../cli/cli/build-app";

const T = 60_000;
const BUILD_T = 20 * 60_000;

async function setup(): Promise<{ host: string } | { skip: string }> {
  if (process.platform !== "win32" || process.arch !== "x64") return { skip: "AOT builds are Windows x64 only" };
  const host = await bareHost("standard").catch((e: Error) => e.message);
  if (!host.endsWith(".exe")) return { skip: `no standard host: ${host}` };
  if (!hostInfo(host).aot) return { skip: `${host} was built without the native AOT loader (native-aot)` };
  return { host };
}

const ready = await setup();
if ("skip" in ready) console.warn(`[aot.spec] skipped: ${ready.skip}`);
const opened: SkApp[] = [];
const caches = join(WORK_DIR, `aot-caches-${process.pid}`);
afterAll(async () => {
  for (const app of opened) await app.close();
  rmSync(caches, { recursive: true, force: true });
});

const lines = (app: SkApp, pattern: RegExp) =>
  readFileSync(app.hostLog, "utf8")
    .split(/\r?\n/)
    .filter((l) => pattern.test(l))
    .map((l) => l.trim());

async function run(build: BuildResult, label: string, env: Record<string, string>) {
  const t0 = performance.now();
  const app = await launch({
    app: label,
    exe: build.exe!,
    packaged: build.config.name,
    label,
    window: [1100, 820],
    env: { SK_PERF: "1", ...env },
  });
  opened.push(app);
  await expect(app.locator(".tab").nth(0)).toHaveText("Forms");
  const mounted = Math.round(performance.now() - t0);
  return { app, mounted };
}

function report(app: SkApp, title: string, mounted: number) {
  console.log(`[aot.spec] ${title}: mounted ${mounted} ms after spawn`);
  for (const l of lines(app, /\[native-aot\]|\[bytecode\]|boot->first-frame|boot js/)) console.log(`[aot.spec]   ${l}`);
}

describe.skipIf("skip" in ready)("sk build --aot", () => {
  let build: BuildResult;

  test(
    "apps/hello compiles to a native image that travels in the payload",
    async () => {
      const { buildApp } = await import("../../cli/cli/build-app");
      build = await buildApp({
        target: join(REPO, "apps", "hello"),
        out: join(WORK_DIR, "packaged", `aot-${process.pid}`),
        host: "host" in ready ? ready.host : undefined,
        variant: "standard",
        aot: true,
        log: (l) => console.log(`[aot.spec] ${l}`),
      });
      bunExpect(build.native).not.toBeNull();
      const native = build.native!;
      const payload = readSkpk(new Uint8Array(readFileSync(build.skpk)));
      const kinds = payload.entries.map((e) => [e.kind, e.name]);
      bunExpect(kinds).toContainEqual([KIND.NATIVE_MANIFEST, NAMES.NATIVE_MANIFEST]);
      bunExpect(kinds).toContainEqual([KIND.NATIVE_IMAGE, NAMES.NATIVE_IMAGE]);
      bunExpect(kinds).toContainEqual([KIND.SCRIPT, NAMES.SCRIPT]);
      const manifest = JSON.parse(readFileSync(native.manifest, "utf8")) as Record<string, unknown>;
      bunExpect(manifest.kind).toBe("application");
      bunExpect(manifest.imageBytes).toBe(native.imageBytes);
      console.log(
        `[aot.spec] bundle ${(build.bundleBytes / 1024).toFixed(0)} KiB -> image ${(native.imageBytes / 2 ** 20).toFixed(1)} MiB ` +
          `(${(native.compressedBytes / 2 ** 20).toFixed(1)} MiB packed), frontend + compiler ${native.ms} ms`,
      );
    },
    BUILD_T,
  );

  test.each(["cold", "warm"])(
    "the exe mounts hello from the native image (%s image cache)",
    async (cache) => {
      const { app, mounted } = await run(build, `aot-hello-${cache}`, { SK_AOT_CACHE_DIR: join(caches, "native") });
      await app.locator(".tab").nth(2).click();
      await expect(app.locator(".card .title")).toContainText("Motion");
      bunExpect(await app.eval("typeof globalThis.__SK_AOT_HOST__")).toBe("object");
      bunExpect(lines(app, /\[native-aot\] selected application image/)).toHaveLength(1);
      bunExpect(lines(app, /\[native-aot\] application native entry completed/)).toHaveLength(1);
      bunExpect(lines(app, /\[bytecode\]/)).toEqual([]);
      report(app, `AOT (${cache} image cache)`, mounted);
      await app.close();
    },
    T,
  );

  // In the image, `typeof` of a global-object property the builtin table does not list answers
  // "undefined", so the runtime's `typeof __sk_reload === "function"` never wires location.reload.
  test.failing(
    "location.reload() runs app.prod.js, since the native entry evaluates once per process",
    async () => {
      const { app } = await run(build, "aot-hello-reload", { SK_AOT_CACHE_DIR: join(caches, "native") });
      bunExpect(await app.eval(`(() => { location.reload(); return "asked"; })()`)).toBe("asked");
      const deadline = Date.now() + 20_000;
      while (!lines(app, /\[native-aot\] reload: running app\.prod\.js/).length && Date.now() < deadline)
        await new Promise((r) => setTimeout(r, 100));
      bunExpect(lines(app, /\[native-aot\] reload: running app\.prod\.js/)).toHaveLength(1);
      await expect(app.locator(".tab").nth(0)).toHaveText("Forms");
      await app.locator(".tab").nth(2).click();
      await expect(app.locator(".card .title")).toContainText("Motion");
      await app.close();
    },
    T,
  );

  test.each(["cold", "warm"])(
    "SK_APP_ENGINE=jit runs app.prod.js from the same exe (%s bytecode cache)",
    async (cache) => {
      const { app, mounted } = await run(build, `aot-hello-jit-${cache}`, {
        SK_APP_ENGINE: "jit",
        SK_BYTECODE_CACHE_DIR: join(caches, "jsc"),
      });
      bunExpect(await app.eval("typeof globalThis.__SK_AOT_HOST__")).toBe("undefined");
      bunExpect(lines(app, /SK_APP_ENGINE=jit/)).toHaveLength(1);
      bunExpect(lines(app, /selected application image/)).toEqual([]);
      report(app, `JIT (${cache} bytecode cache)`, mounted);
      await app.close();
    },
    T,
  );
});
