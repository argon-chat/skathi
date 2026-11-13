// `--features prod` — self-contained, no dist/, the thing you would actually ship — answers on the
// devtools port and opens its inspector on F12.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { connect } from "node:net";
import { join } from "node:path";
import { allocateSlot, ensureBuild, resolveEntry, stagedExe } from "../src/dist";
import { featureExe, fileContains, launch, packagedExe } from "../src/index";

const T = 60_000;
const BUILD_T = 15 * 60_000;

const ON = "packages/vn-test/fixtures/devtools-on/src/main.ts";
const OFF = "packages/vn-test/fixtures/devtools-off/src/main.ts";

/**
 * Present in any build that has the devtools and absent from one that does not — a marker that
 * cannot be there by accident, because nothing else in the process spells it.
 */
const MARKER = "vue-native devtools";

const strays: ChildProcess[] = [];

afterAll(() => {
  for (const child of strays) {
    if (!child.pid) continue;
    try {
      spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    } catch {}
  }
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(what: string, probe: () => Promise<T>, ms = 20_000): Promise<NonNullable<T>> {
  const deadline = Date.now() + ms;
  let last: unknown;
  for (;;) {
    const v = await probe().catch((e) => {
      last = e;
      return undefined as T;
    });
    if (v) return v;
    if (v !== undefined) last = v;
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
    }
    await sleep(150);
  }
}

function handshake(slot: string): { port: number; pid: number; token: string } {
  return JSON.parse(readFileSync(join(slot, "devtools.json"), "utf8"));
}

/** A refused connect on loopback cannot lie. */
function portOpen(port: number, ms = 500): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = connect({ host: "127.0.0.1", port });
    let settled = false;
    const done = (answer: boolean) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolve(answer);
    };
    const timer = setTimeout(() => done(false), ms);
    socket.on("connect", () => {
      clearTimeout(timer);
      done(true);
    });
    socket.on("error", () => {
      clearTimeout(timer);
      done(false);
    });
  });
}

function command(port: number, token: string, cmd: string, args: Record<string, unknown> = {}) {
  return new Promise<any>((resolveReply, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error(`${cmd}: no reply in 10s`));
    }, 10_000);
    socket.on("connect", () => socket.write(`${JSON.stringify({ token, cmd, ...args })}\n`));
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      const nl = buffer.indexOf("\n");
      if (nl < 0) return;
      clearTimeout(timer);
      socket.destroy();
      const reply = JSON.parse(buffer.slice(0, nl));
      reply.ok ? resolveReply(reply.data) : reject(new Error(reply.error));
    });
    socket.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });
}

/**
 * A host started BY HAND on its own slot, for the two cases `launch()` cannot reach: it waits for
 * a handshake, and the whole point of these hosts is that they never write one.
 */
async function spawnHost(
  app: string,
  label: string,
  env: Record<string, string> = {},
  exe: string = stagedExe(),
): Promise<{ slot: string; child: ChildProcess }> {
  const cache = await ensureBuild(resolveEntry(app));
  const slot = allocateSlot(cache, label);
  const child = spawn(exe, [], {
    env: { ...process.env, VN_DIST: slot, VN_WINDOW: "420x220", VN_DEVTOOLS_RAISE: "off", ...env },
    stdio: "ignore",
    detached: false,
  });
  strays.push(child);
  return { slot, child };
}

describe("a shipped build carries its own devtools", () => {
  test(
    "`--features prod` answers on the wire and opens the inspector on F12",
    async () => {
      const host = await featureExe({ features: ["prod"] });
      const packed = await packagedExe({ app: "hello", host, label: "dt-prod" });
      const app = await launch({
        app: "hello",
        exe: packed.exe,
        packaged: packed.name,
        window: [820, 600],
        label: "dt-prod",
        env: { VN_DEVTOOLS_RAISE: "off" },
      });
      try {
        bunExpect((await app.send("ping")).pid).toBe(app.pid);
        bunExpect(await app.eval("typeof globalThis.__VN__")).toBe("undefined");

        bunExpect((await app.send("windows")).inspectors).toEqual([]);
        await app.press("F12");
        const after = await until("the inspector the shipped binary opened on itself", async () => {
          const w = await app.send("windows");
          return w.inspectors.length === 1 ? w : null;
        });
        bunExpect(after.inspectors[0].pid).toBeGreaterThan(0);

        const state = JSON.parse(await app.eval("__vn_devtools_state()"));
        bunExpect(state.enabled).toBe(true);
        bunExpect(state.locked).toBe(false);
        bunExpect(state.port).toBe(app.port);
        bunExpect(state.inspectors).toBe(1);
      } finally {
        await app.close();
      }
    },
    BUILD_T,
  );

  test(
    "`--features no-devtools` compiles, and the image has none of it",
    async () => {
      // Two proofs on purpose: the marker says the inspector's bundle and window config are not in
      // the file, and running it says no listener was started.
      const without = await featureExe({ features: ["prod", "no-devtools"] });
      const with_ = await featureExe({ features: ["prod"] });

      bunExpect(await fileContains(with_, MARKER)).toBe(true);
      bunExpect(await fileContains(without, MARKER)).toBe(false);

      // A bare host exits at once ("no app inside"), so the claim is made on a packaged one — the
      // thing that would actually ship.
      const packed = await packagedExe({ app: "hello", host: without, variant: "no-devtools", label: "dt-nodev" });
      const { slot, child } = await spawnHost("hello", "dt-nodev", {}, packed.exe);
      await sleep(8000);
      bunExpect(child.exitCode).toBeNull();
      bunExpect(existsSync(join(slot, "devtools.json"))).toBe(false);
    },
    BUILD_T,
  );
});

