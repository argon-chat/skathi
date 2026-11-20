// Reads one of the runtime's TypeScript sources and hands back plain JS.
//
// The harnesses in this directory read a src/ file as text, strip its import/export syntax and evaluate the rest with
// every ambient name bound as a parameter, so each world is private. `new Function` cannot parse TypeScript, so the
// text goes through `Bun.Transpiler`, the same parser and emitter `Bun.build` uses for dist/vendor.js: types are
// erased, values and comments survive.
//
// `useDefineForClassFields: false` mirrors tsconfig.json: a bare `x: T` field must be erased, not turned into a
// constructor-prologue define that runs after `super()`.

import { readFileSync } from "node:fs";

const transpiler = new Bun.Transpiler({
  loader: "ts",
  target: "browser",
  tsconfig: JSON.stringify({ compilerOptions: { target: "esnext", useDefineForClassFields: false } }),
});

/** One src/*.ts file, as the JavaScript the bundler would emit for it. */
export function readTsSource(path: string): string {
  return transpiler.transformSync(readFileSync(path, "utf8"));
}

/** The modules a harness hands the runtime as parameters, so they stay imports in {@link bundleRuntime}'s output. */
const INJECTED =
  /^(@vue\/runtime-core|(?:\.\.?\/)+(apphost|appapi|compat|canvas|iframe|devtools|selection|waapi|skwindow|skwindow-vue|tray|hotkeys|haptics|locale))$/;

/** src/runtime.ts with its src/runtime/ modules inlined, as one script; {@link INJECTED} imports are left in. */
export async function bundleRuntime(entry: string): Promise<string> {
  const out = await Bun.build({
    entrypoints: [entry],
    format: "esm",
    target: "browser",
    minify: false,
    plugins: [
      {
        name: "injected",
        setup(build) {
          build.onResolve({ filter: INJECTED }, (a) => ({
            path: a.path.replace(/^(?:\.\.?\/)+/, "./"),
            external: true,
          }));
        },
      },
    ],
  });
  if (!out.success) throw new AggregateError(out.logs, `bundling ${entry} failed`);
  // A name imported from one external by several modules is renamed in all but the first (`warnCompat2`); the
  // harness binds the plain name, so each rename becomes an alias of it.
  return (await out.outputs[0]!.text()).replace(
    /^import\s*\{([^}]*)\}\s*from\s*("[^"]+");?/gm,
    (line, list: string, from: string) => {
      const aliases = list
        .split(",")
        .map((s) => s.trim().split(/\s+as\s+/))
        .filter((p): p is [string, string] => p.length === 2 && p[0] !== p[1]);
      if (!aliases.length) return line;
      const kept = list
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s && !/\sas\s/.test(s));
      return [
        kept.length ? `import { ${kept.join(", ")} } from ${from};` : "",
        ...aliases.map(([name, alias]) => `const ${alias} = ${name};`),
      ]
        .filter(Boolean)
        .join("\n");
    },
  );
}
