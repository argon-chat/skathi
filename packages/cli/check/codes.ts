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
}

export const RANGES = [
  { from: 0, to: 999, what: "the check itself" },
  { from: 1000, to: 1999, what: "breaks under AOT" },
  { from: 2000, to: 2999, what: "works under AOT, but slower" },
  { from: 3000, to: 3999, what: "typing quality: compiles, but AOT gets no type information" },
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
    code: "SK1001",
    severity: "error",
    title: "a built-in method or property is replaced or deleted",
    explanation:
      "In an AOT image the engine's built-in objects are read-only: replacing or deleting a member they already have throws a TypeError at run time (the build rejects it outright when the assignment is unconditional), even though the property descriptor still reports it as writable. Adding a new member works, except on Object.prototype, Promise.prototype and RegExp.prototype, which take no new members. Assignments guarded by a check for the member (polyfills such as `if (!Array.prototype.at) …`) and assignments that put back the value read from the same property are not reported.",
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
];

export const REGISTRY: ReadonlyMap<string, CodeInfo> = new Map(CODES.map((c) => [c.code, c]));

export function codeInfo(code: string): CodeInfo {
  const info = REGISTRY.get(code);
  if (!info) throw new Error(`unregistered check code ${code}`);
  return info;
}

export const ALL_CODES: readonly CodeInfo[] = CODES;