describe("the app owns the switch", () => {
  test(
    "disable() closes the listener; enable() binds a fresh one",
    async () => {
      const app = await launch({
        app: ON,
        window: [420, 220],
        label: "dt-switch",
        env: { VN_DEVTOOLS_RAISE: "off" },
      });
      const firstPort = app.port;
      try {
        bunExpect(await portOpen(firstPort)).toBe(true);
        bunExpect(await app.eval("__VN_TEST_DEVTOOLS__.isEnabled()")).toBe(true);

        // The re-enable is scheduled BEFORE the disable, because the disable takes away the only way
        // to ask for anything — this reply is the last thing that comes off this wire.
        await app.eval(`
        globalThis.__SWITCH__ = {};
        setTimeout(() => {
          __SWITCH__.openedWhileOff = __VN_TEST_DEVTOOLS__.open();
          __SWITCH__.enabledWhileOff = __VN_TEST_DEVTOOLS__.isEnabled();
          __SWITCH__.reenabled = __VN_TEST_DEVTOOLS__.enable();
        }, 2500);
        __VN_TEST_DEVTOOLS__.disable();
      `);

        await until("the listener to close", async () => (await portOpen(firstPort)) === false);
        bunExpect(existsSync(join(app.slotDir, "devtools.json"))).toBe(false);

        // …and then the app turns them back on, on a NEW port with a NEW token, so a client that was
        // shown the door cannot walk back in on the old one.
        const shake = await until("a fresh handshake", async () =>
          existsSync(join(app.slotDir, "devtools.json")) ? handshake(app.slotDir) : null,
        );
        bunExpect(shake.pid).toBe(app.pid);
        bunExpect(shake.port).not.toBe(firstPort);

        const recorded = await command(shake.port, shake.token, "eval", {
          js: "JSON.stringify(globalThis.__SWITCH__)",
        });
        const seen = JSON.parse(recorded.value);
        bunExpect(seen.enabledWhileOff).toBe(false);
        bunExpect(seen.openedWhileOff).toBe(false);
        bunExpect(seen.reenabled).toBe(true);

        const windows = await command(shake.port, shake.token, "windows");
        bunExpect(windows.inspectors).toEqual([]);
        const logs = await command(shake.port, shake.token, "logs", { since: 0 });
        const text = logs.entries.map((e: { text: string }) => e.text).join("\n");
        bunExpect(text).toMatch(/\[devtools\] disabled — the listener is closed/);
        bunExpect(text).toMatch(/open\(\) did not open an inspector: the devtools are disabled/);
      } finally {
        await app.close();
      }
    },
    T,
  );

  test(
    '`"devtools": false` in vn.config.json means no port at all',
    async () => {
      // `launch()` cannot be used: it waits for a handshake this host is never going to write.
      const { slot, child } = await spawnHost(OFF, "dt-cfgoff");
      await sleep(8000);
      bunExpect(child.exitCode).toBeNull();
      bunExpect(existsSync(join(slot, "devtools.json"))).toBe(false);
    },
    T,
  );

  test(
    "VN_DEVTOOLS=1 overrides the app, and refuses disable() out loud",
    async () => {
      const app = await launch({
        app: OFF,
        window: [420, 220],
        label: "dt-forced",
        env: { VN_DEVTOOLS: "1", VN_DEVTOOLS_RAISE: "off" },
      });
      try {
        bunExpect((await app.send("ping")).pid).toBe(app.pid);
        const info = JSON.parse(await app.eval("JSON.stringify(__VN_TEST_DEVTOOLS__.info())"));
        bunExpect(info.enabled).toBe(true);
        bunExpect(info.locked).toBe(true);

        bunExpect(await app.eval("__VN_TEST_DEVTOOLS__.disable()")).toBe(true);
        bunExpect(await portOpen(app.port)).toBe(true);
        const { entries } = await app.logs(0);
        bunExpect(entries.some((e) => /disable\(\) was refused/.test(e.text) && /VN_DEVTOOLS=1/.test(e.text))).toBe(
          true,
        );
      } finally {
        await app.close();
      }
    },
    T,
  );
});
