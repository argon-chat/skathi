// vitecompat.ts (the Vite behaviours the builders reproduce) and prod.ts over test/fixtures/vite-app, which uses every
// one of them.

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
  htmlPreludeModule,
  rebaseCssUrls,
  liftIndexHtml,
  stubWorkerUrls,
  svgAsSfc,
  svgDefaultOf,
  tagModuleNamespaces,
  takeDefineArgs,
} from "../vitecompat";
import { collectAssets } from "../cli/assets";
import { buildApp } from "../cli/build-app";
import { KIND, readSkpk } from "../cli/skpk";
import { createRuntime } from "../../runtime/test/runtime-harness";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_DIR = resolve(HERE, "..");
const FIXTURE = join(HERE, "fixtures", "vite-app");

const tmpDirs: string[] = [];
afterAll(() => {
  for (const dir of tmpDirs) rmSync(dir, { recursive: true, force: true });
});
function scratch(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "sk-vitecompat-"));
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
      { __A__: '"v1"', __B__: "true", __C__: { branch: "sk", n: 2 }, __D__: 3, "process.env.X": "null" },
      [],
    );
    expect(errors).toEqual([]);
    expect(defines).toEqual({
      __A__: '"v1"',
      __B__: "true",
      __C__: '{"branch":"sk","n":2}',
      __D__: "3",
      "process.env.X": "null",
    });
  });

  test("--define wins over the file; Bun-unsafe values and builder-owned keys are refused", () => {
    const ok = appDefines({ __A__: '"file"' }, ['__A__="cli"', "__E__=window.foo"]);
    expect(ok.defines).toEqual({ __A__: '"cli"', __E__: "window.foo" });
    const bad = appDefines({ "import.meta.env": "{}", "a b": "1" }, ["NOVALUE", "__X__=1 + 1"]);
    expect(bad.errors.join("\n")).toContain("set by the builder");
    expect(bad.errors.join("\n")).toContain("not an identifier");
    expect(bad.errors.join("\n")).toContain("expected KEY=VALUE");
    // `1 + 1` is valid JS but Bun would insert it as the STRING "1 + 1".
    expect(bad.errors.join("\n")).toContain("neither JSON nor an identifier");
  });

  test("takeDefineArgs pulls both spellings out of an argv and leaves the positionals", () => {
    expect(takeDefineArgs(["a.ts", "--define", "K=1", "out.js", '--define=J="x=y"'])).toEqual({
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
    expect(r.code).toMatch(/^import \{ name as __sk_glob_0_0 \} from ".*\/src\/kinds\/a\.ts";/);
    expect(r.code).toContain(`({ "./kinds/a.ts": __sk_glob_0_0, "./kinds/b.ts": __sk_glob_0_1 })`);
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
    expect(as.code).toMatch(/import \{ default as __sk_glob_0_0 \} from ".*a\.ts\?url"/);
  });

  test("what Vite refuses is refused with the file and line", () => {
    expect(() => expandGlobs(`\nimport.meta.glob("./x/*.ts", { eagr: true })`, ctx)).toThrow(
      /main\.ts:2: .*unknown option "eagr"/,
    );
    expect(() => expandGlobs(`import.meta.glob("@/kinds/*.ts")`, ctx)).toThrow(/alias/);
    expect(() => expandGlobs(`import.meta.glob(pattern)`, ctx)).toThrow();
    expect(expandGlobs(`type G = typeof import.meta.glob;`, ctx)).toBeNull();
  });

  test("the tripwire name is what a builder defines `import.meta.glob` to", () => {
    expect(GLOB_TRIPWIRE).toMatch(/^[A-Za-z_$][\w$]*$/);
  });
});

