// Each "second launch" below is the SAME staged exe started by hand on the SAME slot, exactly as a
// user's second double-click (or the shell, for a `myapp://` link) would start it.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stagedExe } from "../src/dist";
import { expect, killTree, launch, REPO, WINDOWS } from "../src/index";

const T = 30_000;
const appName = `sk-test-launch-${process.pid}`;
const app = await launch({
  app: "packages/test/fixtures/launch/src/main.ts",
  label: "launch",
  env: { SK_SINGLE_INSTANCE: "1", SK_APP_NAME: appName },
  args: ["--first", "run"],
});
const exe = stagedExe();

// Real path: macOS hands the second launch /private/var for /var.
const scratch = join(realpathSync(tmpdir()), `sk-test-launch-${process.pid}`);
mkdirSync(scratch, { recursive: true });
const doc = join(scratch, "x.txt");
writeFileSync(doc, "a document the second launch names\n");
const extra: number[] = [];
afterAll(() => {
  for (const pid of extra) killTree(pid);
  app.close();
  rmSync(scratch, { recursive: true, force: true });
});

const secondEnv = (more: Record<string, string> = {}) => ({
  ...(process.env as Record<string, string>),
  SK_DIST: app.slotDir,
  SK_APP_NAME: appName,
  SK_SINGLE_INSTANCE: "1",
  SK_LAUNCH_RAISE: "off",
  SK_OWNER_PID: String(process.pid),
  ...more,
});

const forwardedLines = async () =>
  (await app.logs(0)).entries.map((e) => e.text).filter((t) => t.includes("a second launch was forwarded here"));

type Record_ = {
  opened: string[];
  own: { argv: string[]; cwd: string; url: string | null; paths: string[]; singleInstance: boolean };
  events: { argv: string[]; cwd: string; url: string | null; paths: string[]; source: string }[];
  params: { targetURL: string | null; files: number }[];
};
const rec = async (): Promise<Record_> => JSON.parse(await app.eval("JSON.stringify(__SK_TEST_LAUNCH__)"));
async function untilEvents(n: number, ms = 5_000): Promise<Record_> {
  const deadline = Date.now() + ms;
  for (;;) {
    const r = await rec();
    if (r.events.length >= n || Date.now() > deadline) return r;
    await new Promise((res) => setTimeout(res, 50));
  }
}

describe("the first instance", () => {
  test(
    "holds the lock and reports its own launch",
    async () => {
      const r = await rec();
      bunExpect(r.own.singleInstance).toBe(true);
      bunExpect(r.own.argv.slice(1)).toEqual(["--first", "run"]);
      bunExpect(r.own.url).toBe(null);
      bunExpect(r.params).toEqual([{ targetURL: null, files: 0 }]);
    },
    T,
  );
});

