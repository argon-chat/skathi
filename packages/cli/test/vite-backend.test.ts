import { afterAll, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { KIND, NAMES, readSkpk } from "../cli/skpk";
import type { CheckReport } from "../check/schema";
import cliPackage from "../package.json";
import { packageDir } from "../runtime-path";

const scratch = mkdtempSync(join(tmpdir(), "sk-vite-backend-"));
const cli = resolve(import.meta.dir, "../cli/sk.ts");
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

function appWith(files: Record<string, string>): string {
  const app = mkdtempSync(join(scratch, "app-"));
  for (const [name, contents] of Object.entries(files)) {
    const path = join(app, name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, contents);
  }
  return app;
}

function run(...args: string[]) {
  return Bun.spawnSync([process.execPath, cli, ...args], {
    cwd: scratch,
    stdout: "pipe",
    stderr: "pipe",
  });
}

function linkPackages(app: string, ...names: string[]): void {
  for (const name of names) {
    const installed = packageDir(name, import.meta.dir);
    if (!installed) throw new Error(`test dependency ${name} is not installed`);
    const link = join(app, "node_modules", name);
    mkdirSync(dirname(link), { recursive: true });
    symlinkSync(installed, link, "junction");
  }
}

test("sk build uses the target app's Vite config, Vue plugin and assets in the native payload", () => {
  const app = appWith({
    "sk.config.json": JSON.stringify({
      name: "vite-test",
      version: "1.0.0",
      window: { title: "Native payload" },
      assets: ["assets"],
      packs: [{ name: "media", match: ["assets/media/**"] }],
    }),
    "package.json": JSON.stringify({ name: "vite-test", type: "module", dependencies: { vue: "^3.5" } }),
    "web-dist/keep.txt": "existing web build",
    "mode.txt": "plugin-ran",
    "src/asset.dat": "binary asset from Vite",
    "assets/media/sticker.tgs": "sticker pack bytes",
    "src/label.ts": 'export const label = "user-vite-alias";',
    "src/main.ts": 'import {createApp} from "vue"; import App from "./App.vue"; createApp(App).mount();',
    "src/App.vue": `
      <script setup>
        import { label } from "@app/label";
        import generated from "virtual:native-fixture";
        import asset from "./asset.dat?url&no-inline";
      </script>
      <template><div class="native-fixture" :data-plugin="generated" :data-asset="asset">[[ label ]]<lottie src="assets/media/sticker.tgs" /></div></template>
      <style scoped>.native-fixture { padding-top: 1px; }</style>
    `,
    "vite.config.mts": `
      import { readFileSync } from "node:fs";
      import { fileURLToPath } from "node:url";
      import vue from "@vitejs/plugin-vue";
      export default async ({ command, mode }) => ({
        resolve: { alias: { "@app": fileURLToPath(new URL("./src", import.meta.url)) } },
        build: { outDir: "web-dist", emptyOutDir: true, rollupOptions: { input: "only-on-web.html" } },
        css: { postcss: { plugins: [{
          postcssPlugin: "fixture-padding",
          Declaration: { "padding-top": decl => { decl.value = "17px"; } },
        }] } },
        plugins: [
          vue({ template: { compilerOptions: { delimiters: ["[[", "]]"] } } }),
          {
            name: "fixture-virtual",
            apply: "build",
            resolveId(id) { if (id === "virtual:native-fixture") return "\\0native-fixture"; },
            load(id) {
              if (id === "\\0native-fixture") return "export default " + JSON.stringify(readFileSync("mode.txt", "utf8") + "-" + command + "-" + mode + "-" + process.env.SKATHI_BUILD);
            },
          },
        ],
      });
    `,
  });
  linkPackages(app, "vue", "vite", "@vitejs/plugin-vue");
  writeFileSync(join(scratch, "vite.config.mjs"), 'throw new Error("Vite loaded the caller config");');

  const out = join(scratch, "native");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const script = new TextDecoder().decode(payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!));
  expect(script).toContain("user-vite-alias");
  expect(script).toContain("plugin-ran-build-production-1");
  expect(script).not.toContain("[[ label ]]");
  expect(script).toContain("__sk_create_root");
  expect(script).toContain("__SK_CSS__");
  expect(script).toMatch(/\.native-fixture\[data-v-[\da-z]+\]\s*\{(?:\s|\\n)*padding-top:\s*17px/);

  const asset = payload.entries.find((entry) => entry.kind === KIND.ASSET && entry.name.endsWith(".dat"));
  expect(asset).toBeDefined();
  expect(new TextDecoder().decode(payload.bytes(asset!))).toBe("binary asset from Vite");
  expect(script).toContain("/" + asset!.name);
  const pack = readSkpk(readFileSync(join(out, ".sk", "media.skpk")));
  const sticker = pack.find(KIND.ASSET, "assets/media/sticker.tgs");
  expect(sticker).toBeDefined();
  expect(new TextDecoder().decode(pack.bytes(sticker!))).toBe("sticker pack bytes");
  expect(script).toContain("assets/media/sticker.tgs");
  expect(payload.entries.some((entry) => entry.name.endsWith(".tgs"))).toBe(false);
  expect(readdirSync(join(app, "web-dist"))).toEqual(["keep.txt"]);
  expect(readFileSync(join(app, "web-dist", "keep.txt"), "utf8")).toBe("existing web build");
}, 120_000);

test("new URL resolves an emitted Vite asset on the native page origin", () => {
  const image = "packed bytes".repeat(500);
  const app = appWith({
    "vite.config.mjs": "export default {};",
    "src/main.ts": 'globalThis.result = [import.meta.url, new URL("./image.png", import.meta.url).href];',
    "src/image.png": image,
  });
  const out = join(scratch, "native-url");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode, result.stderr.toString()).toBe(0);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const script = new TextDecoder().decode(payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!));
  const globals = {} as { result: [string, string] };
  new Function("globalThis", script)(globals);
  const [moduleUrl, assetUrl] = globals.result;
  expect(moduleUrl).toBe("app://redjs/app.prod.js");
  expect(assetUrl).toStartWith("app://redjs/assets/");
  const asset = payload.find(KIND.ASSET, new URL(assetUrl).pathname.slice(1));
  expect(asset).toBeDefined();
  expect(new TextDecoder().decode(payload.bytes(asset!))).toBe(image);
}, 120_000);

