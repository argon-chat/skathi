import { afterAll, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { modeVariant } from "../cli/build-app";
import { loadAppConfig } from "../cli/config";
import { create, detectPackageManager, runScript } from "../cli/create";

const dirs: string[] = [];
afterAll(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true });
});
const fresh = () => {
  const d = mkdtempSync(join(tmpdir(), "sk-create-"));
  dirs.push(d);
  return join(d, "my-app");
};

test("the answers land in sk.config.json, and the builders read them back", () => {
  const dir = fresh();
  create(dir, { platforms: ["windows", "macos"], without: ["audio"], keepDevtools: false, router: true });
  const config = loadAppConfig(dir);
  expect(config.name).toBe("my-app");
  expect(config.productName).toBe("My App");
  expect(config.platforms).toEqual(["windows", "macos"]);
  expect(config.macos.bundleId).toBe("com.example.my-app");
  expect(modeVariant(config, "release")).toBe("no-devtools+no-audio");
  expect(modeVariant(config, "debug")).toBe("no-audio");
  for (const f of ["package.json", "src/router.ts", "src/stores/counter.ts", "src/views/Home.vue", "icon.png"]) {
    expect(existsSync(join(dir, f))).toBe(true);
  }
  expect(JSON.parse(readFileSync(join(dir, "package.json"), "utf8")).scripts.dev).toBe("sk dev");
});

test("defaults: this OS, every part, devtools only in sk build, no router", () => {
  const dir = fresh();
  create(dir);
  const config = loadAppConfig(dir);
  expect(config.platforms).toHaveLength(1);
  expect(modeVariant(config, "release")).toBe("no-devtools");
  expect(modeVariant(config, "debug")).toBe("standard");
  expect(existsSync(join(dir, "src/router.ts"))).toBe(false);
});

test("keeping devtools keeps them in sk publish", () => {
  const dir = fresh();
  create(dir, { keepDevtools: true });
  expect(modeVariant(loadAppConfig(dir), "release")).toBe("standard");
});

test("nothing is overwritten", () => {
  const dir = fresh();
  create(dir);
  expect(() => create(dir)).toThrow(/already exists — nothing was written/);
});

test("the package manager comes from the one running this, else bun", () => {
  expect(detectPackageManager("pnpm/9.1.0 npm/? node/v22")).toBe("pnpm");
  expect(detectPackageManager("yarn/4.0.0 npm/? node/v22")).toBe("yarn");
  expect(detectPackageManager("")).toBe("bun");
  expect(runScript("pnpm", "dev")).toBe("pnpm dev");
  expect(runScript("npm", "dev")).toBe("npm run dev");
});