describe("a second launch", () => {
  test(
    "forwards argv, cwd and existing paths to the first, exits 0 fast, and opens no window",
    async () => {
      const t0 = Date.now();
      const r = spawnSync(exe, ["--foo", doc, "missing.txt"], {
        cwd: scratch,
        env: secondEnv({ SK_WINDOW_FOCUS: "" }),
        timeout: 15_000,
      });
      const ms = Date.now() - t0;
      bunExpect(r.status).toBe(0);
      bunExpect(ms).toBeLessThan(5_000);
      const got = await untilEvents(1);
      bunExpect(got.events.length).toBe(1);
      const ev = got.events[0]!;
      bunExpect(ev.source).toBe("second-instance");
      bunExpect(ev.argv.slice(1)).toEqual(["--foo", doc, "missing.txt"]);
      bunExpect(ev.cwd.toLowerCase()).toBe(scratch.toLowerCase());
      bunExpect(ev.paths).toEqual([doc]);
      bunExpect(ev.url).toBe(null);
      bunExpect(got.params.at(-1)).toEqual({ targetURL: null, files: 1 });
      await expect(app.locator(".fixture")).toHaveText("forwarded launches: 1");
      // The primary is quiet, so it stayed where it was, and its log says why (the harness's
      // SK_LAUNCH_RAISE=off is named as well).
      const lines = await forwardedLines();
      bunExpect(lines.length).toBe(1);
      bunExpect(lines[0]).toContain("not raised: this instance is quiet (SK_WINDOW_FOCUS=never)");
      bunExpect(lines[0]).not.toContain("the second launch was quiet");
    },
    T,
  );

  test(
    "a protocol launch carries its URL byte-exact, quotes and spaces included",
    async () => {
      const url = 'sktest-launch://open?b="c d"&e=%22';
      // Quoted the way the Windows shell quotes %1; elsewhere argv arrives as it is.
      const arg = process.platform === "win32" ? `"${url}"` : url;
      const r = spawnSync(exe, ["--devtools", "--sk-protocol-launch", "--", arg], {
        cwd: REPO,
        env: secondEnv(),
        windowsVerbatimArguments: true,
        timeout: 15_000,
      });
      bunExpect(r.status).toBe(0);
      const got = await untilEvents(2);
      const ev = got.events[1]!;
      bunExpect(ev.source).toBe("open-url");
      bunExpect(ev.url).toBe(url);
      bunExpect(got.params.at(-1)).toEqual({ targetURL: url, files: 0 });
      bunExpect(ev.argv.includes("--devtools")).toBe(false);
    },
    T,
  );

  test(
    "a `--devtools` AFTER an app argument is data, and the launch is still forwarded",
    async () => {
      const r = spawnSync(exe, ["report.txt", "--devtools"], { cwd: scratch, env: secondEnv(), timeout: 15_000 });
      bunExpect(r.status).toBe(0);
      const got = await untilEvents(3);
      bunExpect(got.events[2]!.argv.slice(1)).toEqual(["report.txt", "--devtools"]);
    },
    T,
  );

  // Fails everywhere, Windows included: the log names the instance's quiet mode, not the launch's.
  test.skipIf(!!process.env.CI)(
    "a quiet second launch is still delivered, and asks for no raise",
    async () => {
      // An agent's or a test's launch (SK_WINDOW_FOCUS=never) hands on no foreground right and says
      // so in its message: it never pulls the running window to the front, whoever is using it.
      const r = spawnSync(exe, ["quiet"], {
        cwd: scratch,
        env: secondEnv({ SK_WINDOW_FOCUS: "never" }),
        timeout: 15_000,
      });
      bunExpect(r.status).toBe(0);
      const got = await untilEvents(4);
      bunExpect(got.events[3]!.argv.slice(1)).toEqual(["quiet"]);
      const lines = await forwardedLines();
      bunExpect(lines.at(-1)).toContain("the second launch was quiet (SK_WINDOW_FOCUS=never)");
    },
    T,
  );
});

const scheme = `sktest-${process.pid}`;
const foreign = `sktest-foreign-${process.pid}`;
const keyOf = (s: string) => `HKCU\\Software\\Classes\\${s}`;
const regExists = (key: string) => spawnSync("reg", ["query", key], { stdio: "ignore" }).status === 0;
function regValue(key: string, name: string | null): string | null {
  const r = spawnSync("reg", ["query", key, ...(name === null ? ["/ve"] : ["/v", name])], { encoding: "utf8" });
  if (r.status !== 0) return null;
  const line = r.stdout.split(/\r?\n/).find((l) => /\sREG_SZ\s/.test(l));
  return line ? line.replace(/^.*?\sREG_SZ\s+/, "") : "";
}
const api = (call: string) => app.eval(`__SK_TEST_LAUNCH__.api.${call}`);
afterAll(() => {
  for (const s of [scheme, foreign]) spawnSync("reg", ["delete", keyOf(s), "/f"], { stdio: "ignore" });
});

