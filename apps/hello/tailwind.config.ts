// Tailwind config for the `hello` demo app.
//
// PINNED TO TAILWIND v3.4 — see packages/vue-native/tailwind.ts for the full
// rationale. Short version: v3 core utilities emit LITERAL declarations
// (`padding: 1rem`, `background-color: rgb(255 255 255 / 0.1)`), which vn-css's
// mapper renders directly. v4 routes almost everything through `var(--tw-*)`
// + `calc()` + `@property`, none of which the engine resolves yet.
//
// ESM + TypeScript, loaded by Tailwind v3's own `loadConfig` — which tries
// `require(path)` first (bun resolves and transpiles a `.ts` natively) and
// falls back to jiti, and then takes `config.default ?? config`. Either route
// gives CommonJS semantics, which is why `__dirname` below still resolves.
// `packages/vue-native/tailwind.ts`'s `CONFIG_NAMES` already looks for
// `tailwind.config.ts`, so nothing on the builder side changes.
//
// NOT type-checked by apps/tsconfig.json, and deliberately: this is a
// BUILD-TIME Node module, not app code, and putting it in scope would mean
// putting Node's globals in every component's scope. See the note at the foot
// of that file.

import { join } from "node:path";

export default {
  // Equivalent to `apps/hello/src/**/*.{vue,ts}` from the workspace root, but
  // anchored to THIS file so it does not depend on the builder's cwd (the dev
  // builder runs from packages/vue-native, prod from wherever it is invoked).
  //
  // `.ts`, not `.js`: the app's own sources are TypeScript now. Same set of
  // files as before the conversion — they were renamed, not added — so the
  // utilities sheet Tailwind extracts from them is byte-for-byte unchanged.
  content: [join(__dirname, "src/**/*.{vue,ts}")],

  corePlugins: {
    // The host has no user-agent stylesheet to reset: there is no <body>, no
    // <h1> default margin, no <button> appearance. Preflight would only spray
    // element and pseudo-element selectors (`*, ::before, ::after`, `html`,
    // `abbr:where(...)`) that vn-css cannot match — pure diagnostic noise.
    preflight: false,

    // THE LOAD-BEARING ONE. With these on, EVERY plain color utility is
    // var()-composed and renders as nothing:
    //
    //   bg-white  ->  --tw-bg-opacity: 1;
    //                 background-color: rgb(255 255 255 / var(--tw-bg-opacity, 1))
    //
    // Off, Tailwind's own `if (!corePlugins('backgroundOpacity'))` branch emits
    // the literal instead — `background-color: #fff` — which vn-css renders:
    //
    //   bg-white  ->  background-color: #fff
    //
    // The modern slash syntax is UNAFFECTED either way (`bg-white/10` is parsed
    // into a color with alpha before the plugin ever runs, and always emits
    // `rgb(255 255 255 / 0.1)`), so nothing about how you write opacity in a
    // template changes.
    //
    // DOCUMENTED DIVERGENCE: the LEGACY `bg-opacity-50` / `text-opacity-*` /
    // `border-opacity-*` / `ring-opacity-*` / `divide-opacity-*` /
    // `placeholder-opacity-*` classes are no longer generated. Tailwind has
    // deprecated all of them in favour of the slash syntax since v3.0 — write
    // `bg-white/50`. Because an unknown class simply produces no rule, there is
    // nothing for vn-css to warn about: this comment IS the advisory.
    backgroundOpacity: false,
    textOpacity: false,
    borderOpacity: false,
    divideOpacity: false,
    placeholderOpacity: false,
    ringOpacity: false,
  },

  // Stock theme — the point of this round is that STANDARD Tailwind works.
  theme: { extend: {} },
  plugins: [],
};
