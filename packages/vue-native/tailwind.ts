// Tailwind CSS integration, shared by build.ts (dev) and prod.ts.
//
// Pinned to v3.4 (package.json): v4 emits `calc(var(--spacing) * 4)`, `color-mix`, `@property`,
// `@layer` and `@theme`, which vn-css cannot resolve completely yet; v3.4 emits literal
// declarations that render.
//
// We ask for `@tailwind base; @tailwind utilities`, not `components`. `base` carries the
// `*, ::before, ::after { --tw-*: … }` seeds that make compound utilities (shadows, rings,
// transforms, `space-*`) resolve; its ::before/::after halves are dropped with one advisory.

import { existsSync } from "fs";
import { dirname, join, resolve as resolvePath } from "path";

import postcss from "postcss";
import tailwindcss from "tailwindcss";

/// The synthetic module id the utilities sheet rides on. A leading "." sorts first, so utilities
/// land before component <style> and lose specificity ties, as in a browser project.
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

/// Runs Tailwind's JIT over the config's `content` globs and returns the utilities sheet.
///
/// Runs with the config's directory as cwd (Tailwind v3 resolves a relative `content` glob against
/// `process.cwd()`); calls are serialized because cwd is process-global.
export function generate(configPath: string, source?: CssSource): Promise<string> {
  const run = async () => {
    const dir = dirname(configPath);
    const prev = process.cwd();
    const move = resolvePath(prev).toLowerCase() !== resolvePath(dir).toLowerCase();
    if (move) process.chdir(dir);
    try {
      const result = await postcss([tailwindcss(configPath)]).process(
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

/// A stylesheet the app owns, handed to Tailwind as its own entry so `@tailwind` and `@apply` expand in place.
export type CssSource = { text: string; from: string };

/// Does this stylesheet need Tailwind's PostCSS pass (`@tailwind` / `@layer` / `@apply` / `theme()`)? A plain sheet is emitted verbatim.
export function needsTailwindPass(css: string): boolean {
  return /@tailwind\b|@layer\b|@apply\b|@screen\b|(?<![\w-])theme\(/.test(css);
}

/// Does this stylesheet declare the utilities layer itself? Then the synthetic `.vn/tailwind.css` module is redundant.
export function declaresUtilities(css: string): boolean {
  return /@tailwind\s+utilities\s*;/.test(css);
}
