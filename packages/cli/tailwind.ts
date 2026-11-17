// Tailwind CSS integration, shared by build.ts (dev) and prod.ts. The app installs tailwindcss; it is loaded from
// next to its config only when there is one.
//
// Tailwind 3 only: v4 emits `calc(var(--spacing) * 4)`, `color-mix`, `@property`, `@layer` and `@theme`, which
// sk-css cannot resolve completely yet; v3 emits literal declarations that render.
//
// We ask for `@tailwind base; @tailwind utilities`, not `components`. `base` carries the
// `*, ::before, ::after { --tw-*: … }` seeds that make compound utilities (shadows, rings,
// transforms, `space-*`) resolve; its ::before/::after halves are dropped with one advisory.

import { existsSync, readFileSync } from "fs";
import { createRequire } from "module";
import { basename, dirname, join, resolve as resolvePath } from "path";

import postcss from "postcss";
import type TailwindPlugin from "tailwindcss";
import { packageDir } from "./runtime-path";

const INSTALL = "bun add -d tailwindcss@3";

/** Why the app with `configPath` cannot run Tailwind, or null when its tailwindcss is a 3.x. */
export function tailwindProblem(configPath: string): string | null {
  const dir = packageDir("tailwindcss", dirname(configPath));
  if (!dir) return `this app has ${basename(configPath)} but "tailwindcss" is not installed: ${INSTALL}`;
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as { version?: unknown };
  const version = String(pkg.version ?? "");
  if (!version.startsWith("3.")) {
    return (
      `this app has tailwindcss ${version}, and Skathi supports Tailwind CSS 3 only (the renderer does not resolve ` +
      `v4's var()-based utilities, color-mix() and @property yet): ${INSTALL}`
    );
  }
  return null;
}

const plugins = new Map<string, typeof TailwindPlugin>();

function tailwindFor(configPath: string): typeof TailwindPlugin {
  const known = plugins.get(configPath);
  if (known) return known;
  const problem = tailwindProblem(configPath);
  if (problem) throw new Error(problem);
  const dir = packageDir("tailwindcss", dirname(configPath))!;
  const plugin = createRequire(join(dir, "package.json"))("./") as typeof TailwindPlugin;
  plugins.set(configPath, plugin);
  return plugin;
}

/// The synthetic module id the utilities sheet rides on. A leading "." sorts first, so utilities
/// land before component <style> and lose specificity ties, as in a browser project.
export const TW_ID = ".sk/tailwind.css";

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
      const result = await postcss([tailwindFor(configPath)(configPath)]).process(
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

/// Does this stylesheet declare the utilities layer itself? Then the synthetic `.sk/tailwind.css` module is redundant.
export function declaresUtilities(css: string): boolean {
  return /@tailwind\s+utilities\s*;/.test(css);
}