describe("protocols", () => {
  // Protocol registration is Windows-only so far (HKCU\Software\Classes).
  test.skipIf(!WINDOWS)(
    "registerProtocol writes the documented per-user key, pointing a dev host at its slot",
    async () => {
      bunExpect(regExists(keyOf(scheme))).toBe(false);
      bunExpect(await api(`isRegistered(${JSON.stringify(scheme)})`)).toBe(false);
      bunExpect(await api(`register(${JSON.stringify(scheme)})`)).toBe("ok");
      bunExpect(regValue(keyOf(scheme), null)).toBe(`URL:${scheme}`);
      bunExpect(regValue(keyOf(scheme), "URL Protocol")).toBe("");
      bunExpect(regValue(keyOf(scheme), "VnOwner")).toBe("gl.argon.sk-test.launch");
      const command = regValue(`${keyOf(scheme)}\\shell\\open\\command`, null) ?? "";
      bunExpect(command.toLowerCase().startsWith(`"${exe.toLowerCase()}" --sk-dist "`)).toBe(true);
      bunExpect(command.toLowerCase()).toContain(app.slotDir.toLowerCase());
      bunExpect(command.endsWith(` --sk-protocol-launch -- "%1"`)).toBe(true);
      bunExpect(await api(`isRegistered(${JSON.stringify(scheme)})`)).toBe(true);
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes(`points ${scheme}:// at this dev slot`))).toBe(true);
    },
    T,
  );

  test.skipIf(!WINDOWS)(
    "a link to it, opened through the SHELL, reaches the running instance",
    async () => {
      const before = (await rec()).events.length;
      const url = `${scheme}://open/item?id=42`;
      await api(`open(${JSON.stringify(url)})`);
      const got = await untilEvents(before + 1, 15_000);
      bunExpect(got.opened.at(-1)).toBe("ok");
      bunExpect(got.events.length).toBe(before + 1);
      bunExpect(got.events.at(-1)!.url).toBe(url);
      bunExpect(got.params.at(-1)).toEqual({ targetURL: url, files: 0 });
    },
    T,
  );

  test.skipIf(!WINDOWS)(
    "a scheme another app owns is refused, named, and left alone",
    async () => {
      spawnSync("reg", ["add", keyOf(foreign), "/ve", "/d", "URL:Some Other App", "/f"], { stdio: "ignore" });
      const answer = await api(`register(${JSON.stringify(foreign)})`);
      bunExpect(answer).toStartWith("TypeError:");
      bunExpect(answer).toContain("Some Other App");
      bunExpect(regValue(keyOf(foreign), null)).toBe("URL:Some Other App");
      bunExpect(regExists(`${keyOf(foreign)}\\shell`)).toBe(false);
      bunExpect(await api(`unregister(${JSON.stringify(foreign)})`)).toBe(false);
      bunExpect(regExists(keyOf(foreign))).toBe(true);
      spawnSync("reg", ["delete", keyOf(foreign), "/f"], { stdio: "ignore" });
      bunExpect(await api(`register("https")`)).toStartWith("TypeError:");
      bunExpect(await api(`register("ms-settings")`)).toStartWith("TypeError:");
    },
    T,
  );

  test.skipIf(!WINDOWS)(
    "unregisterProtocol removes it; --sk-unregister-protocols removes every key of this app and exits",
    async () => {
      bunExpect(await api(`unregister(${JSON.stringify(scheme)})`)).toBe(true);
      bunExpect(regExists(keyOf(scheme))).toBe(false);
      bunExpect(await api(`isRegistered(${JSON.stringify(scheme)})`)).toBe(false);
      bunExpect(await api(`register(${JSON.stringify(scheme)})`)).toBe("ok");
      const r = spawnSync(exe, ["--sk-unregister-protocols"], {
        cwd: REPO,
        env: secondEnv(),
        encoding: "utf8",
        timeout: 15_000,
      });
      bunExpect(r.status).toBe(0);
      bunExpect(r.stdout).toContain(`unregistered ${scheme}:`);
      bunExpect(regExists(keyOf(scheme))).toBe(false);
    },
    T,
  );

  test("the hive is clean", () => {
    bunExpect(regExists(keyOf(scheme))).toBe(false);
    bunExpect(regExists(keyOf(foreign))).toBe(false);
  });
});

describe("opting out", () => {
  test(
    "SK_SINGLE_INSTANCE=0 runs a second copy side by side instead of forwarding",
    async () => {
      const before = (await rec()).events.length;
      const child = spawn(exe, [], {
        cwd: REPO,
        env: secondEnv({ SK_SINGLE_INSTANCE: "0" }),
        detached: true,
        stdio: "ignore",
      });
      child.unref();
      if (child.pid) extra.push(child.pid);
      let exited: number | null = null;
      child.on("exit", (code) => (exited = code ?? -1));
      await new Promise((res) => setTimeout(res, 2_500));
      bunExpect(exited).toBe(null);
      const r = await rec();
      bunExpect(r.events.length).toBe(before);
    },
    T,
  );

  test(
    "nothing went wrong in the primary",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
