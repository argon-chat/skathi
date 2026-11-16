// One dev host runs any app's dev build: a relative URL resolves against the app being run, wherever it lives, not
// against the checkout the host was built in.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { cacheDistFor } from "../src/dist";
import { expect, launch } from "../src/index";

const dir = mkdtempSync(join(tmpdir(), "sk-app-root-"));
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
  rmSync(dir, { recursive: true, force: true });
  rmSync(cacheDistFor(entry), { recursive: true, force: true });
});

test("an app outside the checkout fetches its own assets", async () => {
  await expect(app.locator(".hint")).toHaveText("Hello from assets/hello.txt, packaged with App Root.");
}, 30_000);