test("a Svelte-only install builds the native payload without bringing in Vue", () => {
  // An installed Vue adapter would auto-install its required Vue peer on a clean Bun install.
  expect(cliPackage.dependencies).not.toHaveProperty("@vitejs/plugin-vue");
  expect(cliPackage.dependencies).not.toHaveProperty("vue");
  const app = appWith({
    "package.json": JSON.stringify({ type: "module", dependencies: { svelte: "^5" } }),
    "main.ts": 'import {mount} from "@skathi-native/runtime/svelte"; import App from "./App.svelte"; mount(App);',
    "App.svelte": "<button>svelte-only-fixture</button>",
    "vite.config.mjs": `export default { plugins: [{
      name: "check-renderer",
      generateBundle(_, bundle) {
        for (const chunk of Object.values(bundle)) {
          if (chunk.type !== "chunk") continue;
          const vue = Object.keys(chunk.modules).filter(id => /[/\\\\]node_modules[/\\\\](?:vue|@vue)[/\\\\]/.test(id));
          if (vue.length) this.error("Unexpected Vue modules: " + vue.join(", "));
        }
      },
    }] };`,
  });
  linkPackages(app, "svelte");
  const out = join(scratch, "svelte-only");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const script = new TextDecoder().decode(payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!));
  expect(script).toContain("svelte-only-fixture");
}, 120_000);

