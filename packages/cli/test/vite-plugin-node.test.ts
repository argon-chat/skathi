import { afterAll, expect, test } from "bun:test";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { packageDir } from "../runtime-path";

const app = mkdtempSync(join(tmpdir(), "sk-vite-node-plugin-"));
const cli = resolve(import.meta.dir, "..");
afterAll(() => rmSync(app, { recursive: true, force: true }));

test("a Node Vite config imports the published plugin and builds native Vue, Svelte and typed runes", () => {
  const installedCli = join(app, "node_modules/@skathi-native/cli");
  const pluginFile = join(installedCli, "dist/vite-plugin.mjs");
  mkdirSync(dirname(pluginFile), { recursive: true });
  copyFileSync(join(cli, "package.json"), join(installedCli, "package.json"));
  const emit = Bun.spawnSync([process.execPath, "run", "build:vite-plugin", "--outfile=" + pluginFile], { cwd: cli });
  expect(emit.exitCode, emit.stderr.toString()).toBe(0);
  for (const name of ["@skathi-native/runtime", "postcss", "svelte", "typescript", "vite", "vue"]) {
    const path = packageDir(name, cli);
    if (!path) throw new Error(`test dependency ${name} is not installed`);
    const link = join(app, "node_modules", name);
    mkdirSync(dirname(link), { recursive: true });
    symlinkSync(path, link, "junction");
  }
  writeFileSync(join(app, "App.vue"), '<template><div class="node-vue">Vue fixture</div></template>');
  writeFileSync(
    join(app, "Panel.svelte"),
    '<script>import { model } from "./model.svelte.ts";</script><button>{model.text}</button>',
  );
  writeFileSync(
    join(app, "model.svelte.ts"),
    'type Model = { text: string }; export const model = $state<Model>({text: "typed-svelte-rune"});',
  );
  writeFileSync(
    join(app, "main.js"),
    `
    import {createApp} from "vue";
    import App from "./App.vue";
    import Panel from "./Panel.svelte";
    import {available} from "native:standalone-bridge";
    globalThis.components = [createApp(App), Panel, available];
  `,
  );
  writeFileSync(
    join(app, "vite.config.mjs"),
    `
    import {nativeVitePlugins} from "@skathi-native/cli/vite";
    export default {
      plugins: await nativeVitePlugins({ appRoot: process.cwd(), nativeSvelte: true }),
      build: { write: false, minify: false, rolldownOptions: { input: "main.js", output: { format: "iife" } } },
    };
  `,
  );
  writeFileSync(
    join(app, "probe.mjs"),
    `
    import assert from "node:assert/strict";
    import {build} from "vite";
    assert.equal(typeof Bun, "undefined");
    const {output} = await build({logLevel:"silent"});
    const code = output.find(file => file.type === "chunk" && file.isEntry).code;
    assert.match(code, /__sk_create_root/);
    assert.match(code, /node-vue/);
    assert.match(code, /typed-svelte-rune/);
    assert.match(code, /__sk_native_module/);
    assert.match(code, /standalone-bridge/);
  `,
  );
  const node = Bun.spawnSync(["node", "probe.mjs"], { cwd: app });
  expect(node.exitCode, node.stdout.toString() + node.stderr.toString()).toBe(0);
}, 30_000);