describe("workers and svg", () => {
  test("new Worker(new URL(…, import.meta.url)) constructs a class that throws a named error", () => {
    const r = stubWorkerUrls(
      `const w = new Worker(new URL("./w.ts", import.meta.url), { type: "module" });`,
      "src/a.ts",
    )!;
    expect(r.count).toBe(1);
    expect(r.code).toContain("new (class SkWorkerUnavailable");
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
    expect(r.code).toContain('Promise.resolve().then(() => __skImportNs(require("./Lazy.vue")))');
    expect(r.code).toContain("x.import('y')");
    expect(r.code).toContain("function __skImportNs(m)");
    expect(r.unresolved).toBe(1);
    const helper = r.code.slice(r.code.indexOf("function __skImportNs"));
    const ns = new Function(`${helper}\nreturn __skImportNs;`)()({ a: 1 });
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
    const css = inlineCssImports(readFileSync(join(root, "a.css"), "utf8"), join(root, "a.css"), root, (m) =>
      warned.push(m),
    );
    expect(css).toContain("@media screen {\n\n.b{}\n}");
    expect(css).toContain(".c{}");
    expect(css).toContain(`@import url("https://x.invalid/r.css");`);
    expect(css.match(/\.a\{\}/g)?.length).toBe(1);
    expect(warned).toEqual([]);
    inlineCssImports(`@import "./nope.css";`, join(root, "a.css"), root, (m) => warned.push(m));
    expect(warned[0]).toContain("does not resolve");
  });

  test("a sheet's relative url()s are rebased against its own file, an imported sheet's against its own", () => {
    const root = scratch({
      "packages/assets/fonts/fonts.css": [
        `@font-face { font-family: Lato; src: url('./Lato/Lato-Bold.ttf') format('truetype'); }`,
        `@font-face { font-family: Tengwar; src: url(AlcarinTengwar/a.woff2?v=2#x); }`,
        `.k { background: url("../img/k.png"), url(data:image/png;base64,AA==), url(/rooted.png), url(#f), url(var(--u)); }`,
        `@import url("./more.css");`,
      ].join("\n"),
      "packages/assets/fonts/more.css": `.m { background: url(./m.png) }`,
    });
    const from = join(root, "packages/assets/fonts/fonts.css");
    const css = inlineCssImports(readFileSync(from, "utf8"), from, root, () => {});
    expect(css).toContain(`url('/packages/assets/fonts/Lato/Lato-Bold.ttf') format('truetype')`);
    expect(css).toContain(`url("/packages/assets/fonts/AlcarinTengwar/a.woff2?v=2#x")`);
    expect(css).toContain(`url("/packages/assets/img/k.png")`);
    expect(css).toContain(`url(data:image/png;base64,AA==), url(/rooted.png), url(#f), url(var(--u))`);
    expect(css).toContain(`.m { background: url("/packages/assets/fonts/m.png") }`);
    const out = rebaseCssUrls(`a{b:url(../up.png)}`, join(root, "x.css"), root);
    expect(out).toMatch(/^a\{b:url\("file:\/\/\/.*\/up\.png"\)\}$/);
  });

  test("CSS assets are collected through nested imports; comments, strings and remote URLs are not files", () => {
    const base = scratch({
      "app/main.css": [
        `@import "./styles/fonts.css";`,
        `.image { background: url(./icon%20one.png?v=2#preview), url(/root.png); }`,
        `/* url(./ignored.png) */ .literal { content: "url(./ignored.png)"; }`,
        `.remote { background: url(https://example.invalid/image.png), url(data:image/png;base64,AA==), url(#filter), url(var(--image)); }`,
      ].join("\n"),
      "app/styles/fonts.css": `@font-face { font-family: Pack; src: url('../../shared/font.woff2?v=3#face'); }`,
      "app/icon one.png": "image bytes",
      "app/root.png": "root image",
      "app/ignored.png": "must not be collected",
      "shared/font.woff2": "font bytes",
    });
    const root = join(base, "app");
    const from = join(root, "main.css");
    const assets = new Map<string, string>();
    const css = inlineCssImports(readFileSync(from, "utf8"), from, root, () => {}, undefined, assets);
    const outside = assetKey(join(base, "shared/font.woff2"), root);
    expect([...assets.keys()].sort()).toEqual([outside.key, "icon one.png", "root.png"].sort());
    expect(assets.get(outside.key)).toBe(join(base, "shared/font.woff2"));
    expect(css).toContain(`url('${outside.url}?v=3#face')`);
    expect(css).toContain(`url("/icon%20one.png?v=2#preview")`);
    expect(css).toContain(`/* url(./ignored.png) */ .literal { content: "url(./ignored.png)"; }`);
    expect(css).toContain("url(https://example.invalid/image.png)");
    expect(css).toContain("url(data:image/png;base64,AA==), url(#filter), url(var(--image))");
  });
});

