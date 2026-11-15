// Svelte 5 components (`.svelte`) and rune modules (`.svelte.ts`, `.svelte.js`), compiled with the app's own
// `svelte/compiler`: the compiled code calls `svelte/internal/client`, which must be the same version.

import { pathToFileURL } from "node:url";
import type * as SvelteCompiler from "svelte/compiler";

type Compiler = typeof SvelteCompiler;

/** Files the Svelte compiler owns, components and rune modules both. */
export const SVELTE_SOURCE = /\.svelte(?:\.[jt]s)?$/;
const RUNE_MODULE = /\.svelte\.[jt]s$/;

const compilers = new Map<string, Promise<Compiler>>();

/** `svelte/compiler` as the app at `appRoot` resolves it. */
export function svelteCompiler(appRoot: string): Promise<Compiler> {
  let compiler = compilers.get(appRoot);
  if (!compiler) {
    compiler = (async () => {
      let path: string;
      try {
        path = Bun.resolveSync("svelte/compiler", appRoot);
      } catch {
        throw new Error(
          `${appRoot}: a .svelte file needs Svelte 5 installed in the app: add "svelte" to its dependencies`,
        );
      }
      const mod = (await import(pathToFileURL(path).href)) as Compiler;
      if (!/^5\./.test(mod.VERSION)) {
        throw new Error(`${appRoot}: svelte ${mod.VERSION} is installed; Skathi compiles Svelte 5 components`);
      }
      return mod;
    })();
    compilers.set(appRoot, compiler);
  }
  return compiler;
}

export interface SvelteOutput {
  js: string;
  css: string | null;
  warnings: string[];
}

const stripTypes = new Bun.Transpiler({ loader: "ts", trimUnusedImports: false });

/**
 * Compiles one Svelte source. `filename` names it in dev diagnostics and seeds the scoped-CSS hash, so it is the
 * module id, stable across machines. `dev` adds Svelte's runtime checks and the `import.meta.hot` epilogue.
 */
export async function compileSvelte(
  appRoot: string,
  absPath: string,
  source: string,
  o: { dev: boolean; filename: string },
): Promise<SvelteOutput> {
  const svelte = await svelteCompiler(appRoot);
  const describe = (w: SvelteCompiler.Warning) =>
    `${o.filename}${w.start ? `:${w.start.line}:${w.start.column + 1}` : ""}: ${w.message.split("\n")[0]} (${w.code})`;
  if (RUNE_MODULE.test(absPath)) {
    const js = absPath.endsWith(".ts") ? stripTypes.transformSync(source) : source;
    const out = svelte.compileModule(js, { filename: o.filename, generate: "client", dev: o.dev });
    return { js: out.js.code, css: null, warnings: out.warnings.map(describe) };
  }
  const out = svelte.compile(source, {
    filename: o.filename,
    generate: "client",
    dev: o.dev,
    hmr: o.dev,
    // The builders route component CSS into the app stylesheet, as they do an SFC's <style>.
    css: "external",
    // svelte-dom-shim: fragments: "tree" builds templates with createElement, since Skathi's DOM has no HTML parser for the default <template> innerHTML; goes with the custom renderer (sveltejs/svelte#18042)
    fragments: "tree",
  });
  return { js: out.js.code, css: out.css?.code ?? null, warnings: out.warnings.map(describe) };
}
