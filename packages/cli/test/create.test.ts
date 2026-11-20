import { afterAll, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import cliPackage from "../package.json";
import { modeVariant } from "../cli/build-app";
import { ConfigError, loadAppConfig, viteConfigFile } from "../cli/config";
import {
  create,
  createCommand,
  detectPackageManager,
  parseTemplate,
  runScript,
  TEMPLATES,
  TEMPLATES_DIR,
} from "../cli/create";

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
  create(dir, { platforms: ["windows", "macos"], keepDevtools: false });
  const config = loadAppConfig(dir);
  expect(config.name).toBe("my-app");
  expect(config.productName).toBe("My App");
  expect(config.platforms).toEqual(["windows", "macos"]);
  expect(config.macos.bundleId).toBe("com.example.my-app");
  expect(config.publishDevtools).toBe(false);
  expect(modeVariant(config, "release")).toBe("no-devtools");
  expect(modeVariant(config, "debug")).toBe("standard");
  expect(pkg(dir).scripts.dev).toBe("sk dev");
});

test("defaults: the showcase, this OS, devtools only in sk build", () => {
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
    ".vscode/settings.json",
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

test("every built-in and gallery template configures Vite+ for the native build", () => {
  for (const template of TEMPLATES) {
    const dir = fresh(template);
    create(dir, { template });
    const manifest = pkg(dir);
    const plugin = manifest.dependencies.svelte ? "@sveltejs/vite-plugin-svelte" : "@vitejs/plugin-vue";
    expect(viteConfigFile(dir)).toBe(join(dir, "vite.config.ts"));
    expect(read(dir, "vite.config.ts")).toContain('from "vite-plus"');
    expect(read(dir, "vite.config.ts")).toContain(`from "${plugin}"`);
    expect(manifest.devDependencies[plugin]).toBeDefined();
    expect(manifest.devDependencies["vite-plus"]).toBe(cliPackage.peerDependencies["vite-plus"]);
    expect(manifest.devDependencies["@skathi-native/cli"]).toBe(`^${cliPackage.version}`);
    expect(manifest.dependencies["@skathi-native/runtime"]).toBe(`^${cliPackage.version}`);
    expect(manifest.scripts.build).toBe("sk build");
  }
});

test("every template points VS Code at the runtime's CSS custom data", () => {
  for (const template of TEMPLATES) {
    const dir = fresh();
    create(dir, { template });
    const settings = JSON.parse(read(dir, ".vscode/settings.json")) as Record<string, unknown>;
    expect(settings["css.customData"]).toEqual(["./node_modules/@skathi-native/runtime/css-data.json"]);
  }
});

test("vue is a dependency of the Vue templates, pinia and vue-router only of the one that uses them", () => {
  const showcase = fresh();
  create(showcase, { template: "showcase" });
  expect(Object.keys(pkg(showcase).dependencies).sort()).toEqual([
    "@skathi-native/runtime",
    "pinia",
    "vue",
    "vue-router",
  ]);
  const minimal = fresh();
  create(minimal, { template: "minimal" });
  expect(Object.keys(pkg(minimal).dependencies).sort()).toEqual(["@skathi-native/runtime", "vue"]);
  for (const dir of [showcase, minimal]) {
    expect(Object.keys(pkg(dir).devDependencies).sort()).toEqual([
      "@skathi-native/cli",
      "@vitejs/plugin-vue",
      "typescript",
      "vite-plus",
      "vue-tsc",
    ]);
    expect(pkg(dir).scripts.typecheck).toBe("vue-tsc --noEmit");
  }
});

test("the svelte templates depend on svelte at the version the runtime accepts and check with svelte-check", () => {
  const svelte = (
    JSON.parse(read(join(import.meta.dir, "../../runtime"), "package.json")) as {
      peerDependencies: Record<string, string>;
    }
  ).peerDependencies.svelte!;
  for (const template of ["svelte", "player"] as const) {
    const dir = fresh();
    create(dir, { template });
    expect(read(dir, "src/main.ts")).toContain('from "@skathi-native/runtime/svelte"');
    expect(Object.keys(pkg(dir).dependencies).sort()).toEqual(["@skathi-native/runtime", "svelte"]);
    expect(pkg(dir).dependencies.svelte).toBe(svelte);
    expect(Object.keys(pkg(dir).devDependencies).sort()).toEqual([
      "@skathi-native/cli",
      "@sveltejs/vite-plugin-svelte",
      "svelte-check",
      "typescript",
      "vite-plus",
    ]);
    expect(pkg(dir).scripts.typecheck).toBe("svelte-check");
    expect((JSON.parse(read(dir, "tsconfig.json")) as { include: string[] }).include).toEqual([
      "src/**/*.ts",
      "src/**/*.svelte",
    ]);
  }
  const dir = fresh();
  create(dir, { template: "svelte" });
  expect(read(dir, "src/App.svelte")).toContain('const productName = "My App";');
});

test("the player comes from the templates checkout with its own icon, and packs the media", () => {
  const dir = fresh();
  create(dir, { template: "player" });
  expect(read(dir, "src/lib/product.ts")).toContain('export const productName = "My App";');
  expect(readFileSync(join(dir, "icon.png"))).toEqual(
    readFileSync(join(import.meta.dir, "../../../templates/player/icon.png")),
  );
  const config = loadAppConfig(dir);
  expect(config.packs).toEqual([{ name: "media", match: ["assets/music/**", "assets/covers/**"] }]);
  expect((config.raw.window as Record<string, unknown>).titleBarStyle).toBe("hidden");
});

test("the weather template comes from the templates checkout with its own icon, and packs its textures", () => {
  const dir = fresh();
  create(dir, { template: "weather" });
  expect(readFileSync(join(dir, "icon.png"))).toEqual(
    readFileSync(join(import.meta.dir, "../../../templates/weather/icon.png")),
  );
  expect(loadAppConfig(dir).packs).toEqual([{ name: "textures", match: ["assets/textures/**"] }]);
  expect(Object.keys(pkg(dir).dependencies).sort()).toEqual(["@skathi-native/runtime", "vue"]);
  expect(pkg(dir).scripts.typecheck).toBe("vue-tsc --noEmit");
});

test("a template without its own icon gets the Skathi one", () => {
  const dir = fresh();
  create(dir, { template: "minimal" });
  expect(readFileSync(join(dir, "icon.png"))).toEqual(readFileSync(join(TEMPLATES_DIR, "icon.png")));
});

test("draw scaffolds the gallery template, and its window keeps the template's own backdrop", () => {
  const dir = fresh();
  create(dir, { template: "draw" });
  const source = join(import.meta.dir, "../../../templates/draw");
  expect(read(dir, "src/App.vue")).toBe(read(source, "src/App.vue"));
  expect(readFileSync(join(dir, "icon.png"))).toEqual(readFileSync(join(source, "icon.png")));
  expect(loadAppConfig(dir).raw.window).toMatchObject({ titleBarStyle: "hidden", backdrop: "none" });
});

test("an unknown template is refused before anything is written", async () => {
  expect(parseTemplate(" Minimal ")).toBe("minimal");
  expect(() => parseTemplate("react")).toThrow(ConfigError);
  const dir = fresh();
  await expect(createCommand(dir, { template: "react", yes: true })).rejects.toThrow(ConfigError);
  expect(existsSync(dir)).toBe(false);
});

test("--framework picks minimal's flavour and refuses a template in the other framework", async () => {
  const svelte = fresh();
  expect(await createCommand(svelte, { template: "minimal", framework: "svelte", yes: true, color: false })).toBe(0);
  expect(existsSync(join(svelte, "src/App.svelte"))).toBe(true);
  const only = fresh();
  expect(await createCommand(only, { framework: "svelte", yes: true, color: false })).toBe(0);
  expect(existsSync(join(only, "src/App.svelte"))).toBe(true);
  const vue = fresh();
  expect(await createCommand(vue, { template: "svelte", framework: "vue", yes: true, color: false })).toBe(0);
  expect(existsSync(join(vue, "src/App.vue"))).toBe(true);
  const weather = fresh();
  await expect(createCommand(weather, { template: "weather", framework: "svelte", yes: true })).rejects.toThrow(
    ConfigError,
  );
  expect(existsSync(weather)).toBe(false);
});

test("keeping devtools keeps them in sk publish", () => {
  const dir = fresh();
  create(dir, { keepDevtools: true });
  expect(modeVariant(loadAppConfig(dir), "release")).toBe("standard");
});

test("nothing is overwritten", () => {
  const dir = fresh();
  create(dir);
  expect(() => create(dir)).toThrow();
});

test("the package manager comes from the one running this, else bun", () => {
  expect(detectPackageManager("pnpm/9.1.0 npm/? node/v22")).toBe("pnpm");
  expect(detectPackageManager("yarn/4.0.0 npm/? node/v22")).toBe("yarn");
  expect(detectPackageManager("")).toBe("bun");
  expect(runScript("pnpm", "dev")).toBe("pnpm dev");
  expect(runScript("npm", "dev")).toBe("npm run dev");
});
