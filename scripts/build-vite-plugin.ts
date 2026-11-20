import { mkdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const cli = resolve(import.meta.dir, "../packages/cli");
const fromCli = createRequire(join(cli, "package.json"));
const out = resolve(
  Bun.argv.find((arg) => arg.startsWith("--outfile="))?.slice(10) ?? join(cli, "dist/vite-plugin.mjs"),
);
const pluginEntry = fromCli.resolve("@vitejs/plugin-vue");
const fromPlugin = createRequire(pluginEntry);
const fromVue = createRequire(fromCli.resolve("vue/package.json"));
const reactivity = dirname(fromVue.resolve("@vue/reactivity/package.json"));
const shared = dirname(createRequire(join(reactivity, "package.json")).resolve("@vue/shared/package.json"));
const notices = [
  dirname(fromCli.resolve("@vitejs/plugin-vue/package.json")),
  dirname(fromPlugin.resolve("@rolldown/pluginutils/package.json")),
  reactivity,
  shared,
].map((dir) => readFileSync(join(dir, "LICENSE"), "utf8"));

async function build(file: string, config: Bun.BuildConfig): Promise<void> {
  const result = await Bun.build(config);
  if (!result.success || result.outputs.length !== 1) throw new AggregateError(result.logs, `Could not build ${file}`);
  await Bun.write(file, result.outputs[0]!);
}

mkdirSync(dirname(out), { recursive: true });
await build(out, {
  entrypoints: [join(cli, "vite/plugin.ts")],
  target: "node",
  format: "esm",
  packages: "external",
});
await build(join(dirname(out), "vue-adapter.mjs"), {
  entrypoints: [pluginEntry],
  target: "node",
  format: "esm",
  external: ["vite"],
  banner: "/*!\n" + [...new Set(notices)].join("\n\n") + "\n*/",
  define: { "process.env.NODE_ENV": '"production"' },
  // The adapter uses Vue's reactivity primitives for its own build-time state.
  // Component compilation still uses the target application's vue/compiler-sfc.
  plugins: [
    {
      name: "build-time-vue-reactivity",
      setup(build) {
        build.onResolve({ filter: /^vue$/ }, () => ({ path: join(reactivity, "dist/reactivity.esm-bundler.js") }));
      },
    },
  ],
});
