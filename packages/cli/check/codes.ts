import type { Severity } from "./schema";

export interface CodeInfo {
  code: string;
  severity: Severity;
  title: string;
  explanation: string;
  example: string;
  fix: string;
  /** The check could not run as asked: exit code 2. */
  fatal?: boolean;
  /** `check.allow` cannot hide it. */
  firm?: boolean;
}

export const RANGES = [
  { from: 0, to: 999, what: "the check itself" },
  { from: 1000, to: 1999, what: "breaks under AOT" },
  { from: 2000, to: 2999, what: "works under AOT, but slower" },
  { from: 3000, to: 3999, what: "typing quality: compiles, but AOT gets no type information" },
  { from: 4000, to: 4999, what: "CSS support and rendering compatibility" },
  { from: 5000, to: 5999, what: "Vue and Svelte features that do not work, or work differently, in Skathi" },
  { from: 6000, to: 6999, what: "permissions and modules that reach outside the app" },
] as const;

const CODES: CodeInfo[] = [
  {
    code: "SK0001",
    severity: "error",
    title: "a file of the bundle cannot be read",
    explanation:
      "The production build put this file into the bundle, but the check could not read it, so nothing in it was checked.",
    example: "a module deleted or locked between the build and the check",
    fix: "Make sure the file exists and is readable, then run sk check again.",
  },
  {
    code: "SK0002",
    severity: "warning",
    title: "a file of the bundle does not parse",
    explanation:
      "The TypeScript parser reports a syntax error in a file the bundler accepted. The file is skipped, so its findings are missing from the report.",
    example: "const x = ;",
    fix: "Fix the syntax error; if the file is valid for the bundler, report it as a bug in sk check.",
  },
  {
    code: "SK0003",
    severity: "error",
    title: "the check configuration is invalid",
    explanation:
      'sk.config.json "check" or the app\'s tsconfig cannot be used as written, for example an unknown code in "check.allow". The check stops with exit code 2 so a typo cannot silently allow everything.',
    example: '"check": { "allow": { "SK9999": ["some-package"] } }',
    fix: 'Use codes from `sk explain --list`; "allow" maps a code to package names or app-relative globs.',
    fatal: true,
  },
  {
    code: "SK0004",
    severity: "error",
    title: "the production build fails, so the bundle's files are unknown",
    explanation:
      "sk check checks exactly the files the production build puts into the bundle. When that build fails, for example on an import that does not resolve, there is no bundle to check.",
    example: 'import { thing } from "a-package-that-is-not-installed";',
    fix: "Fix the build error (sk build shows the same one), then run sk check again.",
    fatal: true,
  },
  {
    code: "SK0005",
    severity: "error",
    title: "an effect shader does not compile",
    explanation:
      "The app's shaders/*.slang are its own `--shading` effects, compiled into the app for every OS by sk build. One of them does not compile, or breaks a rule of the effect schema (an attribute, a parameter type, a name), so sk build refuses the app.",
    example:
      '[SkEffect("ripple")]\nstruct Ripple : IEffect\n{\n    int rings; // parameters are float, float2, float3 or float4\n    float4 shade(EffectInput input) { return float4(0, 0, 0, 0); }\n};',
    fix: "Fix the shader at the reported line; the rules are in the comments of the `sk` prelude (`import sk;`).",
  },
  {
    code: "SK0006",
    severity: "warning",
    title: "the effect shaders could not be checked",
    explanation:
      "The app has shaders/*.slang, but the shader compiler (redslangc) could not be found or downloaded, so they were not compiled. sk build needs it too.",
    example: "SK_HOST_OFFLINE=1 with no cached redslangc",
    fix: "Allow the download once, or set SK_REDSLANGC to a redslangc binary.",
  },
  {
    code: "SK1001",
    severity: "error",
    title: "a built-in method or property is replaced or deleted",
    explanation:
      "In an AOT image the engine's built-in objects are read-only: replacing or deleting a member they already have throws a TypeError at run time (the build rejects it outright when the assignment is unconditional), even though the property descriptor still reports it as writable. Adding a new member works, except on Object.prototype, Promise.prototype and RegExp.prototype, which take no new members. The methods of `console` are read-only too: a logger that wraps `console.error` throws at its first line. A key that is one of a few literals (`console[level] = …` over a list of names) is checked for each of them. Assignments guarded by a check for the member (polyfills such as `if (!Array.prototype.at) …`), assignments under `if (!Runtime.isAot())` and assignments that put back the value read from the same property are not reported.",
    example: "Array.prototype.push = function (...items) {\n  return items.length;\n};",
    fix: "Leave the built-in alone: call a helper function, or wrap or subclass the object instead of patching it.",
  },
  {
    code: "SK1002",
    severity: "error",
    title: "a built-in is redefined through Object.defineProperty, Object.assign or Reflect",
    explanation:
      "Redefining an existing member of a built-in prototype or constructor through Object.defineProperty, Object.defineProperties or Object.assign throws a TypeError in an AOT image, as a plain assignment does. Reflect.set and Reflect.defineProperty do not throw: they return false and leave the built-in unchanged, so code that relies on the patch silently misbehaves.",
    example: 'Object.defineProperty(Array.prototype, "includes", { value: () => false });',
    fix: "Do not redefine built-ins; use a helper function or your own subclass.",
  },
  {
    code: "SK1003",
    severity: "error",
    title: "the prototype of a built-in is changed",
    explanation:
      "Changing what a built-in prototype or constructor inherits from (Object.setPrototypeOf, Reflect.setPrototypeOf or a `__proto__` assignment) throws a TypeError in an AOT image; Reflect.setPrototypeOf returns false instead.",
    example: "Object.setPrototypeOf(Array.prototype, Object.create(null));",
    fix: "Build your own class hierarchy (class MyList extends Array) instead of rewiring the built-in one.",
  },
  {
    code: "SK1004",
    severity: "error",
    title: "a global built-in is replaced or deleted",
    explanation:
      "Replacing or deleting a global the language itself relies on (Array, Map, Object, Math, JSON, Symbol, …) throws a TypeError in an AOT image. Some globals stay replaceable (Promise, Date, Error, console and others the engine marks so); those are not reported.",
    example: "globalThis.Map = class MyMap<K, V> extends Map<K, V> {};",
    fix: "Import your implementation under its own name instead of installing it over the global.",
  },
  {
    code: "SK2001",
    severity: "warning",
    title: "eval runs interpreted",
    explanation:
      "eval still works in an AOT image, but an AOT image has no JIT: the evaluated code runs in the interpreter, and it cannot be optimised with the rest of the program. Direct and indirect eval (`(0, eval)(…)`, `globalThis.eval`) are both reported.",
    example: 'const n = Math.random();\nconst value = eval("1 + " + n);',
    fix: "Replace eval with ordinary code (a lookup table, JSON.parse, a small parser).",
  },
  {
    code: "SK2002",
    severity: "warning",
    title: "the Function constructor runs interpreted",
    explanation:
      "`new Function(…)` and `Function(…)` still work in an AOT image, but the generated function runs in the interpreter, without a JIT. Libraries that generate their hot code this way (validators, template engines, serialisers) lose most of their speed.",
    example: 'const add = new Function("a", "b", "return a + b");',
    fix: "Write the function in source, or switch the library to its non-generating mode (often called jitless or CSP-safe).",
  },
  {
    code: "SK2003",
    severity: "warning",
    title: "a timer is given a string of code",
    explanation:
      "In a browser, setTimeout, setInterval and setImmediate evaluate a string like eval, which an AOT image could only interpret. Skathi's timers (like Bun's) take functions only: the string never runs, and the timer reports a TypeError when it fires (Bun throws at the call). Code that relies on it is broken on every Skathi host, with or without AOT.",
    example: 'setTimeout("globalThis.ticked = true", 0);',
    fix: "Pass a function: setTimeout(tick, 100).",
  },
  {
    code: "SK2004",
    severity: "warning",
    title: "import() with a computed specifier",
    explanation:
      "The AOT build can only compile modules it can find at build time. With a computed specifier it compiles every file the expression might name and warns; at run time a path it did not compile fails to load.",
    example: 'const name = "home";\nconst page = await import("./pages/" + name + ".ts");',
    fix: 'Use string literals, for example a map of `() => import("./pages/home.ts")` loaders, or import.meta.glob.',
  },
  {
    code: "SK2005",
    severity: "warning",
    title: "a tight loop is bounded by a number that may not be an integer",
    explanation:
      "A JIT watches a loop run, sees that its bound is always a small integer and compiles the comparison for integers. An AOT image is compiled before the app runs and has nothing to watch: a parameter declared `number` may hold a fraction, so the image cannot compile the comparison for integers only. In a loop whose body is only arithmetic and element access, that comparison is much of the work: the measured loop ran three times slower than under the JIT, and as fast as the JIT with an integer bound. Reported for a `number` parameter that bounds a loop with no calls in its body, in TypeScript only.",
    example:
      "function sum(values: Int32Array, n: number): number {\n  let total = 0;\n  for (let i = 0; i < n; i++) total = (total + values[i]!) | 0;\n  return total;\n}",
    fix: "When the bound is always an integer below 2^31, take an integer copy before the loop and compare with that: `const end = n | 0; for (let i = 0; i < end; i++)`. A loop that runs a few times is not worth changing: allow the finding in check.allow.",
  },
  {
    code: "SK2006",
    severity: "warning",
    title: "a tight loop writes into a number[] by index",
    explanation:
      "An array can change the kind of values it holds, grow, and have holes, and each indexed store has to allow for that. A JIT learns from the run that a `number[]` stays dense and numeric and drops the checks; an AOT image keeps them. In the measured loop, an interpreter's registers read and written by index, the image was about 2.3 times slower with a `number[]` than with an Int32Array, and the JIT was not. Reported for a store through a computed index into a value typed `number[]`, inside a loop with no calls in its body, in TypeScript only.",
    example:
      "function scale(out: number[], by: number): void {\n  for (let i = 0; i < out.length; i++) out[i] = (i * by) | 0;\n}",
    fix: "Keep numbers a hot loop writes in a typed array of a fixed length: Int32Array, Uint8Array, Float64Array. A loop that runs a few times is not worth changing: allow the finding in check.allow.",
  },
  {
    code: "SK3001",
    severity: "warning",
    title: "a dependency ships no type declarations",
    explanation:
      "The AOT compiler uses types to lay out objects and specialise code. This package has no `types`/`typings` entry, no .d.ts beside its modules and no @types package installed, so all of it is compiled untyped, through the generic guarded paths.",
    example: "a JavaScript-only package such as an older jQuery plugin",
    fix: "Install its @types package, prefer a typed alternative, or add a .d.ts for the parts you use.",
  },
  {
    code: "SK3002",
    severity: "warning",
    title: "a dependency's declarations use `any` in what the app imports",
    explanation:
      "The declarations of what the app imports from this package type parameters, results or members as `any` (or as the catch-alls `Function`, `Object`, `{}` and `[key: string]: any`). Values that cross these declarations reach the AOT compiler without a type, so code using them takes the generic path. One diagnostic per package, from 15% of the declarations the app reaches (the typing summary lists every package); the worst are listed as related locations. `any` in type-level machinery (constraints, conditional checks, `keyof`, results of callbacks the app passes in, the plumbing of vue-tsc component declarations) is not counted.",
    example: "export declare function parse(input: any): any;",
    fix: "Upgrade to a better-typed version or alternative, or wrap the import in a typed module of your own.",
  },
  {
    code: "SK3003",
    severity: "warning",
    title: "the app's code uses `any` or suppresses type checking",
    explanation:
      "Explicit `any` (annotations, `as any`, `<any>`), parameters that are implicitly `any`, and `// @ts-nocheck`, `@ts-ignore` or `@ts-expect-error` give the AOT compiler no type information for the code involved. One diagnostic per file; the places are listed as related locations.",
    example: "export function total(items: any) { return items.reduce((n, i) => n + i.price, 0); }",
    fix: "Give the values real types; `unknown` plus narrowing where the type is genuinely open.",
  },
  {
    code: "SK3004",
    severity: "warning",
    title: "an ambient catch-all or untyped JavaScript makes imports `any`",
    explanation:
      'A shorthand ambient module (`declare module "x";`), a `declare module "*"`, a package whose "exports" lead TypeScript to its JavaScript instead of the declarations it ships, or an app JavaScript file without JSDoc types gives everything imported through it the type `any`.',
    example: 'declare module "some-package";',
    fix: "Remove the shorthand declaration (or give it a body with real types), and convert the JavaScript file to TypeScript or annotate it with JSDoc.",
  },
  {
    code: "SK4001",
    severity: "warning",
    title: "a CSS property Skathi does not support",
    explanation:
      "The native renderer does not implement this property. This includes declarations whose value comes from var(). Inactive @supports branches, CSS-wide keywords and inert defaults such as float: none are skipped.",
    example: ".card {\n  float: left;\n}",
    fix: "Use what the help line suggests (flex or grid instead of `float`), or remove the declaration. Keep it for a browser build inside `@supports (float: left) { … }`.",
  },
  {
    code: "SK4002",
    severity: "warning",
    title: "a CSS value Skathi does not apply",
    explanation:
      "The property is supported, but not with this value or function: for example the colour functions of `filter`, 3D functions of `transform`, or `width: fit-content`. The declaration is dropped and the app prints an advisory for it while it runs.",
    example: ".avatar {\n  filter: grayscale(1);\n}",
    fix: "Use a value the help line names, or remove the declaration. Keep it for a browser build inside `@supports`.",
  },
  {
    code: "SK4003",
    severity: "warning",
    title: "a CSS selector that never matches in Skathi",
    explanation:
      "The selector uses a pseudo-class or pseudo-element the engine cannot answer (`:visited`, `:placeholder-shown`, `::first-line`, `::-webkit-scrollbar`, a hover state inside `:not()`), so the whole rule is dropped.",
    example: "input:placeholder-shown {\n  opacity: 0.6;\n}",
    fix: "Select on a class or data attribute the app sets from script, or use the property the help line names.",
  },
  {
    code: "SK4004",
    severity: "warning",
    title: "a CSS at-rule Skathi does not support",
    explanation:
      "The native stylesheet collector ignores this at-rule or query. For @property only percentage syntax with a percentage initial value is supported. The diagnostic identifies the relevant rule.",
    example: "@scope (.card) {\n  img { border-radius: 4px; }\n}",
    fix: "Move the rules out of the block, or write the query with what the help line names.",
  },
  {
    code: "SK4005",
    severity: "warning",
    title: "CSS compatibility warnings in a dependency",
    explanation:
      "Stylesheets imported from the package have CSS support or compatibility warnings. Their source locations appear together in this diagnostic.",
    example: 'import "some-ui-kit/dist/style.css";',
    fix: "Check that the package's components the app uses look right; allow SK4005 for the package once they do.",
  },
  {
    code: "SK4006",
    severity: "warning",
    title: "CSS rendering depends on the surrounding layout or paint",
    explanation:
      "The native renderer can draw this pattern differently depending on the computed background, the SVG paint or the display of the parent and siblings. The warning describes the condition. Check the matched element to see whether it applies.",
    example: ".caption { filter: drop-shadow(0 1px 2px black); }",
    fix: "Use the alternative in the help line if the computed style meets the condition.",
  },
  {
    code: "SK4007",
    severity: "warning",
    title: "this CSS needs a Skathi 0.6 host",
    explanation:
      "This declaration or selector relies on rendering introduced in 0.6. Earlier hosts can drop it or draw a different result.",
    example: ".dialog { width: min(32rem, 100%); }",
    fix: 'Distribute a 0.6 or later host. An app whose package.json asks for "@skathi-native/runtime" 0.6 or later, such as "^0.7.0", is not warned.',
  },
  {
    code: "SK5001",
    severity: "error",
    title: "Vue Vapor mode",
    explanation:
      "Skathi renders Vue through runtime-core's virtual-DOM renderer and has no Vapor runtime. With Vue 3.5 the `vapor` attribute is ignored and the component is compiled as a regular one; with Vue 3.6 the compiled component imports Vapor's runtime, which Skathi's `vue` does not export, and the build fails. `createVaporApp`, `defineVaporComponent` and `vaporInteropPlugin` fail the build in either version.",
    example: '<script setup vapor lang="ts">\nconst count = ref(0);\n</script>',
    fix: "Drop `vapor` from the `<script>` tag and mount with `createApp()`: the same component runs on the virtual-DOM renderer.",
  },
  {
    code: "SK5002",
    severity: "error",
    title: "custom elements (web components)",
    explanation:
      "Skathi has no custom elements and no shadow DOM: `customElements.get()` answers undefined and `customElements.define()` throws NotSupportedError. Vue's `defineCustomElement()` returns a class that throws NotSupportedError when constructed, and `useHost()`/`useShadowRoot()` answer null. A Svelte component with `<svelte:options customElement>` compiles to a `customElements.define()` call, which throws when the module loads; `$host()` only compiles in such a component.",
    example:
      'import { defineCustomElement } from "vue";\nimport Counter from "./Counter.vue";\n\ncustomElements.define("my-counter", defineCustomElement(Counter));',
    fix: "Use the component directly (`<Counter />`), and pass what a host element would carry as props or with provide/inject (Vue) or context (Svelte).",
  },
  {
    code: "SK5003",
    severity: "error",
    title: "HTML strings: `v-html`, `innerHTML`, `{@html}`",
    explanation:
      "Skathi's DOM has no HTML parser. `v-html` and a bound `innerHTML` set an attribute instead of building elements, and Svelte's `{@html}` inserts nothing, so the element stays empty.",
    example: '<div v-html="article.body"></div>',
    fix: "Render the content as elements: a component per block, `h()` calls built from parsed data, or text with `{{ }}` / `{expression}` when it is plain text.",
  },
  {
    code: "SK5004",
    severity: "error",
    title: "CSS modules: `<style module>`, `useCssModule()`",
    explanation:
      "The builders do not compile `<style module>`: its rules are added unscoped, under their own class names, and `$style` is undefined, so a template that reads `$style.x` throws while rendering and the component renders nothing. `useCssModule()` answers `{}`.",
    example:
      '<template>\n  <p :class="$style.red">alert</p>\n</template>\n\n<style module>\n.red { color: red; }\n</style>',
    fix: 'Use `<style scoped>` and plain class names (`class="red"`); `v-bind()` in the styles covers values that come from script.',
  },
  {
    code: "SK5008",
    severity: "warning",
    title: "vue-router web and hash history",
    explanation:
      "A Skathi window has one document and no address bar: `history.pushState()` keeps the state but not the URL, and `history.back()`, `forward()` and `go()` do nothing. With `createWebHistory()` or `createWebHashHistory()`, `router.push()` works but `router.back()` and `router.go()` do not, and the app always starts at `/`.",
    example: "const router = createRouter({ history: createWebHistory(), routes });",
    fix: "Use `createMemoryHistory()`: it keeps its own stack, so back and forward work.",
  },
  {
    code: "SK5009",
    severity: "warning",
    title: "server-side rendering and hydration APIs",
    explanation:
      "There is no server-rendered markup in a Skathi window. Vue's `createSSRApp()` behaves as `createApp()`, and `hydrate()` (Vue's or Svelte's) renders the component from scratch, as `render()`/`mount()` do.",
    example: 'createSSRApp(App).mount("#app");',
    fix: "Call `createApp()` (Vue) or `mount()` from `@skathi-native/runtime/svelte` (Svelte).",
  },
  {
    code: "SK5010",
    severity: "warning",
    title: "`v-model.lazy`",
    explanation:
      "`.lazy` syncs on `change`, which text fields do not fire in Skathi. The model updates on every keystroke instead, as plain `v-model` does.",
    example: '<input v-model.lazy="name" />',
    fix: "Drop `.lazy`, or keep a draft value and commit it on `blur` or Enter (`@keydown.enter`).",
  },
  {
    code: "SK5011",
    severity: "warning",
    title: "`<style lang>` preprocessors",
    explanation:
      'The builders do not run Sass, Less or Stylus: a `<style lang="scss">` block is compiled as plain CSS, so variables, mixins and functions reach the engine as written and the declarations that use them are dropped.',
    example: '<style lang="scss">\n$accent: #0af;\n.title { color: $accent; }\n</style>',
    fix: "Write the block as plain CSS (custom properties and nesting cover most of what variables and nesting did), or compile the preprocessor's output to a .css file and import that.",
  },
  {
    code: "SK5012",
    severity: "warning",
    title: "JSX and TSX files in development",
    explanation:
      'sk build and sk publish compile .tsx and .jsx files (with `/** @jsxImportSource vue */` or `"jsxImportSource": "vue"`), but the development builder does not load the app\'s own .tsx and .jsx files, so importing one fails under sk dev.',
    example: 'import Panel from "./Panel.tsx";',
    fix: "Write the component as an SFC, or with `h()` in a .ts file.",
  },
  {
    code: "SK5013",
    severity: "warning",
    title: "a dependency uses Vue or Svelte features Skathi does not support",
    explanation:
      "Components or modules the app imports from this package use what the other SK5 codes report. One diagnostic per package; the places are listed as related locations. The app may not reach them in the parts of the package it uses.",
    example: "a UI kit that renders markdown with `v-html`",
    fix: "Check that the parts of the package the app uses work; allow SK5013 for the package when they do.",
  },
  {
    code: "SK5015",
    title: "feature unavailable in Svelte's native renderer",
    severity: "error",
    explanation:
      "Svelte's experimental custom renderer rejects markup that depends on the DOM renderer, including bindings on elements, transitions and global elements.",
    example: "<input bind:value={name} />",
    fix: 'Set svelte.renderer to "dom", or implement the interaction with event attributes and application state.',
  },
  {
    code: "SK5016",
    title: "Svelte version lacks custom renderers",
    severity: "error",
    explanation:
      "The native renderer uses the experimental API introduced in Svelte 5.59.0. The compiler and runtime come from the app's Svelte installation.",
    example: 'An app with svelte 5.57 and svelte.renderer set to "native".',
    fix: 'Install svelte@^5.59.0, or set svelte.renderer to "dom".',
  },
  {
    code: "SK6001",
    severity: "error",
    title: "`skathi:ffi` is imported, but the app allows no library",
    explanation:
      'A library is loaded by a name from "permissions.ffi" in sk.config.json. The list is missing or empty, so every `dlopen()` in this file throws NotAllowedError. `check.allow` cannot hide this error.',
    example:
      'import { dlopen } from "skathi:ffi";\n\nconst user32 = dlopen("user32", { GetSystemMetrics: { args: ["i32"], returns: "i32" } });',
    fix: 'List the libraries the app loads: "permissions": { "ffi": [{ "name": "user32", "path": "$SYSTEM/user32" }] }.',
    firm: true,
  },
  {
    code: "SK6002",
    severity: "error",
    title: "`dlopen()` names a library the app does not allow",
    explanation:
      'The name given to `dlopen()` is not a path: it must be the "name" of an entry in "permissions.ffi" of sk.config.json. This one is not there, so the call throws NotAllowedError. `check.allow` cannot hide this error.',
    example:
      '// sk.config.json lists "sqlite3" only\nconst curl = dlopen("curl", { curl_version: { returns: "cstring" } });',
    fix: 'Add an entry for the name ({ "name": "curl", "path": "$APP/lib/curl" }), or use a name that is listed.',
    firm: true,
  },
  {
    code: "SK6003",
    severity: "warning",
    title: "the library name given to `dlopen()` is not a literal",
    explanation:
      'The check compares the name with "permissions.ffi". A name computed at run time cannot be compared, so a name that is not in the list shows up only as a NotAllowedError while the app runs. A constant, or a value whose type is a union of string literals, is checked.',
    example: 'const library = dlopen(settings.codec, { decode: { args: ["buffer", "usize"], returns: "i32" } });',
    fix: 'Pass the name as a string literal, or type it as a union of the listed names ("codec" | "codec-fast").',
  },
  {
    code: "SK6005",
    severity: "warning",
    title: "a dependency imports `skathi:ffi`",
    explanation:
      "Code in node_modules loads native libraries and calls them inside the app's process, with the permissions the app's sk.config.json gives. It can load any library in the list, not only the one the package is for.",
    example: 'node_modules/some-codec/index.js: import { dlopen } from "skathi:ffi";',
    fix: "Read what the package loads and calls; allow SK6005 for the package once you trust it.",
  },
];

export const REGISTRY: ReadonlyMap<string, CodeInfo> = new Map(CODES.map((c) => [c.code, c]));

/** Codes no longer reported because the feature now works; the numbers are not reused. */
export const RETIRED: ReadonlyMap<string, string> = new Map([
  ["SK5005", "checkbox and radio inputs work, with v-model, bind:checked and bind:group"],
  ["SK5006", "Svelte `bind:value` on a `<select>` works"],
  ["SK5007", "`<TransitionGroup>` moves and Svelte's `animate:` play"],
  ["SK5014", "a Vue `<Transition>` waits for a CSS animation"],
]);

export function codeInfo(code: string): CodeInfo {
  const info = REGISTRY.get(code);
  if (!info) throw new Error(`unregistered check code ${code}`);
  return info;
}

export const ALL_CODES: readonly CodeInfo[] = CODES;
