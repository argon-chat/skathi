// Packaging, held down from outside the process (docs/PACKAGING.md).
//
//   bun test packages/vn-test/specs/package.spec.ts
//
// `vn build` turns an app directory into ONE exe by stamping a payload into a
// prebuilt bare host — no Rust toolchain. Every claim below is about the
// shipped artifact, run the way a user runs it:
//
// 1. THE BUILD NEEDS NO TOOLCHAIN. `vn build` runs with every cargo/rustc
//    directory taken off PATH and CARGO_HOME/RUSTUP_HOME pointed at nothing,
//    and the exe it writes carries the app's name, icon and version info and
//    the GUI subsystem (no console window).
// 2. THE PACKAGE IS THE DOCUMENT. `fetch`, `<img>`, `<video>` and `<lottie>`
//    resolve the same relative URLs out of the payload that dev resolves
//    against the app root, and a miss says so instead of reading the cwd.
// 3. IDENTITY IS THE vn.config NAME. The handshake lives in
//    %TEMP%\vue-native\apps\<name>\<pid> (never beside the exe), storage in
//    %LOCALAPPDATA%\<name>\storage — even though the exe launched here is a
//    hard link named `vn-app-vntest-…` (the reaper's pattern), not `Hello.exe`.
// 4. TWO APPS, TWO STORES, side by side.
// 5. A STAMPED EXE CAN BE RE-STAMPED, and keeps its icon and version info.
// 6. THE no-devtools HOST HAS NO LISTENER: no handshake, no listening port,
//    and not even the inspector's window title in the image.
//
// Hosts: whatever `vn build` itself would find (VN_HOST, the in-repo release
// exe, the local cache from `vn host build`); a debug feature build stands in
// when there is none (src/features.ts `bareHost`).

