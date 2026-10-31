// vitecompat.ts — the Vite behaviours vue-native's builders reproduce — and
// prod.ts over test/fixtures/vite-app, which uses every one of them
// (research/reports/m26/client-gap.md §3.1).
//
//   bun test test/vite-compat.test.ts      (from packages/vue-native)

import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { generate } from "../tailwind";
import {
  GLOB_TRIPWIRE,
  SFC_FS,
  appDefines,
  assetKey,
  devEsmInterop,
  expandGlobs,
  frameworkDep,
  inlineCssImports,
  liftIndexHtml,
  stubWorkerUrls,
  svgAsSfc,
  svgDefaultOf,
  tagModuleNamespaces,
  takeDefineArgs,
} from "../vitecompat";
import { collectAssets } from "../cli/assets";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_DIR = resolve(HERE, "..");
const FIXTURE = join(HERE, "fixtures", "vite-app");

const tmpDirs: string[] = [];
afterAll(() => {
  for (const dir of tmpDirs) rmSync(dir, { recursive: true, force: true });
});
function scratch(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "vn-vitecompat-"));
  tmpDirs.push(root);
  for (const [rel, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), text);
  }
  return root;
}

describe("define", () => {
  test("a string is an expression and anything else is serialized, as Vite's define", () => {
    const { defines, errors } = appDefines(
      { __A__: '"v1"', __B__: "true", __C__: { branch: "vn", n: 2 }, __D__: 3, "process.env.X": "null" },
      [],
    );
    expect(errors).toEqual([]);
    expect(defines).toEqual({
      __A__: '"v1"',
      __B__: "true",
      __C__: '{"branch":"vn","n":2}',
      __D__: "3",
      "process.env.X": "null",
    });
  });

  test("--define wins over the file; Bun-unsafe values and builder-owned keys are refused", () => {
    const ok = appDefines({ __A__: '"file"' }, ["__A__=\"cli\"", "__E__=window.foo"]);
    expect(ok.defines).toEqual({ __A__: '"cli"', __E__: "window.foo" });
    const bad = appDefines({ "import.meta.env": "{}", "a b": "1" }, ["NOVALUE", "__X__=1 + 1"]);
    expect(bad.errors.join("\n")).toContain("set by the builder");
    expect(bad.errors.join("\n")).toContain("not an identifier");
    expect(bad.errors.join("\n")).toContain("expected KEY=VALUE");
    // `1 + 1` is valid JS but Bun would insert it as the STRING "1 + 1".
    expect(bad.errors.join("\n")).toContain("neither JSON nor an identifier");
  });

  test("takeDefineArgs pulls both spellings out of an argv and leaves the positionals", () => {
    expect(takeDefineArgs(["a.ts", "--define", "K=1", "out.js", "--define=J=\"x=y\""])).toEqual({
      defines: ["K=1", 'J="x=y"'],
      rest: ["a.ts", "out.js"],
    });
  });
});

describe("compiler-sfc's file system", () => {
  test("fileExists answers for files only — a directory named like the import must not win", () => {
    const root = scratch({ "types/index.ts": "export {}", "types.ts": "export type A = 1" });
    expect(SFC_FS.fileExists(join(root, "types"))).toBe(false);
    expect(SFC_FS.fileExists(join(root, "types.ts"))).toBe(true);
    expect(SFC_FS.readFile(join(root, "missing.ts"))).toBeUndefined();
  });
});

describe("asset URLs", () => {
  test("inside the document root the key is the root-relative path; outside, a content-addressed one", () => {
    const root = resolve("/app");
    expect(assetKey(join(root, "src/assets/a b.png"), root)).toEqual({
      key: "src/assets/a b.png",
      url: "/src/assets/a%20b.png",
    });
    const outside = assetKey(resolve("/elsewhere/node_modules/x/logo.svg"), root);
    expect(outside.key).toMatch(/^_vn\/fs\/[0-9a-f]{1,8}\/logo\.svg$/);
  });
});

