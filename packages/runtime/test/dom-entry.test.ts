import { expect, test } from "bun:test";
import { join } from "node:path";

const SRC = join(import.meta.dir, "../src");

test("an app without Vue gets a runtime that imports nothing of Vue", async () => {
  const vue: string[] = [];
  const out = await Bun.build({
    entrypoints: [join(SRC, "dom.ts"), join(SRC, "svelte.ts")],
    target: "browser",
    plugins: [
      {
        name: "no-vue",
        setup(build) {
          build.onResolve({ filter: /^@skathi-native\/runtime$/ }, () => ({ path: join(SRC, "dom.ts") }));
          build.onResolve({ filter: /^(svelte(\/.*)?|.*\.svelte)$/ }, (args) => ({ path: args.path, external: true }));
          build.onResolve({ filter: /^(vue|@vue\/.*)$/ }, (args) => {
            vue.push(`${args.importer}: ${args.path}`);
            return { path: args.path, external: true };
          });
        },
      },
    ],
  });
  expect(out.success).toBe(true);
  expect(vue).toEqual([]);
});