describe("packaged CSS fonts", () => {
  test("sk build packs actual TTF and WOFF2 bytes referenced only by @font-face", async () => {
    const target = resolve(PKG_DIR, "../test/fixtures/fonts");
    const out = mkdtempSync(join(tmpdir(), "sk-css-font-payload-"));
    tmpDirs.push(out);
    const result = await buildApp({ target, out, payloadOnly: true, log: () => {} });
    const payload = readSkpk(readFileSync(result.skpk));
    for (const name of ["src/fonts/PressStart2P-Regular.ttf", "src/fonts/AlcarinTengwarVF.woff2"]) {
      const entry = payload.find(KIND.ASSET, name);
      expect(entry).toBeDefined();
      expect(Buffer.from(payload.bytes(entry!))).toEqual(readFileSync(join(target, name)));
    }
  }, 30_000);
});

describe("index.html", () => {
  test("lifted SVG definitions become queryable native nodes with case, namespaces and literal text intact", () => {
    const values = "1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0";
    const filters = Array.from(
      { length: 6 },
      (_, index) => `<filter id="filter-${index}"><feColorMatrix type="matrix" values="${values}" /></filter>`,
    ).join("");
    const root = scratch({
      "index.html": `<body><svg style="display:none" viewBox="0 0 10 10" xmlns:xlink="http://www.w3.org/1999/xlink"><defs>${filters}</defs><use xlink:href="#shape" xml:lang="ru"/><title>A &amp; B {{literal}}</title></svg><div id="app"></div></body>`,
    });
    const lifted = liftIndexHtml(join(root, "index.html"), root);
    const rt = createRuntime();
    const document = rt.global.document as unknown as Document;
    const source = htmlPreludeModule(lifted).replace(/^import "@skathi-native\/runtime";\s*/m, "");
    new Function("document", source)(document);
    const all = (node: Node): Element[] =>
      Array.from(node.childNodes).flatMap((child) => (child.nodeType === 1 ? [child as Element, ...all(child)] : []));
    const nodes = all(document.body);
    const svg = nodes.find((node) => node.localName === "svg")!;
    expect(svg.namespaceURI).toBe("http://www.w3.org/2000/svg");
    expect(svg.getAttribute("viewBox")).toBe("0 0 10 10");
    expect(nodes.filter((node) => node.localName === "filter")).toHaveLength(6);
    const primitives = nodes.filter((node) => node.localName === "feColorMatrix");
    expect(primitives).toHaveLength(6);
    expect(
      primitives.every((node) => node.namespaceURI === svg.namespaceURI && node.getAttribute("values") === values),
    ).toBe(true);
    expect(rt.host.props.filter((prop) => prop.key === "values")).toHaveLength(6);
    const use = nodes.find((node) => node.localName === "use")!;
    expect(use.getAttributeNS("http://www.w3.org/1999/xlink", "href")).toBe("#shape");
    expect(use.getAttributeNS("http://www.w3.org/XML/1998/namespace", "lang")).toBe("ru");
    expect(nodes.find((node) => node.localName === "title")!.textContent).toBe("A & B {{literal}}");
  });
  test("app classes and attributes stay separate from html/body theme classes", () => {
    const root = scratch({
      "index.html": `<html class="dark"><body class="body-theme"><main id="app" class="app-layout" vaul-drawer-wrapper style="height:100%"></main></body></html>`,
    });
    const lifted = liftIndexHtml(join(root, "index.html"), root);
    expect(lifted.rootClasses).toEqual(["dark", "body-theme"]);
    expect(lifted.rootAttrs).toEqual({});
    expect(lifted.mountElement).toEqual({
      tag: "main",
      attrs: {
        id: "app",
        class: "app-layout",
        "vaul-drawer-wrapper": "",
        style: "height:100%",
      },
    });
  });
  test("head and body CSS, the classic script, the mount element's attributes, and what is not carried", () => {
    const lifted = liftIndexHtml(join(FIXTURE, "index.html"), FIXTURE);
    expect(lifted.headCss.map((p) => p.text.trim().split("\n")[0])).toEqual([
      ".from-head { color: red; }",
      `@import "./src/styles/tokens.css";`,
    ]);
    expect(lifted.bodyCss).toEqual([]);
    expect(lifted.scripts.map((s) => s.text.trim())).toEqual([`window.__fixTheme = "dark";`]);
    expect(lifted.rootClasses).toEqual(["body-cls"]);
    expect(lifted.rootAttrs).toEqual({});
    expect(lifted.mountElement).toEqual({ tag: "div", attrs: { id: "app", "data-wrapper": "" } });
    expect(lifted.advisories.join("\n")).toContain("remote stylesheet https://example.invalid/remote.css");
    expect(lifted.svgDefinitions).toEqual(['<svg width="0" height="0"><filter id="f"></filter></svg>']);
    expect(lifted.advisories.join("\n")).not.toContain("<svg>");
  });
});

