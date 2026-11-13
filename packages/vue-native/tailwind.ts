// Tailwind CSS integration, shared by build.ts (dev) and prod.ts so both builders give an app the same utilities.
//
// Pinned to v3.4 (package.json): v4 emits `calc(var(--spacing) * 4)`, `color-mix(in oklab, ...)`,
// `@property`, `@layer` and `@theme`, which vn-css cannot resolve completely yet, while v3.4 emits
// literal declarations that render. Moving to v4 is a vn-css work item (custom-property inheritance,
// var() substitution, calc(), @property initial values, color-mix/oklab, @layer in Engine::compile).
//
// We ask for `@tailwind base; @tailwind utilities`, not `components` (only the media-query-only
// `container` plugin). `base` carries the `*, ::before, ::after { --tw-*: … }` seeds that make compound
// utilities (shadows, rings, transforms, `space-*`, `divide-*`) resolve; its ::before/::after halves are
// dropped with one advisory and the `*` half compiles. JIT means the sheet holds only the utilities the
// app's templates use.

import { existsSync } from "fs";
import { dirname, join, resolve as resolvePath } from "path";

import postcss from "postcss";
import tailwindcss from "tailwindcss";

/// The synthetic module id the utilities sheet rides on. Both aggregators order CSS by id and a leading
/// "." sorts first, so utilities land before component <style>, which wins ties on specificity as in a
/// browser project that imports tailwind at the top of its entry stylesheet.
export const TW_ID = ".vn/tailwind.css";

const CONFIG_NAMES = ["tailwind.config.js", "tailwind.config.cjs", "tailwind.config.mjs", "tailwind.config.ts"];

/// Nearest `tailwind.config.*` at or above `startDir`, stopping after `stopDir` (the workspace root).
/// `null`: the app opts out and no synthetic module is emitted.
export function findConfig(startDir: string, stopDir: string): string | null {
  let dir = startDir;
  for (;;) {
    for (const name of CONFIG_NAMES) {
      const candidate = join(dir, name);
      if (existsSync(candidate)) return candidate;
    }
    if (dir === stopDir) return null;
    const parent = dirname(dir);
    if (parent === dir) return null; // filesystem root — startDir was outside stopDir
    dir = parent;
  }
}

/// Runs Tailwind's JIT over the config's `content` globs and returns the utilities sheet; call it again
/// after any template edit.
///
/// Runs with the config's directory as cwd: Tailwind v3 resolves a relative `content` glob against
/// `process.cwd()`, which under Vite is the project root but here would be packages/vue-native. Calls are
/// serialized because cwd is process-global and Tailwind reads it inside the async plugin.
export function generate(configPath: string, source?: CssSource): Promise<string> {
  const run = async () => {
    const dir = dirname(configPath);
    const prev = process.cwd();
    const move = resolvePath(prev).toLowerCase() !== resolvePath(dir).toLowerCase();
    if (move) process.chdir(dir);
    try {
      const result = await postcss([tailwindcss(configPath) as any]).process(
        source?.text ?? "@tailwind base;\n@tailwind utilities;\n",
        { from: source?.from },
      );
      return result.css;
    } finally {
      if (move) process.chdir(prev);
    }
  };
  const next = queue.then(run, run);
  queue = next.catch(() => undefined);
  return next;
}
let queue: Promise<unknown> = Promise.resolve();

/// A stylesheet the app owns, handed to Tailwind as its own entry so its `@tailwind`, `@layer base` and
/// `@apply` expand in place, as in a Vite project.
export type CssSource = { text: string; from: string };

/// Does this stylesheet contain `@tailwind` / `@layer` / `@apply` / `theme()`, i.e. need Tailwind's PostCSS
/// pass? A plain sheet is emitted verbatim.
export function needsTailwindPass(css: string): boolean {
  return /@tailwind\b|@layer\b|@apply\b|@screen\b|(?<![\w-])theme\(/.test(css);
}

/// Does this stylesheet declare the utilities layer itself? Then the synthetic `.vn/tailwind.css` module is
/// redundant.
export function declaresUtilities(css: string): boolean {
  return /@tailwind\s+utilities\s*;/.test(css);
}
