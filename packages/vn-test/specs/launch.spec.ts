// Single instance, forwarded launches and protocol-mode argv, through real
// processes (docs/LAUNCH.md).
//
//   bun test packages/vn-test/specs/launch.spec.ts
//
// The primary is the `launch` fixture (`"singleInstance": true`) on its own
// slot, with VN_SINGLE_INSTANCE=1 overriding the harness default. Each
// "second launch" below is the SAME staged exe started by hand on the SAME
// slot, exactly as a user's second double-click (or the shell, for a
// `myapp://` link) would start it. VN_LAUNCH_RAISE=off (the harness default)
// keeps the primary from taking the foreground, so the run never steals focus
// from whoever is at the machine.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stagedExe } from "../src/dist";
import { expect, launch, REPO } from "../src/index";

const T = 30_000;
const app = await launch({
  app: "packages/vn-test/fixtures/launch/src/main.ts",
  label: "launch",
  env: { VN_SINGLE_INSTANCE: "1" },
  args: ["--first", "run"],
});
const exe = stagedExe();

const scratch = join(tmpdir(), `vn-test-launch-${process.pid}`);
mkdirSync(scratch, { recursive: true });
const doc = join(scratch, "x.txt");
writeFileSync(doc, "a document the second launch names\n");
const extra: number[] = [];
afterAll(() => {
  for (const pid of extra) spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
  app.close();
  rmSync(scratch, { recursive: true, force: true });
});

/** The env a second launch of this slot runs with. */
const secondEnv = (more: Record<string, string> = {}) => ({
  ...(process.env as Record<string, string>),
  VN_DIST: app.slotDir,
  VN_SINGLE_INSTANCE: "1",
  VN_LAUNCH_RAISE: "off",
  VN_OWNER_PID: String(process.pid),
  ...more,
});

type Record_ = {
  own: { argv: string[]; cwd: string; url: string | null; paths: string[]; singleInstance: boolean };
  events: { argv: string[]; cwd: string; url: string | null; paths: string[]; source: string }[];
  params: { targetURL: string | null; files: number }[];
};
const rec = async (): Promise<Record_> => JSON.parse(await app.eval("JSON.stringify(__VN_TEST_LAUNCH__)"));
async function untilEvents(n: number, ms = 5_000): Promise<Record_> {
  const deadline = Date.now() + ms;
  for (;;) {
    const r = await rec();
    if (r.events.length >= n || Date.now() > deadline) return r;
    await new Promise((res) => setTimeout(res, 50));
  }
}

describe("the first instance", () => {
  test("holds the lock and reports its own launch", async () => {
    const r = await rec();
    bunExpect(r.own.singleInstance).toBe(true);
    bunExpect(r.own.argv.slice(1)).toEqual(["--first", "run"]);
    bunExpect(r.own.url).toBe(null);
    // Its own launch is one LaunchParams, delivered once a consumer was set.
    bunExpect(r.params).toEqual([{ targetURL: null, files: 0 }]);
  }, T);
});

describe("a second launch", () => {
  test("forwards argv, cwd and existing paths to the first, exits 0 fast, and opens no window", async () => {
    const t0 = Date.now();
    const r = spawnSync(exe, ["--foo", doc, "missing.txt"], { cwd: scratch, env: secondEnv(), timeout: 15_000 });
    const ms = Date.now() - t0;
    bunExpect(r.status).toBe(0);
    // A debug host on a busy machine: generous, but far below a window's boot.
    bunExpect(ms).toBeLessThan(5_000);
    const got = await untilEvents(1);
    bunExpect(got.events.length).toBe(1);
    const ev = got.events[0];
    bunExpect(ev.source).toBe("second-instance");
    bunExpect(ev.argv.slice(1)).toEqual(["--foo", doc, "missing.txt"]);
    bunExpect(ev.cwd.toLowerCase()).toBe(scratch.toLowerCase());
    bunExpect(ev.paths).toEqual([doc]);
    bunExpect(ev.url).toBe(null);
    // …and one more LaunchParams, after the event.
    bunExpect(got.params.at(-1)).toEqual({ targetURL: null, files: 0 });
    await expect(app.locator(".fixture")).toHaveText("forwarded launches: 1");
  }, T);

  test("a protocol launch carries its URL byte-exact, quotes and spaces included", async () => {
    // What the shell builds from `"<exe>" --vn-protocol-launch -- "%1"` for
    // the URL  vntest-launch://open?b="c d"&e=%22  — %1 substituted verbatim.
    const url = 'vntest-launch://open?b="c d"&e=%22';
    const r = spawnSync(exe, ["--devtools", "--vn-protocol-launch", "--", `"${url}"`], {
      cwd: REPO,
      env: secondEnv(),
      windowsVerbatimArguments: true,
      timeout: 15_000,
    });
    bunExpect(r.status).toBe(0);
    const got = await untilEvents(2);
    const ev = got.events[1];
    bunExpect(ev.url).toBe(url);
    bunExpect(got.params.at(-1)).toEqual({ targetURL: url, files: 0 });
    // `--devtools` in front of the protocol marker is NOT a mode switch: the
    // process forwarded instead of becoming an inspector.
    bunExpect(ev.argv.includes("--devtools")).toBe(false);
  }, T);

  test("a `--devtools` AFTER an app argument is data, and the launch is still forwarded", async () => {
    const r = spawnSync(exe, ["report.txt", "--devtools"], { cwd: scratch, env: secondEnv(), timeout: 15_000 });
    bunExpect(r.status).toBe(0);
    const got = await untilEvents(3);
    bunExpect(got.events[2].argv.slice(1)).toEqual(["report.txt", "--devtools"]);
  }, T);

  test("VN_SINGLE_INSTANCE=0 runs a second copy side by side instead of forwarding", async () => {
    const child = spawn(exe, [], {
      cwd: REPO,
      env: secondEnv({ VN_SINGLE_INSTANCE: "0" }),
      detached: true,
      stdio: "ignore",
    });
    child.unref();
    if (child.pid) extra.push(child.pid);
    let exited: number | null = null;
    child.on("exit", (code) => (exited = code ?? -1));
    // A forwarding process is gone in well under a second; a real second copy
    // is still booting its window.
    await new Promise((res) => setTimeout(res, 2_500));
    bunExpect(exited).toBe(null);
    const r = await rec();
    bunExpect(r.events.length).toBe(3);
  }, T);

  test("nothing went wrong in the primary", async () => {
    await app.expectNoErrors();
  }, T);
});