describe("framework packages", () => {
  test("both builders use the app's installed copy and warn only about incompatible missing-dependency fallback", () => {
    const root = scratch({
      "app/node_modules/pinia/package.json": JSON.stringify({ name: "pinia", version: "3.0.4" }),
      "app/node_modules/pinia/index.js": "",
      "sk/node_modules/pinia/package.json": JSON.stringify({ name: "pinia", version: "4.0.3" }),
      "sk/node_modules/pinia/index.js": "",
    });
    const appResolve = () => join(root, "app/node_modules/pinia/index.js");
    const skResolve = () => join(root, "sk/node_modules/pinia/index.js");
    const own = frameworkDep("pinia", { dependencies: { pinia: "^3.0.4" } }, appResolve, skResolve);
    expect(own).toMatchObject({ from: "app", version: "3.0.4" });
    expect(own.warning).toBeUndefined();
    const missing = frameworkDep(
      "pinia",
      { dependencies: { pinia: "^3.0.4" } },
      () => {
        throw new Error("not installed");
      },
      skResolve,
    );
    expect(missing).toMatchObject({ from: "@skathi-native/runtime", version: "4.0.3" });
    expect(missing.warning).toContain("not installed");
    expect(missing.warning).toContain("different major");
    const theirs = frameworkDep("pinia", { dependencies: {} }, appResolve, skResolve);
    expect(theirs).toMatchObject({ from: "@skathi-native/runtime", version: "4.0.3" });
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
    const [a, b] = await Promise.all([
      generate(join(root, "tailwind.config.ts")),
      generate(join(root, "tailwind.config.ts")),
    ]);
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
  const out = mkdtempSync(join(tmpdir(), "sk-viteapp-"));
  tmpDirs.push(out);
  const proc = Bun.spawnSync(["bun", "prod.ts", join(FIXTURE, "src/main.ts"), join(out, "app.prod.js")], {
    cwd: PKG_DIR,
  });
  const log = proc.stdout.toString() + proc.stderr.toString();
  const body = proc.exitCode === 0 ? readFileSync(join(out, "app.prod.js"), "utf8") : "";
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
    expect(has("work.ts?worker): Skathi has no Web Workers")).toBe(true);
    expect(
      has(`new Worker(new URL("./work.ts", import.meta.url)) in packages/cli/test/fixtures/vite-app/src/main.ts`),
    ).toBe(true);
    expect(has("sk-unsupported:wasm/packages/cli/test/fixtures/vite-app/src/mod.wasm")).toBe(true);
  });

  test("sk.config.json define, including an import.meta.env key", () => {
    expect(/branch:"sk"/.test(body)).toBe(true);
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
    const v = /color: var\(--(v?[0-9a-f]{8})\)/.exec(body)?.[1];
    expect(v).toBeTruthy();
    expect(has(`${v}:`)).toBe(true);
  });

  test("template comments are dropped, so a comment before the root keeps it the single root", () => {
    // Kept, the comment made App.vue's root a Fragment, which the prod runtime
    // does not see through: no fallthrough attrs, no parent scope id.
    expect(has("A root comment, which a production build drops")).toBe(false);
    expect(has(".app-root[data-v-")).toBe(true);
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
  const dist = mkdtempSync(join(tmpdir(), "sk-vitedev-"));
  tmpDirs.push(dist);
  const proc = Bun.spawnSync(["bun", "build.ts", join(HERE, "fixtures/vite-dev-app/src/main.ts"), dist], {
    cwd: PKG_DIR,
  });
  const mod = (rel: string) =>
    readFileSync(join(dist, "modules", `packages__cli__test__fixtures__vite-dev-app__src__${rel}.js`), "utf8");

  test("the app's define and an SFC <style> through @import, Tailwind and the scoped transform", () => {
    if (proc.exitCode !== 0) throw new Error(proc.stdout.toString() + proc.stderr.toString());
    expect(mod("main.ts").includes("__devFix = { on: true }.on")).toBe(true);
    const app = mod("App.vue");
    expect(/\.dev-part\[data-v-[0-9a-f]+\] \{ color: teal;/.test(app)).toBe(true);
    expect(/\.dev-root\[data-v-[0-9a-f]+\] \{ margin-bottom: 1rem;/.test(app)).toBe(true);
  });

  test("an import.meta.glob it cannot expand fails the build with a sentence, not at eval", () => {
    const root = mkdtempSync(join(HERE, "fixtures", "glob-mention-"));
    tmpDirs.push(root);
    mkdirSync(join(root, "src"), { recursive: true });
    writeFileSync(join(root, "src/main.ts"), "const glob = import.meta.glob;\nconsole.log(glob);\n");
    const glob = Bun.spawnSync(["bun", "build.ts", join(root, "src/main.ts"), join(dist, "glob")], { cwd: PKG_DIR });
    expect(glob.exitCode).not.toBe(0);
    expect(glob.stdout.toString() + glob.stderr.toString()).toContain("an import.meta.glob the expansion did not see");
  });
});

describe("build.ts (dev) over test/fixtures/vite-app", () => {
  const dist = mkdtempSync(join(tmpdir(), "sk-viteapp-dev-"));
  tmpDirs.push(dist);
  const proc = Bun.spawnSync(["bun", "build.ts", join(FIXTURE, "src/main.ts"), dist], { cwd: PKG_DIR });
  const log = proc.stdout.toString() + proc.stderr.toString();
  const main =
    proc.exitCode === 0
      ? readFileSync(join(dist, "modules", "packages__cli__test__fixtures__vite-app__src__main.ts.js"), "utf8")
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
    writeFileSync(
      join(root, "package.json"),
      JSON.stringify({ private: true, devDependencies: { "vite-svg-loader": "*" } }),
    );
    writeFileSync(join(root, "src/i.svg"), '<svg viewBox="0 0 1 1"><path d="M0 0h1"/></svg>');
    writeFileSync(
      join(root, "src/App.vue"),
      '<script setup>\nimport Icon from "./i.svg";\n</script>\n<template><Icon class="mark" /></template>\n<style>\n.mark { color: red; }\n</style>\n',
    );
    writeFileSync(join(root, "src/main.ts"), 'import App from "./App.vue";\nexport default App;\n');
    const out = join(dist, "svg-sfc");
    const p = Bun.spawnSync(["bun", "build.ts", join(root, "src/main.ts"), out], { cwd: PKG_DIR });
    if (p.exitCode !== 0) throw new Error(p.stdout.toString() + p.stderr.toString());
    const name = "packages/cli/test/fixtures/" + root.split(/[\\/]/).pop() + "/src/App.vue";
    const app = readFileSync(join(out, "modules", name.split("/").join("__") + ".js"), "utf8");
    expect(app).toContain('d: "M0 0h1"');
    expect(/__SK__\.setCss\([^)]*\.mark \{/.test(app)).toBe(true);
  });

  test("workers and wasm are the named failures prod gives", () => {
    expect(has("work.ts?worker): Skathi has no Web Workers")).toBe(true);
    expect(
      has(`new Worker(new URL("./work.ts", import.meta.url)) in packages/cli/test/fixtures/vite-app/src/main.ts`),
    ).toBe(true);
    expect(has("sk-unsupported:wasm/packages/cli/test/fixtures/vite-app/src/mod.wasm")).toBe(true);
  });
});