import { afterAll, beforeAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, linkSync, mkdirSync, readFileSync, rmSync, cpSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { bareHost, fileContains, launch, packagedSlot, readPng, REPO, WORK_DIR, type VnApp } from "../src/index";
import { readPe, RT, shellIconCount, SUBSYSTEM, versionString } from "../../vue-native/cli/pe";
import { PAYLOAD_NAME, stamp } from "../../vue-native/cli/stamp";
import { readVnpk, writeVnpk, type VnpkInput } from "../../vue-native/cli/vnpk";
import { buildApp } from "../../vue-native/cli/build-app";

const T = 60_000;
/** A missing host is built (debug) once — see `bareHost`. */
const HOST_T = 20 * 60_000;
const CLI = join(REPO, "packages", "vue-native", "cli", "vn.ts");
const OUT = join(WORK_DIR, "packaged", `spec-${process.pid}`);
/** The inspector's window title — present only in an image with the devtools. */
const MARKER = "vue-native devtools";
const PROBE_KEY = `vn-package-spec-${process.pid}`;

let standardHost = "";
let noDevtoolsHost = "";
const built: Record<string, { exe: string; json: any; ms: number }> = {};
const strays: ChildProcess[] = [];
const opened: VnApp[] = [];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

afterAll(async () => {
  for (const app of opened) await app.close();
  for (const child of strays) {
    if (child.pid) spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  }
  await sleep(300);
  rmSync(OUT, { recursive: true, force: true });
});

/** The environment of a machine with NO Rust toolchain at all. */
function noRustEnv(): Record<string, string> {
  const env = { ...(process.env as Record<string, string>) };
  const pathKey = Object.keys(env).find((k) => k.toLowerCase() === "path") ?? "PATH";
  env[pathKey] = (env[pathKey] ?? "")
    .split(delimiter)
    .filter((dir) => dir && !/[\\/]\.(cargo|rustup)[\\/]?/i.test(dir))
    .filter((dir) => !["cargo.exe", "rustc.exe", "rustup.exe"].some((f) => existsSync(join(dir, f))))
    .join(delimiter);
  const nowhere = join(tmpdir(), `vn-no-rust-${process.pid}`);
  env.CARGO_HOME = nowhere;
  env.RUSTUP_HOME = nowhere;
  delete env.VN_HOST;
  return env;
}

/** `vn build <app> --host <host> --out <dir> --json`, as a subprocess with no toolchain. */
function cliBuild(app: string, host: string, variant: string, label: string) {
  const out = join(OUT, label);
  const t0 = performance.now();
  const r = spawnSync(
    process.execPath,
    [CLI, "build", join(REPO, "apps", app), "--host", host, "--variant", variant, "--out", out, "--json"],
    { cwd: REPO, encoding: "utf8", env: noRustEnv() },
  );
  const ms = Math.round(performance.now() - t0);
  if (r.status !== 0) throw new Error(`vn build ${app} failed (${r.status})\n${r.stdout}\n${r.stderr}`);
  const json = JSON.parse(r.stdout.trim().split(/\r?\n/).pop()!);
  return { exe: json.exe as string, json, ms };
}

/** A hard link under the reaper's name — the same image, a name `reap.ts` owns. */
function runnable(exe: string, label: string): string {
  const link = join(dirname(exe), `vn-app-vntest-${process.pid}-${label}.exe`);
  rmSync(link, { force: true });
  try {
    linkSync(exe, link);
  } catch {
    cpSync(exe, link);
  }
  return link;
}

async function until<V>(what: string, probe: () => Promise<V | null | undefined | false>, ms = 20_000): Promise<V> {
  const deadline = Date.now() + ms;
  for (;;) {
    const v = await probe().catch(() => null);
    if (v) return v as V;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await sleep(150);
  }
}

/** Runs `js` (which must eventually set `globalThis[slot]`) and waits for the value. */
async function evalAsync(app: VnApp, js: string, slot: string): Promise<any> {
  await app.eval(`globalThis.${slot} = undefined; ${js}; 'ok'`);
  return until(`${slot}`, async () => {
    const v = await app.eval(`JSON.stringify(globalThis.${slot} ?? null)`);
    return v === "null" ? null : { v: JSON.parse(v) };
  }).then((x) => x.v);
}

beforeAll(async () => {
  mkdirSync(OUT, { recursive: true });
  standardHost = await bareHost("standard", console.log);
  noDevtoolsHost = await bareHost("no-devtools", console.log);
}, HOST_T);

describe("vn build", () => {
  test("hello and shadcn each become one exe with no Rust toolchain on the machine", () => {
    built.hello = cliBuild("hello", standardHost, "standard", "hello");
    built.shadcn = cliBuild("shadcn", standardHost, "standard", "shadcn");
    console.log(
      `[package.spec] vn build: hello ${built.hello.ms} ms (stamp ${built.hello.json.stamp.ms.total} ms), ` +
        `shadcn ${built.shadcn.ms} ms (stamp ${built.shadcn.json.stamp.ms.total} ms)`,
    );

    const cases = [
      { key: "hello", file: "Hello.exe", product: "Hello", version: "0.1.0.0" },
      { key: "shadcn", file: "ShadcnGallery.exe", product: "shadcn-vue gallery", version: "0.0.1.0" },
    ];
    for (const c of cases) {
      const exe = built[c.key]!.exe;
      bunExpect(exe.endsWith(c.file)).toBe(true);
      const pe = readPe(exe);
      // GUI: Explorer opens no console window for it.
      bunExpect(pe.subsystem).toBe(SUBSYSTEM.GUI);
      const has = (type: number, name: number | string) => pe.resources.some((r) => r.type === type && r.name === name);
      bunExpect(has(RT.RCDATA, PAYLOAD_NAME)).toBe(true);
      // gpui's window class loads icon 1 — the taskbar and Alt-Tab icon.
      bunExpect(has(RT.GROUP_ICON, 1)).toBe(true);
      bunExpect(shellIconCount(exe)).toBeGreaterThanOrEqual(1);
      bunExpect(versionString(exe, "ProductName")).toBe(c.product);
      bunExpect(versionString(exe, "FileDescription")).toBe(c.product);
      bunExpect(versionString(exe, "FileVersion")).toBe(c.version);
      bunExpect(versionString(exe, "OriginalFilename")).toBe(c.file);
      bunExpect(built[c.key]!.json.stamp.dataAligned).toBe(true);
    }
    bunExpect(built.hello!.json.assets).toBeGreaterThan(0);
  }, T);
});

describe("a packaged hello", () => {
  let app: VnApp;
  let exe = "";

  test("starts from its payload and publishes its handshake under %TEMP%, never beside the exe", async () => {
    exe = runnable(built.hello!.exe, "hello");
    app = await launch({ app: "hello", exe, packaged: "hello", window: [980, 720], label: "pkg-hello" });
    opened.push(app);
    bunExpect(app.slotDir).toBe(packagedSlot("hello", app.pid));
    bunExpect(app.slotDir.startsWith(join(tmpdir(), "vue-native", "apps", "hello"))).toBe(true);
    bunExpect(existsSync(join(dirname(exe), "devtools.json"))).toBe(false);
    // A prod bundle, not a dev host that happened to be staged.
    bunExpect(await app.eval("typeof globalThis.__VN__")).toBe("undefined");
  }, T);

  test("fetch() reads relative assets out of the payload; a miss is a 404 with an advisory", async () => {
    const text = await evalAsync(app, "fetch('assets/hello.txt').then(r => r.text()).then(t => globalThis.__pkgText = t)", "__pkgText");
    bunExpect(text).toBe(readFileSync(join(REPO, "apps", "hello", "assets", "hello.txt"), "utf8"));
    const dotted = await evalAsync(app, "fetch('./assets/hello.txt?v=2').then(r => r.status).then(s => globalThis.__pkgDot = s)", "__pkgDot");
    bunExpect(dotted).toBe(200);
    const miss = await evalAsync(app, "fetch('assets/not-packaged.txt').then(r => r.status).then(s => globalThis.__pkgMiss = s)", "__pkgMiss");
    bunExpect(miss).toBe(404);
    // `launch()` starts the exe with the REPO as its cwd, where Cargo.toml
    // exists: a miss must not fall through to the working directory.
    bunExpect(existsSync(join(REPO, "Cargo.toml"))).toBe(true);
    const cwd = await evalAsync(app, "fetch('Cargo.toml').then(r => r.status).then(s => globalThis.__pkgCwd = s)", "__pkgCwd");
    bunExpect(cwd).toBe(404);
    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => e.kind === "advisory" && e.text.includes("not in this app's package"))).toBe(true);
  }, T);

  test("<img> and <video> resolve relative srcs out of the payload", async () => {
    await app.locator(".tab").nth(3).click(); // Media
    await until("the 1080p clip's metadata", async () => /1920×1080/.test(await app.locator(".meta").first().textContent()));
    await app.eval("document.querySelector('.relative-img').scrollIntoView(); 'ok'");
    await sleep(600);
    const box = (await app.query(".relative-img")).nodes[0]!.bounds!;
    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const rect = { x: (box.x + 6) * scale, y: (box.y + 6) * scale, w: (box.w - 12) * scale, h: (box.h - 12) * scale };
    // The poster is SMPTE colour bars: an alt-text box is grey, the picture is
    // mostly saturated colour.
    const saturated = shot.count(rect, (_l, [r, g, b]) => Math.max(r, g, b) - Math.min(r, g, b) > 80);
    bunExpect(saturated / (rect.w * rect.h)).toBeGreaterThan(0.25);
    const { entries } = await app.logs(0);
    bunExpect(entries.filter((e) => e.kind === "advisory" && e.text.includes('src="assets/'))).toEqual([]);
  }, T);

  test("<lottie> plays a .tgs out of the payload", async () => {
    await app.locator(".tab").nth(2).click(); // Motion
    await until("a completed lottie loop", async () => /fired [1-9]\d*×/.test(await app.locator(".loops").textContent()), 20_000);
  }, T);

  test("localStorage lives under %LOCALAPPDATA%\\<vn.config name>\\storage, not the exe's name", async () => {
    const info = JSON.parse(await app.eval("__vn_storage_info()"));
    bunExpect(info.folder.toLowerCase().endsWith("\\hello\\storage")).toBe(true);
    bunExpect(info.folder.toLowerCase()).not.toContain("vn-app-vntest");
  }, T);

  test("the app logged nothing broken", async () => {
    await app.expectNoErrors({ ignore: [/not in this app's package/, /127\.0\.0\.1:8731/] });
  }, T);
});

describe("two packaged apps side by side", () => {
  test("hello and shadcn run at once with separate storage", async () => {
    const hello = await launch({ app: "hello", exe: runnable(built.hello!.exe, "hello-b"), packaged: "hello", label: "pkg-hello-b" });
    opened.push(hello);
    const shadcn = await launch({ app: "shadcn", exe: runnable(built.shadcn!.exe, "shadcn"), packaged: "shadcn", label: "pkg-shadcn" });
    opened.push(shadcn);
    try {
      bunExpect(hello.slotDir).not.toBe(shadcn.slotDir);
      const hi = JSON.parse(await hello.eval("__vn_storage_info()"));
      const sh = JSON.parse(await shadcn.eval("__vn_storage_info()"));
      bunExpect(sh.folder.toLowerCase().endsWith("\\shadcn\\storage")).toBe(true);
      bunExpect(hi.folder).not.toBe(sh.folder);

      await hello.eval(`localStorage.setItem(${JSON.stringify(PROBE_KEY)}, 'from hello'); __vn_storage_flush()`);
      bunExpect(await hello.eval(`localStorage.getItem(${JSON.stringify(PROBE_KEY)})`)).toBe("from hello");
      bunExpect(await shadcn.eval(`String(localStorage.getItem(${JSON.stringify(PROBE_KEY)}))`)).toBe("null");
      const after = JSON.parse(await hello.eval("__vn_storage_info()"));
      if (!after.encrypted) {
        bunExpect(readFileSync(after.file, "utf8")).toContain(PROBE_KEY);
        const shFile = JSON.parse(await shadcn.eval("__vn_storage_info()")).file;
        bunExpect(existsSync(shFile) ? readFileSync(shFile, "utf8") : "").not.toContain(PROBE_KEY);
      }
    } finally {
      // Leave the user's real hello store as it was.
      await hello.eval(`localStorage.removeItem(${JSON.stringify(PROBE_KEY)}); __vn_storage_flush()`).catch(() => {});
      await hello.close();
      await shadcn.close();
    }
  }, T);
});

describe("re-stamping", () => {
  test("a stamped exe takes a new payload and keeps its icon and version info", async () => {
    const fixture = join(REPO, "packages", "vn-test", "fixtures", "devtools-on");
    const work = join(OUT, "restamp");
    const r = await buildApp({ target: fixture, out: work, payloadOnly: true, log: () => {} });
    const v = readVnpk(new Uint8Array(readFileSync(r.vnpk)));
    const inputs: VnpkInput[] = v.entries.map((e) => ({ name: e.name, kind: e.kind, data: v.bytes(e), alignLog2: e.alignLog2 }));
    const out = join(work, "Hello-restamped.exe");
    const s = stamp({ host: built.hello!.exe, out, payload: (skew) => writeVnpk(inputs, { skew }) });
    bunExpect(s.dataAligned).toBe(true);
    bunExpect(versionString(out, "ProductName")).toBe("Hello");
    bunExpect(shellIconCount(out)).toBeGreaterThanOrEqual(1);
    bunExpect(readPe(out).subsystem).toBe(SUBSYSTEM.GUI);

    const app = await launch({ app: "devtools-on", exe: runnable(out, "restamped"), packaged: "devtools-on", label: "pkg-restamp" });
    opened.push(app);
    try {
      await until("the fixture's text", async () => /devtools: on/.test(await app.locator(".fixture").textContent()));
      bunExpect(await app.eval("typeof globalThis.__VN_TEST_DEVTOOLS__")).toBe("object");
    } finally {
      await app.close();
    }
  }, T);
});

describe("the no-devtools host", () => {
  test("publishes no handshake, listens on no port, and does not contain the inspector", async () => {
    const nd = cliBuild("hello", noDevtoolsHost, "no-devtools", "hello-nodev");
    bunExpect(nd.json.variant).toBe("no-devtools");
    bunExpect(await fileContains(built.hello!.exe, MARKER)).toBe(true);
    bunExpect(await fileContains(nd.exe, MARKER)).toBe(false);

    const exe = runnable(nd.exe, "hello-nodev");
    const env: Record<string, string> = { ...(process.env as Record<string, string>), VN_OWNER_PID: String(process.pid) };
    for (const k of ["VN_DIST", "VN_APP_ROOT", "VN_PAYLOAD"]) delete env[k];
    const child = spawn(exe, [], { env, stdio: "ignore" });
    strays.push(child);
    // Long enough that a host which was going to publish a handshake has.
    await sleep(8000);
    bunExpect(child.exitCode).toBeNull();
    const slot = packagedSlot("hello", child.pid!);
    bunExpect(existsSync(join(slot, "devtools.json"))).toBe(false);
    const net = spawnSync("netstat", ["-ano", "-p", "TCP"], { encoding: "utf8" }).stdout ?? "";
    const listening = net
      .split(/\r?\n/)
      .filter((l) => /LISTENING/.test(l) && l.trim().split(/\s+/).pop() === String(child.pid));
    bunExpect(listening).toEqual([]);
    spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  }, T);
});
