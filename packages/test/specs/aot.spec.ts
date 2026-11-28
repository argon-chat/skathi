// `sk build --aot`: apps/hello compiled to a native image, stamped into an AOT-capable host and run
// through the devtools protocol. Proves the app mounted from the image, not from app.prod.js, and
// compares startup with the same exe told to run its JavaScript (SK_APP_ENGINE=jit).
//
// Needs a standard host with the native AOT loader (a release host, or `cargo build --release -p sk-app
// --features prod,native-aot` with REDJS_AOT_BUILTIN_TABLE_DIR) and a matching sk-aot (SK_AOT_COMPILER, the
// checkout's tools/sk-aot build, or the release's). Without them it skips and says why.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { bareHost, expect, launch, refuseStaleBareHost, REPO, WORK_DIR, type SkApp } from "../src/index";
import { readPayload } from "../../cli/cli/payload";
import { KIND, NAMES } from "../../cli/cli/skpk";
import { hostInfo } from "../../cli/cli/host";
import type { BuildResult } from "../../cli/cli/build-app";

const T = 60_000;
const BUILD_T = 20 * 60_000;

async function setup(): Promise<{ host: string } | { skip: string }> {
  if (process.platform !== "win32" || process.arch !== "x64") return { skip: "AOT builds are Windows x64 only" };
  await refuseStaleBareHost("standard");
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
    "apps/hello compiles to a native image in a container beside the exe, its manifest in the payload",
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
      bunExpect(basename(build.native!.compiler)).toBe("sk-aot.exe");
      const native = build.native!;
      const payload = readPayload(new Uint8Array(readFileSync(build.skpk)));
      const kinds = payload.entries.map((e) => [e.kind, e.name]);
      bunExpect(kinds).toContainEqual([KIND.NATIVE_MANIFEST, NAMES.NATIVE_MANIFEST]);
      bunExpect(kinds).not.toContainEqual([KIND.NATIVE_IMAGE, NAMES.NATIVE_IMAGE]);
      bunExpect(native.container).toBe(join(dirname(build.exe!), "app.image.dll"));
      bunExpect(existsSync(native.container!)).toBe(true);
      bunExpect(kinds).toContainEqual([KIND.SCRIPT, NAMES.SCRIPT]);
      const manifest = JSON.parse(readFileSync(native.manifest, "utf8")) as Record<string, unknown>;
      bunExpect(manifest.kind).toBe("application");
      bunExpect(manifest.imageBytes).toBe(native.imageBytes);
      console.log(
        `[aot.spec] bundle ${(build.bundleBytes / 1024).toFixed(0)} KiB -> image ${(native.imageBytes / 2 ** 20).toFixed(1)} MiB ` +
          `in ${native.container ?? "the exe"}, sk-aot ${native.ms} ms`,
      );
    },
    BUILD_T,
  );

  test(
    "the exe mounts hello from the image where it lies, and expands nothing",
    async () => {
      const { app, mounted } = await run(build, "aot-hello", { SK_AOT_CACHE_DIR: join(caches, "native") });
      await app.locator(".tab").nth(2).click();
      await expect(app.locator(".card .title")).toContainText("Motion");
      bunExpect(await app.eval("typeof globalThis.__SK_AOT_HOST__")).toBe("object");
      bunExpect(lines(app, /\[native-aot\] selected application image .*app\.image\.dll/)).toHaveLength(1);
      bunExpect(existsSync(join(caches, "native"))).toBe(false);
      bunExpect(lines(app, /\[native-aot\] application native entry completed/)).toHaveLength(1);
      bunExpect(lines(app, /\[bytecode\]/)).toEqual([]);
      report(app, "AOT", mounted);
      await app.close();
    },
    T,
  );

  test(
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

  // The runtime wires these by `typeof` of host functions that the preludes and the host bind before
  // the entry, outside the image.
  test(
    "host functions bound outside the image wire focus() and the native locale",
    async () => {
      const root = join(WORK_DIR, `aot-probe-${process.pid}`);
      mkdirSync(join(root, "src"), { recursive: true });
      writeFileSync(
        join(root, "sk.config.json"),
        JSON.stringify({ name: "aotprobe", productName: "AotProbe", version: "0.1.0" }),
      );
      writeFileSync(
        join(root, "src", "main.ts"),
        [
          'import { appLocale, createApp, h } from "vue";',
          'appLocale.define({ en: { "aot.probe": "from the host catalog" } });',
          'createApp({ render: () => h("input", { class: "probe" }) }).mount();',
        ].join("\n"),
      );
      const { buildApp } = await import("../../cli/cli/build-app");
      const probe = await buildApp({
        target: root,
        out: join(WORK_DIR, "packaged", `aot-probe-${process.pid}`),
        host: "host" in ready ? ready.host : undefined,
        variant: "standard",
        aot: true,
        log: (l) => console.log(`[aot.spec] ${l}`),
      });
      const app = await launch({
        app: "aot-probe",
        exe: probe.exe!,
        packaged: probe.config.name,
        label: "aot-probe",
        window: [600, 400],
        env: { SK_AOT_CACHE_DIR: join(caches, "native") },
      });
      opened.push(app);
      await expect(app.locator(".probe")).toHaveCount(1);
      bunExpect(lines(app, /\[native-aot\] application native entry completed/)).toHaveLength(1);
      bunExpect(await app.eval(`__sk_appLocale.t("aot.probe")`)).toBe("from the host catalog");
      bunExpect(
        await app.eval(
          `(() => { const input = document.querySelector(".probe"); input.focus(); return document.activeElement === input; })()`,
        ),
      ).toBe(true);
      bunExpect(lines(app, /no programmatic focus/)).toEqual([]);
      await app.close();
    },
    BUILD_T,
  );

  test(
    "an audio worklet runs beside the image, in a VM of its own",
    async () => {
      const { buildApp } = await import("../../cli/cli/build-app");
      const built = await buildApp({
        target: join(REPO, "packages", "test", "fixtures", "audio-worklet"),
        out: join(WORK_DIR, "packaged", `aot-worklet-${process.pid}`),
        host: "host" in ready ? ready.host : undefined,
        variant: "standard",
        aot: true,
        log: (l) => console.log(`[aot.spec] ${l}`),
      });
      const app = await launch({
        app: "aot-worklet",
        exe: built.exe!,
        packaged: built.config.name,
        label: "aot-worklet",
        window: [360, 200],
        env: { SK_AOT_CACHE_DIR: join(caches, "native") },
      });
      opened.push(app);
      await expect(app.locator("#root")).toHaveText("audio worklet");
      bunExpect(lines(app, /\[native-aot\] application native entry completed/)).toHaveLength(1);
      await app.eval(
        `globalThis.__awl = ""; (async () => {
          const fixture = globalThis.__SK_TEST_WORKLET__;
          const ctx = new AudioContext();
          await fixture.load(ctx);
          const tone = new AudioWorkletNode(ctx, "tone", { numberOfInputs: 0 });
          const echo = new AudioWorkletNode(ctx, "echo", { numberOfInputs: 0 });
          const meter = ctx.createAnalyser();
          tone.connect(meter);
          const answers = [await fixture.ask(tone, "ping"), await fixture.ask(echo, "pong")];
          await new Promise((r) => setTimeout(r, 500));
          const b = new Float32Array(1024);
          meter.getFloatTimeDomainData(b);
          const rms = Math.sqrt(b.reduce((a, v) => a + v * v, 0) / b.length);
          const moved = ctx.currentTime > 0;
          await ctx.close();
          return { answers: answers.map((a) => a.answer), heard: moved ? Math.abs(rms - 0.25) < 1e-3 : true };
        })().then((v) => { globalThis.__awl = JSON.stringify(v); }, (e) => { globalThis.__awl = JSON.stringify(String(e)); }); "started"`,
      );
      let raw = "";
      for (const until = Date.now() + 20_000; !raw && Date.now() < until; await Bun.sleep(50))
        raw = String(await app.eval("globalThis.__awl"));
      bunExpect(JSON.parse(raw)).toEqual({ answers: ["tone:ping", "echo:pong"], heard: true });
      await app.close();
    },
    BUILD_T,
  );

  test(
    "an app that imports nothing runs from its image and sees the host's functions",
    async () => {
      const root = join(WORK_DIR, `aot-bare-${process.pid}`);
      mkdirSync(join(root, "src"), { recursive: true });
      writeFileSync(
        join(root, "sk.config.json"),
        JSON.stringify({ name: "aotbare", productName: "AotBare", version: "0.1.0" }),
      );
      writeFileSync(
        join(root, "src", "main.ts"),
        [
          "declare const __sk_fullscreen_window: unknown;",
          'console.log("aot-bare", typeof __sk_fullscreen_window);',
        ].join("\n"),
      );
      const { buildApp } = await import("../../cli/cli/build-app");
      const bare = await buildApp({
        target: root,
        out: join(WORK_DIR, "packaged", `aot-bare-${process.pid}`),
        host: "host" in ready ? ready.host : undefined,
        variant: "standard",
        aot: true,
        log: (l) => console.log(`[aot.spec] ${l}`),
      });
      const app = await launch({
        app: "aot-bare",
        exe: bare.exe!,
        packaged: bare.config.name,
        label: "aot-bare",
        window: [400, 300],
        waitForMount: false,
        env: { SK_AOT_CACHE_DIR: join(caches, "native") },
      });
      opened.push(app);
      const deadline = Date.now() + 20_000;
      while (!lines(app, /aot-bare function/).length && Date.now() < deadline)
        await new Promise((r) => setTimeout(r, 100));
      bunExpect(lines(app, /aot-bare function/)).toHaveLength(1);
      bunExpect(lines(app, /\[native-aot\] application native entry completed/)).toHaveLength(1);
      await app.close();
    },
    BUILD_T,
  );
});
