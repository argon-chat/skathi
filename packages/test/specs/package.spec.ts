import { afterAll, beforeAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, linkSync, mkdirSync, readFileSync, rmSync, cpSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, sep } from "node:path";
import {
  bareHost,
  fileContains,
  killTree,
  launch,
  MACOS,
  packagedSlot,
  readPng,
  REPO,
  SCREENSHOTS,
  type SkApp,
  WINDOWS,
  WORK_DIR,
} from "../src/index";
import { readPe, RT, shellIconCount, SUBSYSTEM, versionString } from "../../cli/cli/pe";
import { PAYLOAD_NAME, stamp } from "../../cli/cli/stamp";
import { readPayload } from "../../cli/cli/payload";
import { writeSkpk, type SkpkInput } from "../../cli/cli/skpk";
import { buildApp } from "../../cli/cli/build-app";

const T = 60_000;
const HOST_T = 20 * 60_000;
const CLI = join(REPO, "packages", "cli", "cli", "sk.ts");
const OUT = join(WORK_DIR, "packaged", `spec-${process.pid}`);
const MARKER = "Skathi devtools";
const PROBE_KEY = `sk-package-spec-${process.pid}`;

let standardHost = "";
let noDevtoolsHost = "";
const built: Record<string, { exe: string; json: any; ms: number }> = {};
const strays: ChildProcess[] = [];
const opened: SkApp[] = [];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

afterAll(async () => {
  for (const app of opened) await app.close();
  for (const child of strays) killTree(child);
  await sleep(300);
  rmSync(OUT, { recursive: true, force: true });
});

function noRustEnv(): Record<string, string> {
  const env = { ...(process.env as Record<string, string>) };
  const pathKey = Object.keys(env).find((k) => k.toLowerCase() === "path") ?? "PATH";
  env[pathKey] = (env[pathKey] ?? "")
    .split(delimiter)
    .filter((dir) => dir && !/[\\/]\.(cargo|rustup)[\\/]?/i.test(dir))
    .filter((dir) => !["cargo.exe", "rustc.exe", "rustup.exe"].some((f) => existsSync(join(dir, f))))
    .join(delimiter);
  const nowhere = join(tmpdir(), `sk-no-rust-${process.pid}`);
  env.CARGO_HOME = nowhere;
  env.RUSTUP_HOME = nowhere;
  delete env.SK_HOST;
  return env;
}

function cliBuild(app: string, host: string, variant: string, label: string) {
  const out = join(OUT, label);
  const t0 = performance.now();
  const r = spawnSync(
    process.execPath,
    [CLI, "build", join(REPO, "apps", app), "--host", host, "--variant", variant, "--out", out, "--json"],
    { cwd: REPO, encoding: "utf8", env: noRustEnv() },
  );
  const ms = Math.round(performance.now() - t0);
  if (r.status !== 0) throw new Error(`sk build ${app} failed (${r.status})\n${r.stdout}\n${r.stderr}`);
  const json = JSON.parse(r.stdout.trim().split(/\r?\n/).pop()!);
  return { exe: json.exe as string, json, ms };
}

