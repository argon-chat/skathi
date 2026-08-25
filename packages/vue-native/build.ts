// vue-native bundler, v1: Bun.build + an @vue/compiler-sfc plugin.
// Usage: bun build.ts <app-entry.js> [outfile]
//
// - .vue files: <script setup> + <template> compiled together
//   (inlineTemplate), <style scoped> rewritten to [data-v-hash] selectors and
//   collected into globalThis.__VN_CSS__ at module-eval time.
// - hoistStatic is FORCED OFF: our host has no insertStaticContent, and Vue's
//   stringified-static path must never be emitted (research F7).
// - "vue" and "vue-native" resolve to our runtime module.

import { parse, compileScript, compileStyle } from "@vue/compiler-sfc";
import { resolve, dirname } from "path";

const entry = resolve(process.argv[2] ?? "../../apps/hello/src/main.js");
const outfile = resolve(process.argv[3] ?? "dist/app.js");
const runtimePath = resolve(import.meta.dir, "src/runtime.js");

const vuePlugin = {
  name: "vue-sfc",
  setup(build: any) {
    build.onResolve({ filter: /^(vue|vue-native)$/ }, () => ({ path: runtimePath }));

    build.onLoad({ filter: /\.vue$/ }, async (args: any) => {
      const source = await Bun.file(args.path).text();
      const { descriptor, errors } = parse(source, { filename: args.path });
      if (errors.length) {
        throw new Error(`${args.path}: ${errors.map((e) => e.message).join("; ")}`);
      }
      const id = Bun.hash(args.path + source).toString(16).slice(0, 8);
      const scopeId = `data-v-${id}`;
      const hasScoped = descriptor.styles.some((s) => s.scoped);

      let code: string;
      if (descriptor.scriptSetup || descriptor.script) {
        const compiled = compileScript(descriptor, {
          id,
          inlineTemplate: !!descriptor.scriptSetup,
          genDefaultAs: "_sfc_main",
          templateOptions: {
            scoped: hasScoped,
            compilerOptions: { hoistStatic: false },
          },
        });
        code = compiled.content;
      } else {
        // Template-only component.
        const { compileTemplate } = await import("@vue/compiler-sfc");
        const t = compileTemplate({
          source: descriptor.template?.content ?? "",
          filename: args.path,
          id,
          scoped: hasScoped,
          compilerOptions: { hoistStatic: false },
        });
        code = `${t.code}\nconst _sfc_main = { render };`;
      }

      const cssChunks: string[] = [];
      for (const style of descriptor.styles) {
        const out = compileStyle({
          source: style.content,
          filename: args.path,
          id: scopeId,
          scoped: style.scoped,
        });
        if (out.errors.length) {
          throw new Error(`${args.path} <style>: ${out.errors.join("; ")}`);
        }
        cssChunks.push(out.code);
      }

      const footer = [
        hasScoped ? `_sfc_main.__scopeId = ${JSON.stringify(scopeId)};` : "",
        cssChunks.length
          ? `(globalThis.__VN_CSS__ ??= []).push(${JSON.stringify(cssChunks.join("\n"))});`
          : "",
        `export default _sfc_main;`,
      ].join("\n");

      return { contents: `${code}\n${footer}`, loader: "js" };
    });
  },
};

const result = await Bun.build({
  entrypoints: [entry],
  format: "iife",
  target: "browser",
  plugins: [vuePlugin],
  define: {
    __DEV__: "true",
    "process.env.NODE_ENV": '"development"',
    __FEATURE_SUSPENSE__: "true",
    __COMPAT__: "false",
    __BROWSER__: "false",
    __ESM_BUNDLER__: "true",
    __GLOBAL__: "false",
    __NODE_JS__: "false",
    __SSR__: "false",
    __TEST__: "false",
    __VERSION__: '"3.5"',
  },
});

if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
await Bun.write(outfile, await result.outputs[0].text());
console.log(`bundled ${entry} -> ${outfile} (${(await Bun.file(outfile).size) ?? "?"} bytes)`);