describe("import.meta.glob", () => {
  const root = scratch({
    "src/main.ts": "",
    "src/kinds/a.ts": "",
    "src/kinds/b.ts": "",
    "src/kinds/deep/c.ts": "",
    "src/node_modules/m/x.ts": "",
    "src/other.vue": "",
  });
  const ctx = { file: join(root, "src/main.ts"), docRoot: root };

  test("eager with a named import becomes hoisted static imports, keyed and sorted as Vite keys them", () => {
    const r = expandGlobs(`const k = import.meta.glob("./kinds/*.ts", { eager: true, import: "name" });`, ctx)!;
    expect(r.count).toBe(1);
    expect(r.files).toBe(2);
    expect(r.code).toMatch(/^import \{ name as __vn_glob_0_0 \} from ".*\/src\/kinds\/a\.ts";/);
    expect(r.code).toContain(`({ "./kinds/a.ts": __vn_glob_0_0, "./kinds/b.ts": __vn_glob_0_1 })`);
  });

  test("lazy entries are dynamic imports; negative patterns, ** and the importer itself", () => {
    const r = expandGlobs(`import.meta.glob(["./**/*.ts", "!./kinds/b.ts"])`, ctx)!;
    expect(r.code).toContain(`"./kinds/a.ts": () => import(`);
    expect(r.code).toContain(`"./kinds/deep/c.ts": () => import(`);
    expect(r.code).not.toContain("kinds/b.ts");
    expect(r.code).not.toContain("main.ts"); // the importer never matches itself
    expect(r.code).not.toContain("node_modules"); // skipped unless named
  });

  test("root patterns key from the document root; query and the deprecated `as`", () => {
    const r = expandGlobs(`import.meta.glob("/src/*.vue", { query: "?raw", import: "default" })`, ctx)!;
    expect(r.code).toContain(`"/src/other.vue": () => import(`);
    expect(r.code).toContain(`other.vue?raw").then((m) => m["default"])`);
    const as = expandGlobs(`import.meta.glob("./kinds/a.ts", { as: "url", eager: true })`, ctx)!;
    expect(as.code).toMatch(/import \{ default as __vn_glob_0_0 \} from ".*a\.ts\?url"/);
  });

  test("what Vite refuses is refused with the file and line", () => {
    expect(() => expandGlobs(`\nimport.meta.glob("./x/*.ts", { eagr: true })`, ctx)).toThrow(/main\.ts:2: .*unknown option "eagr"/);
    expect(() => expandGlobs(`import.meta.glob("@/kinds/*.ts")`, ctx)).toThrow(/alias/);
    expect(() => expandGlobs(`import.meta.glob(pattern)`, ctx)).toThrow();
    // A mention is not a call.
    expect(expandGlobs(`type G = typeof import.meta.glob;`, ctx)).toBeNull();
  });

  test("the tripwire name is what a builder defines `import.meta.glob` to", () => {
    expect(GLOB_TRIPWIRE).toMatch(/^[A-Za-z_$][\w$]*$/);
  });
});

describe("workers and svg", () => {
  test("new Worker(new URL(…, import.meta.url)) constructs a class that throws a named error", () => {
    const r = stubWorkerUrls(`const w = new Worker(new URL("./w.ts", import.meta.url), { type: "module" });`, "src/a.ts")!;
    expect(r.count).toBe(1);
    expect(r.code).toContain("new (class VnWorkerUnavailable");
    expect(r.code).toContain("NotSupportedError");
    expect(r.code).toContain(`new URL("./w.ts", import.meta.url), { type: "module" })`);
    expect(stubWorkerUrls(`new Worker("/w.js")`, "src/a.ts")).toBeNull();
  });

  test("a bare .svg is a component exactly when the app uses vite-svg-loader", () => {
    expect(svgDefaultOf({ devDependencies: { "vite-svg-loader": "^5" } })).toBe("component");
    expect(svgDefaultOf({ dependencies: {} })).toBe("url");
    expect(svgDefaultOf(null)).toBe("url");
    const sfc = svgAsSfc(`<?xml version="1.0"?><!-- c --><svg><style>.a{}</style><rect/></svg>`);
    expect(sfc).toBe(`<template><svg><component is="style">.a{}</component><rect/></svg></template>`);
  });
});