test("serve-only Vue and Svelte plugins leave their native build compilers active", () => {
  const app = appWith({
    "package.json": JSON.stringify({ type: "module", dependencies: { vue: "^3.5", svelte: "^5" } }),
    "main.ts": 'import App from "./App.vue"; import Panel from "./Panel.svelte"; globalThis.pair = [App, Panel];',
    "App.vue": '<template><div class="native-fallback-vue">Vue</div></template>',
    "Panel.svelte": "<button>native-fallback-svelte</button>",
    "vite.config.mjs": `import vue from "@vitejs/plugin-vue";
      const environments = new WeakSet();
      export default { plugins: [
        Object.assign(vue(), {apply:"serve"}),
        { name:"vite-plugin-svelte:config", apply(config,env) {
          if (environments.has(env)) throw new Error("apply already evaluated for this environment");
          environments.add(env);
          return env.command === "serve";
        } },
      ] };`,
  });
  linkPackages(app, "vue", "vite", "@vitejs/plugin-vue", "svelte");
  const out = join(scratch, "serve-only-frameworks");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const script = new TextDecoder().decode(payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!));
  expect(script).toContain("native-fallback-vue");
  expect(script).toContain("native-fallback-svelte");
}, 120_000);

test("sk uses the Vite+ build API from the target app and checks CSS generated by its plugins", () => {
  const app = appWith({
    "package.json": JSON.stringify({ name: "plus-app", type: "module", devDependencies: { "vite-plus": "^1.7.0" } }),
    "main.ts": 'import "./style.css"; console.log(APP_TOOLCHAIN);',
    "style.css": ".card { color: red; }",
    "vite.config.ts": `import { defineConfig } from "vite-plus";
      export default defineConfig({ css: { postcss: { plugins: [{
        postcssPlugin: "fixture", Declaration: { color(decl) { decl.replaceWith({ prop: "float", value: "left" }); } }
      }] } } });`,
    "node_modules/vite-plus/package.json": JSON.stringify({
      name: "vite-plus",
      type: "module",
      version: "1.7.0",
      exports: { ".": "./index.js", "./package.json": "./package.json" },
    }),
    "node_modules/vite-plus/index.js": `import { build as viteBuild } from "vite";
      export * from "vite";
      export function build(config) {
        return viteBuild({ ...config, define: { ...config.define, APP_TOOLCHAIN: '"app-local-toolchain"' } });
      }`,
  });
  symlinkSync(packageDir("vite", import.meta.dir)!, join(app, "node_modules/vite"), "junction");
  const out = join(scratch, "vite-plus-api");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const script = new TextDecoder().decode(payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!));
  expect(script).toContain("app-local-toolchain");
  const checked = run("check", app, "--format", "json", "--no-deps");
  expect(checked.exitCode, checked.stdout.toString() + checked.stderr.toString()).toBe(0);
  const diagnostics = (JSON.parse(checked.stdout.toString()) as CheckReport).diagnostics;
  expect(diagnostics).toContainEqual(expect.objectContaining({ code: "SK4001", file: "main.ts" }));
}, 60_000);

test("the native output uses the IIFE hooks and asset names from a multi-output Vite config", () => {
  const app = appWith({
    "main.ts": 'import sample from "./file.dat?url&no-inline"; console.log(sample);',
    "file.dat": "sample",
    "web/keep.txt": "existing",
    "vite.config.mjs": `
      const nativeOutput = {
        format: "iife", dir: "web", entryFileNames: "browser-only.js", assetFileNames: "media/[name][extname]",
        banner: 'globalThis.nativeBanner = true;', footer: 'globalThis.nativeFooter = true;',
        plugins: [{
          name: "output-hook",
          generateBundle(options) {
            this.emitFile({type:"asset", fileName:"styles.json", source:"output hook " + options.format});
          }
        }]
      };
      export default {
        build: {outDir: "web", emptyOutDir: true, write: true, rolldownOptions: {
          output: [{format: "es", banner:"globalThis.browserOutput = true;"}, nativeOutput]
        }},
        plugins: [{name:"web-output", configEnvironment() {return {build:{outDir:"web", write:true, emptyOutDir:true}}}}]
      };
    `,
  });
  const out = join(scratch, "output-hooks");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const read = (kind: number, name: string) => new TextDecoder().decode(payload.bytes(payload.find(kind, name)!));
  const script = read(KIND.SCRIPT, NAMES.SCRIPT);
  expect(script).toContain("nativeBanner");
  expect(script).toContain("nativeFooter");
  expect(script).not.toContain("browserOutput");
  expect(script).toContain("/media/file.dat");
  expect(read(KIND.ASSET, "media/file.dat")).toBe("sample");
  expect(read(KIND.ASSET, "styles.json")).toBe("output hook iife");
  expect(JSON.parse(readFileSync(join(out, ".sk", "styles.json"), "utf8")).compiledCss).toBe("");
  expect(readdirSync(join(app, "web"))).toEqual(["keep.txt"]);
});

