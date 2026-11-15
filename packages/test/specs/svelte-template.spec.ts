// `sk create --template svelte`, run as `sk dev` runs it: the dev build, the app's own asset fetch, clicks, and an edit
// on disk that the watcher rebuilds and the host hot-updates.

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { expect, launch, REPO, WORK_DIR } from "../src/index";

const T = 60_000;
const dir = join(WORK_DIR, `svelte-template-${process.pid}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(WORK_DIR, { recursive: true });
create(dir, { name: "svelte-template", template: "svelte", platforms: ["windows"] });
const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");

const app = await launch({ app: entry, window: [760, 520], label: "svelte-template" });
let watcher: { kill(): void; exited: Promise<number> } | null = null;
afterAll(async () => {
  watcher?.kill();
  await watcher?.exited;
  await app.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("the svelte template", () => {
  test(
    "mounts, fetches its asset and counts clicks",
    async () => {
      await expect(app.locator(".title")).toHaveText("Svelte Template");
      await expect(app.locator(".count")).toHaveText("0 clicks");
      await expect(app.locator(".hint")).toHaveText("Hello from assets/hello.txt, packaged with Svelte Template.");
      await app.locator(".button").nth(0).click();
      await expect(app.locator(".count")).toHaveText("1 click");
      await app.locator(".button").nth(0).click();
      await expect(app.locator(".count")).toHaveText("2 clicks");
      await app.locator(".button.ghost").click();
      await expect(app.locator(".count")).toHaveText("0 clicks");
    },
    T,
  );

  test(
    "an edit on disk is rebuilt by the watcher and hot-updated without a restart",
    async () => {
      // The host reads its slot's copy of the dist, as `sk dev`'s host reads the watcher's.
      const proc = Bun.spawn(["bun", "--no-install", "build.ts", join(REPO, entry), app.slotDir, "--watch"], {
        cwd: join(REPO, "packages/cli"),
        stdout: "pipe",
        stderr: "ignore",
        env: { ...process.env, SK_OWNER_PID: String(process.pid) },
      });
      watcher = proc;
      const reader = proc.stdout.getReader();
      let seen = "";
      while (!seen.includes("watching")) {
        const { value, done } = await reader.read();
        if (done) break;
        seen += new TextDecoder().decode(value);
      }
      bunExpect(seen).toContain("watching");
      void (async () => {
        for (;;) if ((await reader.read()).done) return;
      })();

      const file = join(dir, "src/App.svelte");
      writeFileSync(file, readFileSync(file, "utf8").replace(">Count up<", ">Count more<"));
      await expect(app.locator(".button").nth(0)).toHaveText("Count more", { timeout: 20_000 });
      await app.locator(".button").nth(0).click();
      await expect(app.locator(".count")).toHaveText("1 click");
      bunExpect(await app.eval(`String(__SK__.mountCount())`)).toBe("1");
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes("hot update") && e.text.includes("App.svelte"))).toBe(true);
      bunExpect(entries.some((e) => e.text.includes("hard restart"))).toBe(false);
    },
    T,
  );

  test(
    "nothing threw",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
