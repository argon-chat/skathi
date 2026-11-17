// One dev host runs any app's dev build: a relative URL resolves against the app being run, wherever it lives, not
// against the checkout the host was built in.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { cacheDistFor } from "../src/dist";
import { expect, launch } from "../src/index";

// Vue is the app's own dependency: install it beside the app, as `bun install` would.
const root = mkdtempSync(join(tmpdir(), "sk-app-root-"));
const vue: { version: string } = JSON.parse(
  readFileSync(resolve(import.meta.dir, "../../../node_modules/vue/package.json"), "utf8"),
);
writeFileSync(join(root, "package.json"), JSON.stringify({ private: true, dependencies: { vue: vue.version } }));
const install = spawnSync(process.execPath, ["install"], { cwd: root, encoding: "utf8" });
if (install.status !== 0) throw new Error(`bun install vue failed:\n${install.stderr}`);
const dir = join(root, "app");
create(dir, { name: "app-root", template: "minimal", platforms: ["windows"] });
const entry = join(dir, "src/main.ts");
const app = await launch({
  app: entry,
  window: [640, 480],
  label: "app-root",
  env: { SK_APP_ROOT: "" },
});
afterAll(async () => {
  await app.close();
  rmSync(root, { recursive: true, force: true });
  rmSync(cacheDistFor(entry), { recursive: true, force: true });
});

test("an app outside the checkout fetches its own assets", async () => {
  await expect(app.locator(".hint")).toHaveText("Hello from assets/hello.txt, packaged with App Root.");
}, 30_000);