test("an output plugin cannot switch the native payload to an ESM build", () => {
  const app = appWith({
    "main.ts": "console.log(123);",
    "vite.config.mjs": `export default { plugins: [{name:"browser-format",outputOptions(o){return {...o,format:"es"};}}] };`,
  });
  const out = join(scratch, "browser-format");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode).not.toBe(0);
  expect(result.stdout.toString() + result.stderr.toString()).toContain("required Skathi output format");
  expect(existsSync(join(out, ".sk", "app.skpk"))).toBe(false);
});

test("an app plugin cannot add a second JavaScript entry to the native payload", () => {
  const app = appWith({
    "main.ts": 'console.log("main");',
    "web-entry.ts": 'console.log("web entry");',
    "vite.config.mjs": `import { fileURLToPath } from "node:url";
      export default {plugins:[{name:"extra-entry",buildStart(){
        this.emitFile({type:"chunk",id:fileURLToPath(new URL("./web-entry.ts",import.meta.url)),fileName:"web-entry.js"});
      }}]};`,
  });
  const out = join(scratch, "extra-entry");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode).not.toBe(0);
  expect(result.stdout.toString() + result.stderr.toString()).toMatch(/multiple|self-contained|code.splitting/i);
  expect(existsSync(join(out, ".sk", "app.skpk"))).toBe(false);
});

test("sk build reports a failing app Vite config even when its entry can build without plugins", () => {
  const app = appWith({
    "main.ts": 'console.log("plain entry");',
    "vite.config.mjs": 'throw new Error("fixture Vite config rejected this build");',
  });
  const out = join(scratch, "failed-native");
  const result = run("build", app, "--out", out, "--payload-only");
  expect(result.exitCode).not.toBe(0);
  expect(result.stdout.toString() + result.stderr.toString()).toContain("fixture Vite config rejected this build");
  expect(existsSync(join(out, ".sk", "app.skpk"))).toBe(false);
}, 30_000);

test("sk check reports the Vite plugin error when an app module cannot be transformed", () => {
  const app = appWith({
    "main.ts": 'console.log("valid without plugin");',
    "vite.config.mjs": `
      export default { plugins: [{
        name: "reject-main",
        transform(_code, id) { if (id.endsWith("/main.ts")) this.error("fixture failed to transform main.ts"); },
      }] };
    `,
  });
  const checked = run("check", app, "--format", "json", "--no-deps");
  expect(checked.exitCode, checked.stdout.toString() + checked.stderr.toString()).toBe(2);
  const report = JSON.parse(checked.stdout.toString()) as CheckReport;
  expect(
    report.diagnostics.some(
      (d) => d.code === "SK0004" && (d.message + d.help).includes("fixture failed to transform main.ts"),
    ),
  ).toBe(true);
}, 30_000);

test("sk check distinguishes a live built-in write from Svelte's removed development patches", () => {
  const app = appWith({
    "package.json": JSON.stringify({ type: "module", dependencies: { svelte: "^5.59" } }),
    "vite.config.mjs": "export default {};",
    "src/main.ts": `import { mount } from "@skathi-native/runtime/svelte";
      import App from "./App.svelte";
      mount(App);
      Array.prototype.includes = () => false;`,
    "src/App.svelte": "<p>Native Svelte app</p>",
  });
  for (const name of ["svelte", "@skathi-native/runtime"]) {
    const link = join(app, "node_modules", name);
    mkdirSync(dirname(link), { recursive: true });
    symlinkSync(packageDir(name, import.meta.dir)!, link, "junction");
  }
  for (const nodeEnv of ["production", "development"]) {
    const checked = Bun.spawnSync([process.execPath, cli, "check", app, "--format", "json"], {
      cwd: scratch,
      env: { ...process.env, NODE_ENV: nodeEnv },
    });
    expect(checked.exitCode, checked.stdout.toString() + checked.stderr.toString()).toBe(1);
    const report = JSON.parse(checked.stdout.toString()) as CheckReport;
    expect(report.diagnostics.filter((d) => d.code === "SK1001").map((d) => d.file)).toEqual(["src/main.ts"]);
  }
}, 60_000);

