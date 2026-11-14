// The host is whatever `sk build` would find (`bareHost`).

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, launch, packagedExe, REPO, WORK_DIR } from "../src/index";
import { KIND, NAMES, readSkpk } from "../../cli/cli/skpk";

const T = 30_000;
const BUILD_T = 20 * 60_000;

const scheme = `sktest-pkg-${process.pid}`;
const id = "gl.argon.sk-test.launch-packaged";
const keyOf = (s: string) => `HKCU\\Software\\Classes\\${s}`;
const regExists = (key: string) => spawnSync("reg", ["query", key], { stdio: "ignore" }).status === 0;
function regValue(key: string, name: string | null): string | null {
  const r = spawnSync("reg", ["query", key, ...(name === null ? ["/ve"] : ["/v", name])], { encoding: "utf8" });
  if (r.status !== 0) return null;
  const line = r.stdout.split(/\r?\n/).find((l) => /\sREG_SZ\s/.test(l));
  return line ? line.replace(/^.*?\sREG_SZ\s+/, "") : "";
}

const appDir = join(WORK_DIR, "launch-packaged", `app-${process.pid}`);
rmSync(appDir, { recursive: true, force: true });
mkdirSync(appDir, { recursive: true });
cpSync(join(REPO, "packages", "test", "fixtures", "launch", "src"), join(appDir, "src"), { recursive: true });
writeFileSync(
  join(appDir, "sk.config.json"),
  JSON.stringify(
    {
      name: "sk-test-launch-packaged",
      productName: "sk-test launch (packaged)",
      version: "0.0.1",
      id,
      singleInstance: true,
      raiseOnSecondInstance: true,
      protocols: [scheme],
      assets: [],
      window: { title: "Skathi — launch, packaged (fixture)", width: 480, height: 320 },
    },
    null,
    2,
  ),
);

// Nothing of ours may exist before the exe starts: the registration below must be the exe's own
// doing.
spawnSync("reg", ["delete", keyOf(scheme), "/f"], { stdio: "ignore" });

const scratch = join(WORK_DIR, "launch-packaged", `cwd-${process.pid}`);
mkdirSync(scratch, { recursive: true });
const doc = join(scratch, "y.txt");
writeFileSync(doc, "a document the second launch names\n");

const p = await packagedExe({ app: appDir, label: "launch-packaged" });
const app = await launch({
  app: "launch-packaged",
  exe: p.exe,
  packaged: p.name,
  label: "launch-packaged",
  env: { SK_SINGLE_INSTANCE: "1" },
  launchTimeout: BUILD_T,
});

afterAll(() => {
  app.close();
  spawnSync(p.exe, ["--sk-unregister-protocols"], { cwd: REPO, stdio: "ignore", timeout: 15_000 });
  spawnSync("reg", ["delete", keyOf(scheme), "/f"], { stdio: "ignore" });
  rmSync(scratch, { recursive: true, force: true });
  rmSync(appDir, { recursive: true, force: true });
});

const secondEnv = () => ({
  ...(process.env as Record<string, string>),
  SK_SINGLE_INSTANCE: "1",
  SK_LAUNCH_RAISE: "off",
  SK_OWNER_PID: String(process.pid),
});

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

describe("the packaged exe", () => {
  test(
    "carries the launch keys in its payload and holds the lock",
    async () => {
      // The payload that was stamped, read back: META's `launch` is what the host enforces before
      // anything else exists.
      const v = readSkpk(new Uint8Array(readFileSync(p.result.skpk)));
      const meta = v.find(KIND.META, NAMES.META);
      bunExpect(meta).toBeDefined();
      const launchMeta = JSON.parse(new TextDecoder().decode(v.bytes(meta!))).launch;
      bunExpect(launchMeta).toMatchObject({
        id,
        singleInstance: true,
        raiseOnSecondInstance: true,
        protocols: [scheme],
      });
      const r = await rec();
      bunExpect(r.own.singleInstance).toBe(true);
      bunExpect(r.params).toEqual([{ targetURL: null, files: 0 }]);
    },
    T,
  );

  test(
    "registered its declared protocol at startup: per user, our marker, no dev slot",
    async () => {
      const key = keyOf(scheme);
      bunExpect(regValue(key, null)).toBe(`URL:${scheme}`);
      bunExpect(regValue(key, "URL Protocol")).toBe("");
      bunExpect(regValue(key, "VnOwner")).toBe(id);
      bunExpect(regValue(`${key}\\DefaultIcon`, null)?.toLowerCase()).toBe(`"${p.exe.toLowerCase()}",0`);
      const command = regValue(`${key}\\shell\\open\\command`, null) ?? "";
      bunExpect(command.toLowerCase()).toBe(`"${p.exe.toLowerCase()}" --sk-protocol-launch -- "%1"`);
      bunExpect(await app.eval(`__SK_TEST_LAUNCH__.api.isRegistered(${JSON.stringify(scheme)})`)).toBe(true);
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes(`registered ${scheme}://`))).toBe(true);
    },
    T,
  );
});

describe("a second launch of the packaged exe", () => {
  test(
    "forwards its argv, cwd and paths to the running one and exits 0",
    async () => {
      const r = spawnSync(p.exe, ["--open", doc], { cwd: scratch, env: secondEnv(), timeout: 15_000 });
      bunExpect(r.status).toBe(0);
      const got = await untilEvents(1);
      bunExpect(got.events.length).toBe(1);
      const ev = got.events[0]!;
      bunExpect(ev.source).toBe("second-instance");
      bunExpect(ev.argv.slice(1)).toEqual(["--open", doc]);
      bunExpect(ev.cwd.toLowerCase()).toBe(scratch.toLowerCase());
      bunExpect(ev.paths).toEqual([doc]);
      await expect(app.locator(".fixture")).toHaveText("forwarded launches: 1");
    },
    T,
  );

  test(
    "a link to its scheme, opened through the SHELL, reaches the running instance",
    async () => {
      const before = (await rec()).events.length;
      // (Quotes and spaces byte-exact are launch.spec's, through a hand-built command line: what the
      // shell itself does to them is Windows' business.)
      const url = `${scheme}://open/item?id=7&q=x`;
      await app.eval(`__SK_TEST_LAUNCH__.api.open(${JSON.stringify(url)})`);
      const got = await untilEvents(before + 1, 15_000);
      bunExpect(got.opened.at(-1)).toBe("ok");
      bunExpect(got.events.length).toBe(before + 1);
      bunExpect(got.events.at(-1)!.url).toBe(url);
      bunExpect(got.params.at(-1)).toEqual({ targetURL: url, files: 0 });
    },
    T,
  );
});

describe("uninstalling", () => {
  test(
    "--sk-unregister-protocols removes the exe's keys and exits without a window",
    async () => {
      const r = spawnSync(p.exe, ["--sk-unregister-protocols"], {
        cwd: REPO,
        env: secondEnv(),
        encoding: "utf8",
        timeout: 15_000,
      });
      bunExpect(r.status).toBe(0);
      bunExpect(r.stdout).toContain(`unregistered ${scheme}:`);
      bunExpect(regExists(keyOf(scheme))).toBe(false);
      bunExpect(await app.eval(`__SK_TEST_LAUNCH__.api.isRegistered(${JSON.stringify(scheme)})`)).toBe(false);
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

  test("the hive is clean", () => {
    bunExpect(regExists(keyOf(scheme))).toBe(false);
  });
});
