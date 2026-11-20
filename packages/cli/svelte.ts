// Svelte 5 components (`.svelte`) and rune modules (`.svelte.ts`, `.svelte.js`), compiled with the app's own
// `svelte/compiler`: the compiled code calls `svelte/internal/client`, which must be the same version.

import { createRequire } from "node:module";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type * as SvelteCompiler from "svelte/compiler";
import { protectHapticPatterns } from "./haptic-patterns";

type Compiler = typeof SvelteCompiler;

/** Files the Svelte compiler owns, components and rune modules both. */
export const SVELTE_SOURCE = /\.svelte(?:\.[jt]s)?$/;
const RUNE_MODULE = /\.svelte\.[jt]s$/;
export const SVELTE_RENDERER = "@skathi-native/runtime/svelte-renderer";

export class SvelteRendererError extends Error {
  constructor(
    readonly code: "SK5015" | "SK5016",
    readonly detail: string,
    readonly file: string,
    readonly line = 1,
    readonly column = 0,
  ) {
    super(`${file}:${line}:${column + 1}: ${code} ${detail}`);
  }
}

const compilers = new Map<string, Promise<Compiler>>();

/** `svelte/compiler` as the app at `appRoot` resolves it. */
export function svelteCompiler(appRoot: string): Promise<Compiler> {
  let compiler = compilers.get(appRoot);
  if (!compiler) {
    compiler = (async () => {
      let path: string;
      try {
        path = createRequire(resolve(appRoot, "package.json")).resolve("svelte/compiler");
      } catch {
        throw new Error(
          `${appRoot}: a .svelte file needs Svelte 5 installed in the app: add "svelte" to its dependencies`,
        );
      }
      const module = (await import(pathToFileURL(path).href)) as Compiler & { default?: Compiler };
      return module.default ?? module;
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

const bunTranspiler =
  typeof Bun === "undefined" ? undefined : new Bun.Transpiler({ loader: "ts", trimUnusedImports: false });

async function stripTypes(source: string, filename: string): Promise<string> {
  if (bunTranspiler) return bunTranspiler.transformSync(source);
  const ts = (await import("typescript")).default;
  return ts.transpileModule(source, {
    fileName: filename,
    compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext, verbatimModuleSyntax: true },
  }).outputText;
}

/**
 * Compiles one Svelte source. `filename` names it in dev diagnostics and seeds the scoped-CSS hash, so it is the
 * module id, stable across machines. `dev` adds Svelte's runtime checks and the `import.meta.hot` epilogue.
 */
export async function compileSvelte(
  appRoot: string,
  absPath: string,
  source: string,
  o: { dev: boolean; filename: string; native?: boolean },
): Promise<SvelteOutput> {
  const svelte = await svelteCompiler(appRoot);
  const stableMinor = /^5\.(\d+)\.\d+(?:\+[\w.-]+)?$/.exec(svelte.VERSION)?.[1];
  if (o.native && (stableMinor === undefined || Number(stableMinor) < 59)) {
    throw new SvelteRendererError(
      "SK5016",
      `Native rendering needs Svelte 5.59.0 or newer in major 5 (found ${svelte.VERSION}). Install svelte@^5.59.0 or set svelte.renderer to "dom".`,
      absPath,
    );
  }
  if (!/^5\./.test(svelte.VERSION)) {
    throw new Error(`${appRoot}: svelte ${svelte.VERSION} is installed; Skathi compiles Svelte 5 components`);
  }
  const describe = (w: SvelteCompiler.Warning) =>
    `${o.filename}${w.start ? `:${w.start.line}:${w.start.column + 1}` : ""}: ${w.message.split("\n")[0]} (${w.code})`;
  if (RUNE_MODULE.test(absPath)) {
    const js = absPath.endsWith(".ts") ? await stripTypes(source, o.filename) : source;
    const out = svelte.compileModule(js, { filename: o.filename, generate: "client", dev: o.dev });
    return { js: out.js.code, css: null, warnings: out.warnings.map(describe) };
  }
  let restore = (css: string) => css;
  if (/@haptic-pattern\b/i.test(source)) {
    const prepared = await svelte.preprocess(
      source,
      {
        style({ content }) {
          const patterns = protectHapticPatterns(content);
          restore = patterns.restore;
          return { code: patterns.css };
        },
      },
      { filename: o.filename },
    );
    source = prepared.code;
  }
  let out: SvelteCompiler.CompileResult;
  try {
    out = svelte.compile(source, {
      filename: o.filename,
      generate: "client",
      dev: o.dev,
      hmr: o.dev,
      css: "external",
      // svelte-dom-shim: DOM components, including DOM guests in native apps, need templates without an HTML parser.
      fragments: "tree",
      ...(o.native && { experimental: { customRenderer: SVELTE_RENDERER } }),
    });
  } catch (e) {
    const err = e as SvelteCompiler.CompileError;
    if (!o.native || err?.code !== "incompatible_with_custom_renderer") throw e;
    throw new SvelteRendererError(
      "SK5015",
      `${err.message.split("\n")[0]}. Set svelte.renderer to "dom" to use this feature.`,
      absPath,
      err.start?.line,
      err.start?.column,
    );
  }
  return { js: out.js.code, css: out.css ? restore(out.css.code) : null, warnings: out.warnings.map(describe) };
}