function runnable(exe: string, label: string): string {
  // A macOS binary finds its payload only inside its .app bundle.
  if (process.platform === "darwin") return exe;
  const link = join(dirname(exe), `sk-app-sktest-${process.pid}-${label}.exe`);
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
async function evalAsync(app: SkApp, js: string, slot: string): Promise<any> {
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

describe("sk build", () => {
  test(
    "hello and shadcn each become one exe with no Rust toolchain on the machine",
    () => {
      built.hello = cliBuild("hello", standardHost, "standard", "hello");
      built.shadcn = cliBuild("shadcn", standardHost, "standard", "shadcn");
      console.log(
        `[package.spec] sk build: hello ${built.hello.ms} ms (stamp ${built.hello.json.stamp?.ms.total} ms), ` +
          `shadcn ${built.shadcn.ms} ms (stamp ${built.shadcn.json.stamp?.ms.total} ms)`,
      );

      const cases = [
        { key: "hello", file: "Hello.exe", product: "Hello", version: "0.1.0.0" },
        { key: "shadcn", file: "ShadcnGallery.exe", product: "shadcn-vue gallery", version: "0.0.1.0" },
      ];
      // The exe checks read a PE; elsewhere the build succeeding is the check.
      for (const c of process.platform === "win32" ? cases : []) {
        const exe = built[c.key]!.exe;
        bunExpect(exe.endsWith(c.file)).toBe(true);
        const pe = readPe(exe);
        bunExpect(pe.subsystem).toBe(SUBSYSTEM.GUI);
        const has = (type: number, name: number | string) =>
          pe.resources.some((r) => r.type === type && r.name === name);
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
    },
    T,
  );
});

describe("a packaged hello", () => {
  let app: SkApp;
  let exe = "";

  test(
    "starts from its payload and publishes its handshake under %TEMP%, never beside the exe",
    async () => {
      exe = runnable(built.hello!.exe, "hello");
      app = await launch({ app: "hello", exe, packaged: "hello", window: [980, 720], label: "pkg-hello" });
      opened.push(app);
      bunExpect(app.slotDir).toBe(packagedSlot("hello", app.pid));
      bunExpect(app.slotDir.startsWith(join(tmpdir(), "skathi", "apps", "hello"))).toBe(true);
      bunExpect(existsSync(join(dirname(exe), "devtools.json"))).toBe(false);
      bunExpect(await app.eval("typeof globalThis.__SK__")).toBe("undefined");
    },
    T,
  );

  test(
    "fetch() reads relative assets out of the payload; a miss is a 404 with an advisory",
    async () => {
      const text = await evalAsync(
        app,
        "fetch('assets/hello.txt').then(r => r.text()).then(t => globalThis.__pkgText = t)",
        "__pkgText",
      );
      bunExpect(text).toBe(readFileSync(join(REPO, "apps", "hello", "assets", "hello.txt"), "utf8"));
      const dotted = await evalAsync(
        app,
        "fetch('./assets/hello.txt?v=2').then(r => r.status).then(s => globalThis.__pkgDot = s)",
        "__pkgDot",
      );
      bunExpect(dotted).toBe(200);
      const miss = await evalAsync(
        app,
        "fetch('assets/not-packaged.txt').then(r => r.status).then(s => globalThis.__pkgMiss = s)",
        "__pkgMiss",
      );
      bunExpect(miss).toBe(404);
      // `launch()` starts the exe with the REPO as its cwd, where Cargo.toml exists: a miss must not
      // fall through to the working directory.
      bunExpect(existsSync(join(REPO, "Cargo.toml"))).toBe(true);
      const cwd = await evalAsync(
        app,
        "fetch('Cargo.toml').then(r => r.status).then(s => globalThis.__pkgCwd = s)",
        "__pkgCwd",
      );
      bunExpect(cwd).toBe(404);
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.kind === "advisory" && e.text.includes("not in this app's package"))).toBe(true);
    },
    T,
  );

  // macOS CI: the payload's <img> and <video> never paint.
  test.skipIf(!SCREENSHOTS || MACOS)(
    "<img> and <video> resolve relative srcs out of the payload",
    async () => {
      await app.locator(".tab").nth(3).click();
      await until("the 1080p clip's metadata", async () =>
        /1920×1080/.test(await app.locator(".meta").first().textContent()),
      );
      await app.eval("document.querySelector('.relative-img').scrollIntoView(); 'ok'");
      await sleep(600);
      const box = (await app.query(".relative-img")).nodes[0]!.bounds!;
      const shot = readPng(await app.screenshot());
      const scale = shot.width / Number(await app.eval("window.innerWidth"));
      const rect = { x: (box.x + 6) * scale, y: (box.y + 6) * scale, w: (box.w - 12) * scale, h: (box.h - 12) * scale };
      const saturated = shot.count(rect, (_l, [r, g, b]) => Math.max(r, g, b) - Math.min(r, g, b) > 80);
      bunExpect(saturated / (rect.w * rect.h)).toBeGreaterThan(0.25);
      const { entries } = await app.logs(0);
      bunExpect(entries.filter((e) => e.kind === "advisory" && e.text.includes('src="assets/'))).toEqual([]);
    },
    T,
  );

  test(
    "<lottie> plays a .tgs out of the payload",
    async () => {
      await app.locator(".tab").nth(2).click();
      await until(
        "a completed lottie loop",
        async () => /fired [1-9]\d*×/.test(await app.locator(".loops").textContent()),
        20_000,
      );
    },
    T,
  );

  test(
    "localStorage lives under %LOCALAPPDATA%\\<sk.config name>\\storage, not the exe's name",
    async () => {
      const info = JSON.parse(await app.eval("JSON.stringify(__SK_STORAGE__.info())"));
      bunExpect(info.folder.toLowerCase().endsWith(`${sep}hello${sep}storage`)).toBe(true);
      bunExpect(info.folder.toLowerCase()).not.toContain("sk-app-sktest");
    },
    T,
  );

  test(
    "the app logged nothing broken",
    async () => {
      await app.expectNoErrors({ ignore: [/not in this app's package/, /127\.0\.0\.1:8731/] });
    },
    T,
  );
});

describe("two packaged apps side by side", () => {
  test(
    "hello and shadcn run at once with separate storage",
    async () => {
      const hello = await launch({
        app: "hello",
        exe: runnable(built.hello!.exe, "hello-b"),
        packaged: "hello",
        label: "pkg-hello-b",
      });
      opened.push(hello);
      const shadcn = await launch({
        app: "shadcn",
        exe: runnable(built.shadcn!.exe, "shadcn"),
        packaged: "shadcn",
        label: "pkg-shadcn",
      });
      opened.push(shadcn);
      try {
        bunExpect(hello.slotDir).not.toBe(shadcn.slotDir);
        const hi = JSON.parse(await hello.eval("JSON.stringify(__SK_STORAGE__.info())"));
        const sh = JSON.parse(await shadcn.eval("JSON.stringify(__SK_STORAGE__.info())"));
        bunExpect(sh.folder.toLowerCase().endsWith(`${sep}shadcn${sep}storage`)).toBe(true);
        bunExpect(hi.folder).not.toBe(sh.folder);

        await hello.eval(`localStorage.setItem(${JSON.stringify(PROBE_KEY)}, 'from hello'); __SK_STORAGE__.flush()`);
        bunExpect(await hello.eval(`localStorage.getItem(${JSON.stringify(PROBE_KEY)})`)).toBe("from hello");
        bunExpect(await shadcn.eval(`String(localStorage.getItem(${JSON.stringify(PROBE_KEY)}))`)).toBe("null");
        const after = JSON.parse(await hello.eval("JSON.stringify(__SK_STORAGE__.info())"));
        if (!after.encrypted) {
          bunExpect(readFileSync(after.file, "utf8")).toContain(PROBE_KEY);
          const shFile = JSON.parse(await shadcn.eval("JSON.stringify(__SK_STORAGE__.info())")).file;
          bunExpect(existsSync(shFile) ? readFileSync(shFile, "utf8") : "").not.toContain(PROBE_KEY);
        }
      } finally {
        await hello
          .eval(`localStorage.removeItem(${JSON.stringify(PROBE_KEY)}); __SK_STORAGE__.flush()`)
          .catch(() => {});
        await hello.close();
        await shadcn.close();
      }
    },
    T,
  );
});

describe("re-stamping", () => {
  // Re-stamping rewrites a PE's resources.
  test.skipIf(!WINDOWS)(
    "a stamped exe takes a new payload and keeps its icon and version info",
    async () => {
      const fixture = join(REPO, "packages", "test", "fixtures", "devtools-on");
      const work = join(OUT, "restamp");
      const r = await buildApp({ target: fixture, out: work, payloadOnly: true, log: () => {} });
      const v = readPayload(new Uint8Array(readFileSync(r.skpk)));
      const inputs: SkpkInput[] = v.entries.map((e) => ({
        name: e.name,
        kind: e.kind,
        data: v.bytes(e),
        ...(e.alignLog2 === undefined ? {} : { alignLog2: e.alignLog2 }),
      }));
      const out = join(work, "Hello-restamped.exe");
      const s = stamp({ host: built.hello!.exe, out, payload: (skew) => writeSkpk(inputs, { skew }) });
      bunExpect(s.dataAligned).toBe(true);
      bunExpect(versionString(out, "ProductName")).toBe("Hello");
      bunExpect(shellIconCount(out)).toBeGreaterThanOrEqual(1);
      bunExpect(readPe(out).subsystem).toBe(SUBSYSTEM.GUI);

      const app = await launch({
        app: "devtools-on",
        exe: runnable(out, "restamped"),
        packaged: "devtools-on",
        label: "pkg-restamp",
      });
      opened.push(app);
      try {
        await until("the fixture's text", async () => /devtools: on/.test(await app.locator(".fixture").textContent()));
        bunExpect(await app.eval("typeof globalThis.__SK_TEST_DEVTOOLS__")).toBe("object");
      } finally {
        await app.close();
      }
    },
    T,
  );
});

describe("the no-devtools host", () => {
  test(
    "publishes no handshake, listens on no port, and does not contain the inspector",
    async () => {
      const nd = cliBuild("hello", noDevtoolsHost, "no-devtools", "hello-nodev");
      bunExpect(nd.json.variant).toBe("no-devtools");
      bunExpect(await fileContains(built.hello!.exe, MARKER)).toBe(true);
      bunExpect(await fileContains(nd.exe, MARKER)).toBe(false);

      const exe = runnable(nd.exe, "hello-nodev");
      const env: Record<string, string> = {
        ...(process.env as Record<string, string>),
        SK_OWNER_PID: String(process.pid),
      };
      for (const k of ["SK_DIST", "SK_APP_ROOT", "SK_PAYLOAD"]) delete env[k];
      const child = spawn(exe, [], { env, stdio: "ignore" });
      strays.push(child);
      await sleep(8000);
      bunExpect(child.exitCode).toBeNull();
      const slot = packagedSlot("hello", child.pid!);
      bunExpect(existsSync(join(slot, "devtools.json"))).toBe(false);
      const net = spawnSync("netstat", ["-ano", "-p", "TCP"], { encoding: "utf8" }).stdout ?? "";
      const listening = net
        .split(/\r?\n/)
        .filter((l) => /LISTENING/.test(l) && l.trim().split(/\s+/).pop() === String(child.pid));
      bunExpect(listening).toEqual([]);
      killTree(child);
    },
    T,
  );
});