// m26 client sweep #2: lazy components rendered nothing (see vitecompat.ts).
describe("module namespaces", () => {
  const MINIFIED =
    "var p=Object.defineProperty;var s=(o)=>o;function x(o,t){this[o]=s.bind(null,t)}" +
    "var n=(o,t)=>{for(var e in t)p(o,e,{get:t[e],enumerable:!0,configurable:!0,set:x.bind(t,e)})};" +
    'var c={};n(c,{default:()=>u});var u={name:"B"};globalThis.__nsUnit=c;';

  test("Bun's __export helper gives every namespace the Module tag and a null prototype", () => {
    const r = tagModuleNamespaces(MINIFIED);
    expect(r.count).toBe(1);
    new Function(r.code)();
    const ns = (globalThis as any).__nsUnit;
    expect(ns[Symbol.toStringTag]).toBe("Module");
    expect(Object.getPrototypeOf(ns)).toBe(null);
    expect(ns.default.name).toBe("B");
    // Unminified, as Bun writes it without `minify`.
    const plain =
      "var __export = (target, all) => {\n  for (var name in all)\n    __defProp(target, name, {\n" +
      "      get: all[name],\n      enumerable: true,\n      configurable: true,\n      set: __exportSetter.bind(all, name)\n    });\n};";
    expect(tagModuleNamespaces(plain).count).toBe(1);
  });

  test("a helper in a shape it does not know fails the build; a bundle with none passes", () => {
    const changed = MINIFIED.replace("for(var e in t)", "for(const e of Object.keys(t))");
    expect(() => tagModuleNamespaces(changed)).toThrow(/__export helper/);
    expect(tagModuleNamespaces("console.log(1)")).toEqual({ code: "console.log(1)", count: 0 });
  });

  test("dev: Node's interop flag is dropped, and import() goes through require", () => {
    const body =
      'var import_App = __toESM(require("./App.vue"), 1);\n' +
      "var lazy = () => import(\"./Lazy.vue\");\nvar other = x.import('y');\nvar dyn = (p) => import(p);\n";
    const r = devEsmInterop(body);
    expect(r.code).toContain('__toESM(require("./App.vue"));');
    expect(r.code).toContain('Promise.resolve().then(() => __vnImportNs(require("./Lazy.vue")))');
    expect(r.code).toContain("x.import('y')");
    expect(r.code).toContain("function __vnImportNs(m)");
    expect(r.unresolved).toBe(1);
    // A CommonJS module's exports become a namespace with `default`, as Vite gives one.
    const helper = r.code.slice(r.code.indexOf("function __vnImportNs"));
    const ns = new Function(`${helper}\nreturn __vnImportNs;`)()({ a: 1 });
    expect(ns[Symbol.toStringTag]).toBe("Module");
    expect(ns.default).toEqual({ a: 1 });
    expect(ns.a).toBe(1);
    expect(devEsmInterop("var a = 1;")).toEqual({ code: "var a = 1;", unresolved: 0 });
  });
});

describe("CSS @import inlining", () => {
  test("relative, rooted and nested imports inline; a media list wraps; a cycle stops; remote stays", () => {
    const root = scratch({
      "a.css": `@import "./b.css" screen;\n@import "/c.css";\n@import url("https://x.invalid/r.css");\n.a{}`,
      "b.css": `@import "./a.css";\n.b{}`,
      "c.css": `.c{}`,
    });
    const warned: string[] = [];
    const css = inlineCssImports(readFileSync(join(root, "a.css"), "utf8"), join(root, "a.css"), root, (m) => warned.push(m));
    expect(css).toContain("@media screen {\n\n.b{}\n}");
    expect(css).toContain(".c{}");
    expect(css).toContain(`@import url("https://x.invalid/r.css");`);
    expect(css.match(/\.a\{\}/g)?.length).toBe(1);
    expect(warned).toEqual([]);
    inlineCssImports(`@import "./nope.css";`, join(root, "a.css"), root, (m) => warned.push(m));
    expect(warned[0]).toContain("does not resolve");
  });
});

describe("index.html", () => {
  test("head and body CSS, the classic script, the mount element's attributes, and what is not carried", () => {
    const lifted = liftIndexHtml(join(FIXTURE, "index.html"), FIXTURE);
    expect(lifted.headCss.map((p) => p.text.trim().split("\n")[0])).toEqual([
      ".from-head { color: red; }",
      `@import "./src/styles/tokens.css";`,
    ]);
    expect(lifted.bodyCss).toEqual([]);
    expect(lifted.scripts.map((s) => s.text.trim())).toEqual([`window.__fixTheme = "dark";`]);
    expect(lifted.rootClasses).toEqual(["body-cls"]);
    expect(lifted.rootAttrs).toEqual({ "data-wrapper": "" });
    expect(lifted.advisories.join("\n")).toContain("remote stylesheet https://example.invalid/remote.css");
    expect(lifted.advisories.join("\n")).toContain("<svg>, <filter>");
  });
});

