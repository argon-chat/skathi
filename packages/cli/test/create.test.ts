import { afterAll, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { modeVariant } from "../cli/build-app";
import { ConfigError, loadAppConfig } from "../cli/config";
import { create, createCommand, detectPackageManager, parseTemplate, runScript, TEMPLATES } from "../cli/create";

const dirs: string[] = [];
afterAll(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true });
});
const fresh = (name = "my-app") => {
  const d = mkdtempSync(join(tmpdir(), "sk-create-"));
  dirs.push(d);
  return join(d, name);
};
const read = (dir: string, file: string) => readFileSync(join(dir, file), "utf8");
const pkg = (dir: string) =>
  JSON.parse(read(dir, "package.json")) as {
    scripts: Record<string, string>;
    dependencies: Record<string, string>;
    devDependencies: Record<string, string>;
  };

test("the answers land in sk.config.json, and the builders read them back", () => {
  const dir = fresh();
  create(dir, { platforms: ["windows", "macos"], without: ["audio"], keepDevtools: false });
  const config = loadAppConfig(dir);
  expect(config.name).toBe("my-app");
  expect(config.productName).toBe("My App");
  expect(config.platforms).toEqual(["windows", "macos"]);
  expect(config.macos.bundleId).toBe("com.example.my-app");
  expect(modeVariant(config, "release")).toBe("no-devtools+no-audio");
  expect(modeVariant(config, "debug")).toBe("no-audio");
  expect(pkg(dir).scripts.dev).toBe("sk dev");
});

test("defaults: the showcase, this OS, every part, devtools only in sk build", () => {
  const dir = fresh();
  create(dir);
  const config = loadAppConfig(dir);
  expect(config.platforms).toHaveLength(1);
  expect(modeVariant(config, "release")).toBe("no-devtools");
  expect(modeVariant(config, "debug")).toBe("standard");
  expect(existsSync(join(dir, "src/router.ts"))).toBe(true);
});

test("the showcase copies its pages and media, and packs the media", () => {
  const dir = fresh();
  create(dir, { template: "showcase" });
  for (const f of [
    "src/main.ts",
    "src/App.vue",
    "src/router.ts",
    "src/styles.css",
    "src/stores/counter.ts",
    "src/pages/HomePage.vue",
    "src/pages/GraphicsPage.vue",
    "assets/hello.txt",
    "assets/media/aurora.png",
    "assets/media/sticker.tgs",
    "icon.png",
    "tsconfig.json",
    ".gitignore",
  ]) {
    expect(existsSync(join(dir, f))).toBe(true);
  }
  expect(loadAppConfig(dir).packs).toEqual([{ name: "media", match: ["assets/media/**"] }]);
  const png = readFileSync(join(dir, "assets/media/aurora.png"));
  expect([...png.subarray(1, 4)]).toEqual([0x50, 0x4e, 0x47]);
});

test("placeholders are filled in, escaped for code and verbatim in text", () => {
  const dir = fresh();
  create(dir, { name: 'say "hi"', template: "showcase" });
  expect(read(dir, "src/product.ts")).toContain('export const productName = "Say \\"hi\\"";');
  expect(read(dir, "src/product.ts")).toContain('export const appName = "say \\"hi\\"";');
  expect(read(dir, "assets/hello.txt")).toContain('packaged with Say "hi".');
  expect(read(dir, "src/pages/HomePage.vue")).toContain("{{ counter.count }}");
  for (const f of ["src/product.ts", "src/App.vue", "assets/hello.txt"]) expect(read(dir, f)).not.toContain("{{name");
});

test("the minimal template is one component, with OS chrome and no router", () => {
  const dir = fresh();
  create(dir, { template: "minimal" });
  expect(existsSync(join(dir, "src/App.vue"))).toBe(true);
  expect(existsSync(join(dir, "src/router.ts"))).toBe(false);
  expect(read(dir, "src/App.vue")).toContain('const productName = "My App";');
  const config = loadAppConfig(dir);
  expect(config.packs).toEqual([]);
  expect((config.raw.window as Record<string, unknown>).titleBarStyle).toBe("default");
});

test("pinia and vue-router are dependencies only of the templates that use them", () => {
  const showcase = fresh();
  create(showcase, { template: "showcase" });
  expect(Object.keys(pkg(showcase).dependencies).sort()).toEqual(["@skathi-native/runtime", "pinia", "vue-router"]);
  const minimal = fresh();
  create(minimal, { template: "minimal" });
  expect(Object.keys(pkg(minimal).dependencies)).toEqual(["@skathi-native/runtime"]);
  for (const dir of [showcase, minimal]) {
    expect(Object.keys(pkg(dir).devDependencies).sort()).toEqual(["@skathi-native/cli", "typescript", "vue-tsc"]);
    expect(pkg(dir).scripts.typecheck).toBe("vue-tsc --noEmit");
  }
});

test("an unknown template is refused with the names", async () => {
  expect(parseTemplate(" Minimal ")).toBe("minimal");
  expect(() => parseTemplate("react")).toThrow(ConfigError);
  expect(() => parseTemplate("react")).toThrow(TEMPLATES.join(", "));
  const dir = fresh();
  await expect(createCommand(dir, { template: "react", yes: true })).rejects.toThrow(/showcase, minimal/);
  expect(existsSync(dir)).toBe(false);
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
