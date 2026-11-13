import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, launch } from "../src/index";

const T = 30_000;
const app = await launch({
  app: "packages/vn-test/fixtures/shell/src/main.ts",
  label: "shell",
  env: { VN_SHELL: "record" },
});

const scratch = join(tmpdir(), `vn-test-shell-${process.pid}`);
mkdirSync(scratch, { recursive: true });
const doc = join(scratch, "report.txt");
writeFileSync(doc, "vn-test shell fixture\n");
afterAll(() => {
  app.close();
  rmSync(scratch, { recursive: true, force: true });
});

async function recorded(): Promise<string[]> {
  const { entries } = await app.logs(0);
  return entries.filter((e) => e.kind === "shell").map((e) => e.text);
}

async function until<T>(read: () => Promise<T>, pred: (v: T) => boolean, ms = 5_000): Promise<T> {
  const deadline = Date.now() + ms;
  for (;;) {
    const v = await read();
    if (pred(v) || Date.now() > deadline) return v;
    await new Promise((r) => setTimeout(r, 50));
  }
}

const results = () => app.eval("JSON.stringify(__VN_TEST_SHELL__.results)").then((s) => JSON.parse(s) as string[]);
const run = (verb: string, ...args: unknown[]) =>
  app.eval(`__VN_TEST_SHELL__.run(${JSON.stringify(verb)}, ...${JSON.stringify(args)})`);

describe("links (tier 1)", () => {
  test(
    "a REAL click on a link's listening child is one navigation, and each handler runs once",
    async () => {
      await app.eval(
        `globalThis.__aClicks = 0; document.querySelector('.ext').addEventListener('click', () => globalThis.__aClicks++); 1`,
      );
      await app.locator(".ext-label").click({ real: true });
      const lines = await until(recorded, (l) => l.length >= 1);
      // Give a second (wrong) delivery the frame it would need to show up.
      await new Promise((r) => setTimeout(r, 300));
      const all = await recorded();
      bunExpect(all.filter((l) => l.includes("openExternal"))).toEqual([
        "[shell] openExternal https://example.invalid/vn-test?q=a%20b (recorded — VN_SHELL=record)",
      ]);
      bunExpect(lines.length).toBeGreaterThan(0);
      await expect(app.locator(".count")).toHaveText("span clicks: 1");
      bunExpect(await app.eval("globalThis.__aClicks")).toBe(1);
    },
    T,
  );

  test(
    "a second real press is a second click (the claim is per press, not per session)",
    async () => {
      await app.locator(".ext-label").click({ real: true });
      await expect(app.locator(".count")).toHaveText("span clicks: 2");
      bunExpect(await app.eval("globalThis.__aClicks")).toBe(2);
      const opens = (await recorded()).filter((l) => l.includes("example.invalid/vn-test"));
      bunExpect(opens.length).toBe(2);
    },
    T,
  );

  test(
    "mailto: goes to the shell too",
    async () => {
      await app.locator(".mail").click();
      const lines = await until(recorded, (l) => l.some((x) => x.includes("mailto:")));
      bunExpect(lines.some((l) => l.startsWith("[shell] openExternal mailto:nobody@example.invalid"))).toBe(true);
    },
    T,
  );

  test(
    "a custom scheme and file: are refused with an advisory, and never reach the shell",
    async () => {
      const before = (await recorded()).length;
      await app.locator(".custom").click();
      await app.locator(".file").click();
      await new Promise((r) => setTimeout(r, 300));
      bunExpect((await recorded()).length).toBe(before);
      const { entries } = await app.logs(0);
      const text = entries.map((e) => e.text).join("\n");
      bunExpect(text).toContain("with a vntest-refused: URL was refused");
      bunExpect(text).toContain("with a file: URL was refused");
    },
    T,
  );

  test(
    "preventDefault on the link cancels the navigation",
    async () => {
      const before = (await recorded()).length;
      await app.locator(".routed").click();
      await new Promise((r) => setTimeout(r, 300));
      bunExpect((await recorded()).length).toBe(before);
    },
    T,
  );

  test(
    "window.open(https) opens the system browser and returns null",
    async () => {
      bunExpect(await app.eval(`window.open("https://example.invalid/w", "_blank") === null`)).toBe(true);
      const lines = await until(recorded, (l) => l.some((x) => x.includes("/w ")));
      bunExpect(lines.some((l) => l.startsWith("[shell] openExternal https://example.invalid/w "))).toBe(true);
    },
    T,
  );
});

describe("appShell (tier 2)", () => {
  test(
    "every verb settles, with the right name when it fails",
    async () => {
      run("openPath", doc);
      run("openPath", join(scratch, "missing.txt"));
      run("openPath", join(scratch, "setup.exe"));
      run("openPath", "relative.txt");
      run("showItemInFolder", doc);
      run("trashItem", doc);
      run("openExternal", "vntest-allowed://from-app-code");
      run("openExternal", "file:///C:/x.txt");
      const out = await until(results, (r) => r.length >= 8);
      bunExpect(out.sort()).toEqual(
        [
          "openPath ok",
          "openPath NotFoundError ENOENT",
          "openPath NotAllowedError EEXECUTABLE",
          "openPath TypeError",
          "showItemInFolder ok",
          "trashItem ok",
          "openExternal ok",
          "openExternal TypeError",
        ].sort(),
      );
      const lines = await recorded();
      bunExpect(lines).toContain(`[shell] openPath ${doc} (recorded — VN_SHELL=record)`);
      bunExpect(lines).toContain(`[shell] showItemInFolder ${doc} (recorded — VN_SHELL=record)`);
      bunExpect(lines).toContain(`[shell] trashItem ${doc} (recorded — VN_SHELL=record)`);
      bunExpect(lines).toContain("[shell] openExternal vntest-allowed://from-app-code (recorded — VN_SHELL=record)");
    },
    T,
  );

  test(
    "nothing went wrong in the app",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