describe("framework packages", () => {
  test("the app's own copy when it declares and installs one, with a warning on a major mismatch", () => {
    const root = scratch({
      "app/node_modules/pinia/package.json": JSON.stringify({ name: "pinia", version: "3.0.4" }),
      "app/node_modules/pinia/index.js": "",
      "vn/node_modules/pinia/package.json": JSON.stringify({ name: "pinia", version: "4.0.3" }),
      "vn/node_modules/pinia/index.js": "",
    });
    const appResolve = () => join(root, "app/node_modules/pinia/index.js");
    const vnResolve = () => join(root, "vn/node_modules/pinia/index.js");
    const own = frameworkDep("pinia", { dependencies: { pinia: "^3.0.4" } }, appResolve, vnResolve);
    expect(own).toMatchObject({ from: "app", version: "3.0.4" });
    expect(own.warning).toContain("different majors");
    const theirs = frameworkDep("pinia", { dependencies: {} }, appResolve, vnResolve);
    expect(theirs).toMatchObject({ from: "vue-native", version: "4.0.3" });
    expect(theirs.warning).toBeUndefined();
  });
});

describe("Tailwind runs in the config's directory", () => {
  test("a relative content glob scans the app, not the builder's cwd, and cwd is restored", async () => {
    const root = scratch({
      "tailwind.config.ts": `export default { content: ["./src/**/*.vue"], corePlugins: { preflight: false } };`,
      "src/A.vue": `<template><div class="pt-7 text-center"></div></template>`,
    });
    const before = process.cwd();
    const [a, b] = await Promise.all([generate(join(root, "tailwind.config.ts")), generate(join(root, "tailwind.config.ts"))]);
    expect(process.cwd()).toBe(before);
    for (const css of [a, b]) {
      expect(css).toContain(".pt-7");
      expect(css).toContain(".text-center");
    }
  });
});

describe("publicDir", () => {
  test("public/x is packaged as x; an undeclared public/ is a warning", () => {
    const withIt = collectAssets(FIXTURE, ["assets"], false, "public");
    expect(withIt.files.map((f) => f.key)).toEqual(["robots.txt"]);
    const without = collectAssets(FIXTURE, ["assets"], false, null);
    expect(without.files).toEqual([]);
    expect(without.warnings.join("\n")).toContain('declares no "publicDir"');
  });
});

