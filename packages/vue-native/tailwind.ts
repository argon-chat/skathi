// Tailwind CSS integration for vue-native — shared by build.ts (dev) and
// prod.ts (single-IIFE), so the two builders can never drift on what utilities
// the app gets.
//
// ---------------------------------------------------------------------------
// WHY TAILWIND v3.4 (pinned in package.json as `tailwindcss@3.4`)
// ---------------------------------------------------------------------------
// vn-css maps `lightningcss::Property` values onto `gpui::StyleRefinement`.
// It resolves LITERAL declarations; it does not (yet) have a custom-property
// store, so `var()` never resolves and `calc()` is dropped.
//
//   v3.4 `p-4`      -> `padding: 1rem`                        rendered
//   v3.4 `bg-white/10` -> `background-color: rgb(255 255 255 / 0.1)`  rendered
//   v3.4 `rounded-xl`  -> `border-radius: 0.75rem`            rendered
//
//   v4   `p-4`      -> `padding: calc(var(--spacing) * 4)`    NOTHING renders
//   v4   `bg-white/10` -> `background-color: color-mix(in oklab, ...)`   ditto
//   v4   also emits `@property --tw-*`, `@layer`, `@theme` at-rules that our
//        compiler skips wholesale, and leans on `:where()` for its reset.
//
// So v4 would degrade essentially EVERY utility to a diagnostic. Moving to v4
// is a vn-css work item, not a version bump. Until then the pin is the
// compat-honest choice: standard Tailwind classes in templates, standard
// Tailwind semantics on screen.
//
// WHAT v4 WOULD NEED FROM vn-css (scoped, in dependency order):
//   1. A custom-property store on the cascade. `--*` declarations currently hit
//      one advisory and vanish; they need to fold per element like any other
//      declaration, INHERITING down the tree (that is the part that makes it a
//      cascade change rather than a mapper change — `element.rs` would have to
//      expose the parent's resolved map).
//   2. `var()` substitution before mapping. lightningcss hands us these as
//      `Property::Unparsed(UnparsedProperty { property_id, value: TokenList })`;
//      substitute from (1), then re-parse the token list with `property_id` to
//      get back a typed `Property` the existing mapper arms already handle.
//   3. `calc()` evaluation for the numeric cases — `Length::Calc` /
//      `DimensionPercentage::Calc` are dropped by the length converters today.
//      v4's spacing scale is `calc(var(--spacing) * 4)`, so (1)+(2)+(3) together
//      are the price of admission for `p-4`.
//   4. `@property` initial values, so an unset `--tw-*` falls back correctly
//      instead of resolving to nothing.
//   5. `color-mix(in oklab, …)` — v4's opacity modifiers — plus the oklab/oklch
//      color spaces in the `color()` converter.
//   6. `@layer` handling in `Engine::compile` (currently every at-rule is
//      dropped, so a v4 sheet would compile to zero rules).
// (1)-(3) are the load-bearing majority; (4)-(6) are what makes it correct
// rather than approximate.
//
// ---------------------------------------------------------------------------
// WHAT WE ASK TAILWIND FOR
// ---------------------------------------------------------------------------
// `@tailwind base; @tailwind utilities;` — deliberately NOT `components`
// (which is just the `container` plugin, media-query-only).
//
// `base` was excluded until compat round 6, on the grounds that it is preflight
// (disabled in every vue-native config anyway) plus the
// `*, ::before, ::after { --tw-*: … }` universal-defaults block, and "the
// `--tw-*` seeds go nowhere". They go somewhere NOW: vn-css stores custom
// properties and substitutes `var()`, and those seeds are what makes
// `--tw-ring-inset`, `--tw-shadow`, `--tw-translate-x` &c resolve at all.
// Without them every compound utility — shadows, rings, transforms, `space-*`,
// `divide-*` — is invalid at computed-value time and drops, exactly as it would
// in a browser served the same sheet.
//
// The `::before` / `::after` halves of that selector list are dropped by the
// selector audit with one advisory (no pseudo-element support); the `*` half
// compiles and is what carries the seeds.
//
// JIT means the sheet contains ONLY the utilities the app's templates actually
// use, so the diagnostics an app sees are about ITS OWN classes.

import { existsSync } from "fs";
import { dirname, join } from "path";

import postcss from "postcss";
import tailwindcss from "tailwindcss";

/// The synthetic module id the utilities sheet rides on.
///
/// Module ids are workspace-relative POSIX paths, and BOTH aggregators order
/// CSS by id: the dev loader appends in first-`setCss` order (= the manifest's
/// module order, which build.ts emits sorted) and prod.ts sorts `cssById`.
/// A leading "." (0x2E) sorts below every letter and digit, so this id lands
/// FIRST in both — which is exactly where utilities belong:
///
///   utilities first, component <style> last
///
/// Tailwind is a flat single-class layer (specificity 0-1-0) while component
/// styles are almost always `<style scoped>` (`.foo[data-v-x]`, 0-2-0), so the
/// cascade already prefers the component. Source order only decides ties
/// between two equally-specific rules — and there the author's own <style>
/// should win over a framework utility, same as a browser project that imports
/// tailwind at the top of its entry stylesheet.
export const TW_ID = ".vn/tailwind.css";

const CONFIG_NAMES = ["tailwind.config.js", "tailwind.config.cjs", "tailwind.config.mjs", "tailwind.config.ts"];

/// Nearest `tailwind.config.*` at or above `startDir`, stopping after
/// `stopDir` (the workspace root). `null` = this app opts out of Tailwind, and
/// the builders then emit no synthetic module at all — which is what keeps the
/// build fixtures (and any non-Tailwind app) byte-identical to before.
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

/// Runs Tailwind's JIT over the config's `content` globs and returns the
/// utilities sheet. Re-reads the content files on every call (Tailwind's own
/// mtime tracking decides whether to rebuild its context), so watch mode can
/// simply call this again after any template edit.
export async function generate(configPath: string): Promise<string> {
  const result = await postcss([tailwindcss(configPath) as any]).process(
    "@tailwind base;\n@tailwind utilities;\n",
    { from: undefined },
  );
  return result.css;
}