test.each([false, true])(
  "Unicode source stays ASCII in the packaged script and AOT input (Vite: %s)",
  (vite) => {
    const app = appWith({
      "main.ts": [
        "/*! Привет */",
        "const слово = 'ёж'; const re = /[ёж]+/u;",
        "globalThis.answer = [re.test(слово), re.source, `текст: ${слово}`, String.raw`пароль\\ж`];",
      ].join("\n"),
      ...(vite ? { "vite.config.mjs": "export default {};" } : {}),
    });
    const out = join(scratch, "unicode-aot-" + vite);
    const result = run("build", app, "--out", out, "--payload-only", "--aot-inputs");
    expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
    const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
    const bytes = payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!);
    expect(bytes.every((b) => b < 0x80)).toBe(true);
    const script = new TextDecoder().decode(bytes);
    expect(new Function("globalThis", script + "\nreturn globalThis.answer;")({})).toEqual([
      true,
      "[ёж]+",
      "текст: ёж",
      "пароль\\ж",
    ]);
    const manifest = JSON.parse(readFileSync(join(out, ".sk", "aot-inputs", "manifest.json"), "utf8"));
    expect(manifest.script.sha256).toBe(createHash("sha256").update(bytes).digest("hex"));
  },
  60_000,
);

test("sk check and --aot-inputs see a typed source imported only by a Vite plugin", () => {
  const app = appWith({
    "sk.config.json": JSON.stringify({ name: "vite-graph-test", version: "1.0.0" }),
    "package.json": '{"type":"module"}',
    "src/main.ts": `import "virtual:bootstrap";
      import logo from "./logo.svg?url";
      import example from "./example.ts?raw";
      console.log(logo, example);`,
    "src/logo.svg": '<svg xmlns="http://www.w3.org/2000/svg"/>',
    "src/example.ts": "this is displayed as text",
    "src/reachable.ts": 'export const timeout: number = 10; setTimeout("tick()", timeout);',
    "src/unreachable.ts": 'setTimeout("never()", 20);',
    "vite.config.mjs": `
      export default { plugins: [{
        name: "native-bootstrap",
        resolveId(id) { if (id === "virtual:bootstrap") return "\\0bootstrap"; },
        load(id) { if (id === "\\0bootstrap") return 'import "/src/reachable.ts";'; },
      }] };
    `,
  });
  const checked = run("check", app, "--format", "json", "--no-deps");
  expect(checked.exitCode, checked.stdout.toString() + checked.stderr.toString()).toBe(0);
  const report = JSON.parse(checked.stdout.toString()) as CheckReport;
  expect(report.diagnostics).toContainEqual(expect.objectContaining({ code: "SK2003", file: "src/reachable.ts" }));
  expect(report.diagnostics.some((d) => d.file === "src/example.ts")).toBe(false);
  expect(report.diagnostics.some((d) => d.file === "src/unreachable.ts")).toBe(false);
  const out = join(scratch, "native-aot");
  const result = run("build", app, "--out", out, "--payload-only", "--aot-inputs");
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const archive = join(out, ".sk", "aot-inputs");
  const manifest = JSON.parse(readFileSync(join(archive, "manifest.json"), "utf8")) as {
    script: { sha256: string };
    files: { path: string; kind: string; blob: string }[];
  };
  const source = manifest.files.find((f) => f.path === "src/reachable.ts" && f.kind === "source");
  expect(source).toBeDefined();
  expect(readFileSync(join(archive, source!.blob), "utf8")).toBe(readFileSync(join(app, "src/reachable.ts"), "utf8"));
  expect(manifest.files.some((f) => f.path === "src/unreachable.ts")).toBe(false);
  const payload = readSkpk(readFileSync(join(out, ".sk", "app.skpk")));
  const bytes = payload.bytes(payload.find(KIND.SCRIPT, NAMES.SCRIPT)!);
  expect(manifest.script.sha256).toBe(createHash("sha256").update(bytes).digest("hex"));
}, 120_000);