describe("prod.ts over test/fixtures/vite-app", () => {
  const out = mkdtempSync(join(tmpdir(), "vn-viteapp-"));
  tmpDirs.push(out);
  const proc = Bun.spawnSync(["bun", "prod.ts", join(FIXTURE, "src/main.ts"), join(out, "app.prod.js")], { cwd: PKG_DIR });
  const log = proc.stdout.toString() + proc.stderr.toString();
  const body = proc.exitCode === 0 ? readFileSync(join(out, "app.prod.js"), "utf8") : "";
  /** Boolean, so a failure prints one line instead of the whole bundle. */
  const has = (s: string) => body.includes(s);

  test("builds", () => {
    if (proc.exitCode !== 0) throw new Error(log);
    expect(body.length).toBeGreaterThan(0);
  });

  test("?raw, asset URLs, ?url, .svg as a component, and assets.json lists what to pack", () => {
    expect(has("hello from a ?raw import")).toBe(true);
    expect(has(`"/src/assets/logo.png"`)).toBe(true);
    expect(has(`"/src/assets/icon.svg"`)).toBe(true);
    expect(log).toContain(".svg as a component: 1");
    const assets = JSON.parse(readFileSync(join(out, "assets.json"), "utf8")) as { key: string; path: string }[];
    expect(assets.map((a) => a.key)).toEqual(["src/assets/icon.svg", "src/assets/logo.png"]);
    expect(resolve(assets[1]!.path)).toBe(join(FIXTURE, "src/assets/logo.png"));
  });

  test("import.meta.glob in a .ts and in an SFC's <script>, and no survivor", () => {
    expect(log).toContain("import.meta.glob calls expanded: 3");
    expect(has("kind-a")).toBe(true);
    expect(has(GLOB_TRIPWIRE)).toBe(false);
  });

  test("workers and wasm are named failures, with the file in the message", () => {
    expect(has("work.ts?worker): vue-native has no Web Workers")).toBe(true);
    expect(has(`new Worker(new URL("./work.ts", import.meta.url)) in packages/vue-native/test/fixtures/vite-app/src/main.ts`)).toBe(true);
    expect(has("vn-unsupported:wasm/packages/vue-native/test/fixtures/vite-app/src/mod.wasm")).toBe(true);
  });

  test("vn.config.json define, including an import.meta.env key", () => {
    expect(/branch:"vn"/.test(body)).toBe(true);
    expect(/greeting:"hi"/.test(body)).toBe(true);
    expect(has("__FIX_BUILD__")).toBe(false);
    expect(has("__FIX_FLAG__")).toBe(false);
  });

  test("index.html: its CSS ahead of the bundle's, the classic script and the root attributes", () => {
    const head = body.indexOf(".from-head");
    const tokens = body.indexOf(".tokens"); // global.css's own @import, inlined
    const global = body.indexOf(".global {");
    const sfc = body.indexOf(".app-root[data-v-");
    expect(head).toBeGreaterThan(-1);
    expect(head < tokens && tokens < global && global < sfc).toBe(true);
    expect(has("__fixTheme")).toBe(true);
    expect(/setAttribute\("data-wrapper",\s*""\)/.test(body)).toBe(true);
    expect(log).toContain("remote stylesheet https://example.invalid/remote.css");
  });

  test("an SFC <style>: @import inlined and scoped, @apply expanded, Tailwind's relative content glob found", () => {
    expect(/\.imported-into-scoped\[data-v-[0-9a-f]+\]/.test(body)).toBe(true);
    expect(/\.app-root\[data-v-[0-9a-f]+\] \{ margin-top: 0\.5rem;/.test(body)).toBe(true);
    expect(/\.p-4 \{\\n\s+padding: 1rem/.test(body)).toBe(true);
    // v-bind(): the style's variable and the script's useCssVars key agree.
    const v = /color: var\(--(v?[0-9a-f]{8})\)/.exec(body)?.[1];
    expect(v).toBeTruthy();
    expect(has(`${v}:`)).toBe(true);
  });

  test("a define the builder cannot express fails the build with a sentence", () => {
    const bad = Bun.spawnSync(
      ["bun", "prod.ts", join(FIXTURE, "src/main.ts"), join(out, "bad.js"), "--define", "__X__=1 + 1"],
      { cwd: PKG_DIR },
    );
    expect(bad.exitCode).toBe(1);
    expect(bad.stdout.toString() + bad.stderr.toString()).toContain("neither JSON nor an identifier");
  });
});

describe("build.ts (dev) over test/fixtures/vite-dev-app", () => {
  const dist = mkdtempSync(join(tmpdir(), "vn-vitedev-"));
  tmpDirs.push(dist);
  const proc = Bun.spawnSync(["bun", "build.ts", join(HERE, "fixtures/vite-dev-app/src/main.ts"), dist], { cwd: PKG_DIR });
  const mod = (rel: string) =>
    readFileSync(join(dist, "modules", `packages__vue-native__test__fixtures__vite-dev-app__src__${rel}.js`), "utf8");

  test("the app's define and an SFC <style> through @import, Tailwind and the scoped transform", () => {
    if (proc.exitCode !== 0) throw new Error(proc.stdout.toString() + proc.stderr.toString());
    expect(mod("main.ts").includes("__devFix = { on: true }.on")).toBe(true);
    const app = mod("App.vue");
    expect(/\.dev-part\[data-v-[0-9a-f]+\] \{ color: teal;/.test(app)).toBe(true);
    expect(/\.dev-root\[data-v-[0-9a-f]+\] \{ margin-bottom: 1rem;/.test(app)).toBe(true);
  });

  test("an import.meta.glob it cannot expand fails the build with a sentence, not at eval", () => {
    // Every literal call expands in dev, `query` ones included
    // (test/build-graph.test.ts, esm-app). A mention that is not a call is left
    // for the define, whose tripwire names it.
    // Inside the workspace, as an app is: module ids are workspace-relative.
    const root = mkdtempSync(join(HERE, "fixtures", "glob-mention-"));
    tmpDirs.push(root);
    mkdirSync(join(root, "src"), { recursive: true });
    writeFileSync(join(root, "src/main.ts"), "const glob = import.meta.glob;\nconsole.log(glob);\n");
    const glob = Bun.spawnSync(["bun", "build.ts", join(root, "src/main.ts"), join(dist, "glob")], { cwd: PKG_DIR });
    expect(glob.exitCode).not.toBe(0);
    expect(glob.stdout.toString() + glob.stderr.toString()).toContain("an import.meta.glob the expansion did not see");
  });
});

// The same fixture prod.ts is tested on above: dev gives each shape the value
// prod gives it, through the same plugin.
describe("build.ts (dev) over test/fixtures/vite-app", () => {
  const dist = mkdtempSync(join(tmpdir(), "vn-viteapp-dev-"));
  tmpDirs.push(dist);
  const proc = Bun.spawnSync(["bun", "build.ts", join(FIXTURE, "src/main.ts"), dist], { cwd: PKG_DIR });
  const log = proc.stdout.toString() + proc.stderr.toString();
  const main = proc.exitCode === 0
    ? readFileSync(join(dist, "modules", "packages__vue-native__test__fixtures__vite-app__src__main.ts.js"), "utf8")
    : "";
  const has = (s: string) => main.includes(s);

  test("builds, with every import resolved", () => {
    if (proc.exitCode !== 0) throw new Error(log);
    expect(log).not.toContain("resolves to no built module");
    expect(log).not.toContain("cannot resolve");
  });

  test("?raw, asset URLs, ?url and .svg as a component", () => {
    expect(has("hello from a ?raw import")).toBe(true);
    expect(has(`"/src/assets/logo.png"`)).toBe(true);
    expect(has(`"/src/assets/icon.svg"`)).toBe(true);
    expect(log).toContain(".svg as a component: 1");
  });

  test("an .svg component compiled inside an SFC's build leaves that SFC's <style> alone", () => {
    const root = mkdtempSync(join(HERE, "fixtures", "svg-sfc-"));
    tmpDirs.push(root);
    mkdirSync(join(root, "src"), { recursive: true });
    writeFileSync(join(root, "package.json"), JSON.stringify({ private: true, devDependencies: { "vite-svg-loader": "*" } }));
    writeFileSync(join(root, "src/i.svg"), '<svg viewBox="0 0 1 1"><path d="M0 0h1"/></svg>');
    writeFileSync(
      join(root, "src/App.vue"),
      '<script setup>\nimport Icon from "./i.svg";\n</script>\n<template><Icon class="mark" /></template>\n<style>\n.mark { color: red; }\n</style>\n',
    );
    writeFileSync(join(root, "src/main.ts"), 'import App from "./App.vue";\nexport default App;\n');
    const out = join(dist, "svg-sfc");
    const p = Bun.spawnSync(["bun", "build.ts", join(root, "src/main.ts"), out], { cwd: PKG_DIR });
    if (p.exitCode !== 0) throw new Error(p.stdout.toString() + p.stderr.toString());
    const name = "packages/vue-native/test/fixtures/" + root.split(/[\\/]/).pop() + "/src/App.vue";
    const app = readFileSync(join(out, "modules", name.split("/").join("__") + ".js"), "utf8");
    expect(app).toContain('d: "M0 0h1"');
    expect(/__VN__\.setCss\([^)]*\.mark \{/.test(app)).toBe(true);
  });

  test("workers and wasm are the named failures prod gives", () => {
    expect(has("work.ts?worker): vue-native has no Web Workers")).toBe(true);
    expect(has(`new Worker(new URL("./work.ts", import.meta.url)) in packages/vue-native/test/fixtures/vite-app/src/main.ts`)).toBe(true);
    expect(has("vn-unsupported:wasm/packages/vue-native/test/fixtures/vite-app/src/mod.wasm")).toBe(true);
  });
});
