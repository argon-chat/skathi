import { registration } from "./registration";
// build.ts: the dependency graph it records and the transport shape the Rust host reads. Runs the real
// builder against test/fixtures/* into a temp dist.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createLoader } from "../../runtime/test/loader-harness";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_DIR = resolve(HERE, "..");
const FIXTURES = join(HERE, "fixtures");

const tmpDirs: string[] = [];

interface Manifest {
  rev: number;
  vendorRev: number;
  entry: string;
  modules: string[];
  deps: Record<string, string[]>;
}
interface Changes {
  changed: { id: string; kind: string }[];
}
interface LazyNs {
  __esModule?: boolean;
  [Symbol.toStringTag]: string;
  default: { name: string };
}
interface EsmMain {
  app: { name: string; parts: string[] };
  ns: { default: (x: string) => string; answer: number };
  eagerNames: Record<string, string>;
  lazyKinds: Record<string, () => Promise<{ name: string }>>;
  lazy: () => Promise<LazyNs>;
  sameInstance: boolean;
  json: {
    config: unknown;
    title: string;
    meta: unknown;
    aliasedJson: unknown;
  };
  raw: string;
  texts: Record<string, string>;
  worker: new () => unknown;
  lazyWorker: () => Promise<{ [Symbol.toStringTag]: string; default: unknown }>;
}
interface NpmMain {
  injected: string;
  registered: string;
  keys: { app: symbol; old: symbol };
  env: string;
  edited: number;
}
/** Every `bun build.ts --watch` this file starts, killed at the end and on runner exit (a test's `finally` does not run on timeout). */
const watchers: Bun.Subprocess[] = [];
async function watchBuild(entry: string, dist: string) {
  const log = Bun.file(join(dist, "watch.log"));
  const proc = Bun.spawn(["bun", "build.ts", entry, dist, "--watch"], {
    cwd: PKG_DIR,
    stdout: log,
    stderr: "inherit",
    env: { ...process.env, SK_OWNER_PID: String(process.pid) },
  });
  watchers.push(proc);
  for (let i = 0; i < 150 && proc.exitCode === null; i++) {
    if ((await log.text()).includes("watching")) return proc;
    await Bun.sleep(100);
  }
  throw new Error("watcher did not start:\n" + (await log.text()));
}
const killWatchers = () => {
  for (const proc of watchers) {
    try {
      proc.kill();
    } catch {}
  }
};
process.on("exit", killWatchers);
afterAll(async () => {
  killWatchers();
  await Promise.all(watchers.map((proc) => proc.exited.catch(() => {})));
  for (const dir of tmpDirs) {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {}
  }
});

function build(fixture: string) {
  const dist = mkdtempSync(join(tmpdir(), "sk-build-"));
  tmpDirs.push(dist);
  const entry = join(FIXTURES, fixture, "src/main.ts");
  const proc = Bun.spawnSync(["bun", "build.ts", entry, dist], { cwd: PKG_DIR });
  if (proc.exitCode !== 0) {
    throw new Error(`build.ts exited ${proc.exitCode}\n${proc.stdout.toString()}\n${proc.stderr.toString()}`);
  }
  const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8")) as Manifest;
  const changes = JSON.parse(readFileSync(join(dist, "changes.json"), "utf8")) as Changes;
  return {
    dist,
    manifest,
    changes,
    stderr: proc.stderr.toString(),
    script: (id: string) => readFileSync(join(dist, "modules", id.split("/").join("__") + ".js"), "utf8"),
  };
}

const id = (fixture: string, rel: string) =>
  ["packages/cli/test/fixtures", fixture, "src", rel].filter(Boolean).join("/");

describe("manifest", () => {
  const out = build("graph-app");
  const ENTRY = id("graph-app", "main.ts");
  const APP = id("graph-app", "App.vue");
  const COUNTER = id("graph-app", "components/Counter.vue");
  const UTIL = id("graph-app", "util.ts");
  const LIB = id("graph-app", "lib/index.ts");

  test("keeps the fields the Rust host reads", () => {
    expect(typeof out.manifest.rev).toBe("number");
    expect(typeof out.manifest.vendorRev).toBe("number");
    expect(out.manifest.entry).toBe(ENTRY);
    expect(out.manifest.modules.slice().sort()).toEqual([APP, COUNTER, LIB, ENTRY, UTIL].sort());
    expect(Object.keys(out.changes).sort()).toEqual(["changed", "rev", "vendorRev"]);
    expect(out.changes.changed).toEqual([]);
  });

  test("records one edge list per module", () => {
    expect(Object.keys(out.manifest.deps).sort()).toEqual(out.manifest.modules.slice().sort());
    expect(out.manifest.deps[ENTRY]).toEqual([APP]);
    expect(out.manifest.deps[APP]!.slice().sort()).toEqual([COUNTER, UTIL].sort());
    expect(out.manifest.deps[COUNTER]).toEqual([UTIL]);
    expect(out.manifest.deps[UTIL]).toEqual([LIB]); // "./lib" -> "lib/index.ts"
    expect(out.manifest.deps[LIB]).toEqual([]);
  });

  test("leaves vendor specifiers out of the graph", () => {
    for (const list of Object.values(out.manifest.deps)) {
      expect(list).not.toContain("vue");
      expect(list).not.toContain("@skathi-native/runtime");
    }
    expect(out.script(ENTRY)).toContain('"spec":"@skathi-native/runtime"');
    expect(out.script(COUNTER)).toContain('"spec":"vue"');
  });

  test("builds cleanly — no unresolved-import advisories", () => {
    expect(out.stderr).not.toContain("resolves to no built module");
  });
});

describe("app-owned framework versions", () => {
  test("dev resolves the browser ESM exports of Vue Router", async () => {
    const app = mkdtempSync(join(FIXTURES, "router-exports-"));
    const dist = mkdtempSync(join(tmpdir(), "sk-router-"));
    tmpDirs.push(app, dist);
    mkdirSync(join(app, "src"));
    writeFileSync(
      join(app, "src/main.ts"),
      'import { createRouter } from "vue-router"; export const ready = typeof createRouter === "function";',
    );
    const out = Bun.spawnSync([process.execPath, "build.ts", join(app, "src/main.ts"), dist], { cwd: PKG_DIR });
    expect(out.exitCode, out.stderr.toString()).toBe(0);
    const manifest: Manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8"));
    const { SK } = createLoader();
    for (const id of manifest.modules)
      new Function("__SK__", readFileSync(join(dist, "modules", id.replaceAll("/", "__") + ".js"), "utf8"))(SK);
    expect((await SK.importModule(manifest.entry)).ready).toBe(true);
  });

  test("dev vendor and prod use the declared app package instead of a different runtime major", () => {
    const dir = mkdtempSync(join(FIXTURES, "framework-app-"));
    const dist = mkdtempSync(join(tmpdir(), "sk-framework-"));
    tmpDirs.push(dir, dist);
    mkdirSync(join(dir, "src"), { recursive: true });
    mkdirSync(join(dir, "node_modules/pinia"), { recursive: true });
    writeFileSync(join(dir, "package.json"), JSON.stringify({ dependencies: { pinia: "3.0.4" } }));
    writeFileSync(
      join(dir, "node_modules/pinia/package.json"),
      JSON.stringify({ name: "pinia", version: "3.0.4", main: "index.js" }),
    );
    writeFileSync(join(dir, "node_modules/pinia/index.js"), 'exports.createPinia = () => "app-owned-pinia-three";');
    writeFileSync(
      join(dir, "src/main.ts"),
      'import { createPinia } from "pinia"; globalThis.__frameworkProbe = createPinia();',
    );
    const dev = Bun.spawnSync(["bun", "build.ts", join(dir, "src/main.ts"), dist], { cwd: PKG_DIR });
    expect(dev.exitCode).toBe(0);
    expect(readFileSync(join(dist, "vendor.js"), "utf8")).not.toContain("createPinia");
    expect(readFileSync(join(dist, "modules", "pinia.js"), "utf8")).toContain("app-owned-pinia-three");
    const prod = Bun.spawnSync(["bun", "prod.ts", join(dir, "src/main.ts"), join(dist, "app.prod.js")], {
      cwd: PKG_DIR,
    });
    expect(prod.exitCode).toBe(0);
    expect(readFileSync(join(dist, "app.prod.js"), "utf8")).toContain("app-owned-pinia-three");
    expect(prod.stderr.toString()).not.toContain("different majors");
  });
});

// An app is not bound by the repo's no-`.js` rule: a ported Vite app may bring a tree of it.
describe("a JavaScript app still builds", () => {
  const dir = mkdtempSync(join(FIXTURES, "js-app-"));
  tmpDirs.push(dir);
  const rel = "packages/cli/test/fixtures/" + dir.split(/[\\/]/).pop();

  test("resolves an extensionless import to a .js file and graphs it", () => {
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(join(dir, "src/util.js"), 'export const greet = (who) => "hello " + who;\n');
    writeFileSync(
      join(dir, "src/main.js"),
      'import { createApp, h } from "@skathi-native/runtime";\n' +
        'import { greet } from "./util";\n' +
        'createApp({ render: () => h("div", null, greet("js")) }).mount();\n',
    );

    const dist = mkdtempSync(join(tmpdir(), "sk-build-js-"));
    tmpDirs.push(dist);
    const proc = Bun.spawnSync(["bun", "build.ts", join(dir, "src/main.js"), dist], {
      cwd: PKG_DIR,
    });
    expect(proc.stderr.toString()).not.toContain("resolves to no built module");
    expect(proc.exitCode).toBe(0);

    const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8"));
    expect(manifest.entry).toBe(`${rel}/src/main.js`);
    expect(manifest.modules).toContain(`${rel}/src/util.js`);
    expect(manifest.deps[`${rel}/src/main.js`]).toEqual([`${rel}/src/util.js`]);
  });
});

describe("import.meta in a prod bundle", () => {
  const dir = mkdtempSync(join(FIXTURES, "meta-app-"));
  tmpDirs.push(dir);

  test("evaluates to Vite's production values, and a mention in a string is not mistaken for one", () => {
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(
      join(dir, "src/main.ts"),
      "const { MODE } = import.meta.env;\n" +
        "const parts = [import.meta.env.MODE, import.meta.env.PROD, import.meta.env.DEV, MODE, import.meta.url, typeof import.meta.hot];\n" +
        'const note = "import.meta is only valid inside modules";\n' +
        "(globalThis as any).__metaProbe = parts.join('|') + '|' + note.length;\n",
    );
    const out = join(mkdtempSync(join(tmpdir(), "sk-prod-meta-")), "app.prod.js");
    tmpDirs.push(dirname(out));
    const proc = Bun.spawnSync(["bun", "prod.ts", join(dir, "src/main.ts"), out], { cwd: PKG_DIR });
    if (proc.exitCode !== 0) throw new Error(`prod.ts exited ${proc.exitCode}\n${proc.stderr.toString()}`);
    const bundle = readFileSync(out, "utf8");
    new Function(bundle)();
    expect((globalThis as any).__metaProbe).toBe("production|true|false|production|sk://app.prod.js|undefined|40");
  });

  test("the dev builder substitutes an OBJECT, not a string, for import.meta.env", () => {
    const dist = mkdtempSync(join(tmpdir(), "sk-build-meta-"));
    tmpDirs.push(dist);
    const proc = Bun.spawnSync(["bun", "build.ts", join(dir, "src/main.ts"), dist], { cwd: PKG_DIR });
    expect(proc.exitCode).toBe(0);
    const rel = "packages/cli/test/fixtures/" + dir.split(/[\\/]/).pop() + "/src/main.ts";
    const script = readFileSync(join(dist, "modules", rel.split("/").join("__") + ".js"), "utf8");
    expect(script).not.toContain("({MODE:");
    expect(registration(script)[1]).toMatch(/MODE:\s*"development"/);
  });
});

describe("module scripts", () => {
  const out = build("graph-app");
  const APP = id("graph-app", "App.vue");
  const COUNTER = id("graph-app", "components/Counter.vue");
  const UTIL = id("graph-app", "util.ts");

  test("carry the graph next to the factory, guarded for old vendors", () => {
    const script = out.script(APP);
    expect(script).toContain(
      `__SK__.setDeps && __SK__.setDeps(${JSON.stringify(APP)}, ${JSON.stringify([COUNTER, UTIL])});`,
    );
    expect(script.indexOf("__SK__.setDeps")).toBeLessThan(script.indexOf("__SK__.define"));
  });

  test("close define() with a body hash", () => {
    for (const module of [APP, COUNTER, UTIL]) {
      expect(registration(out.script(module))[3]).toMatch(/^[0-9a-f]+$/);
    }
  });

  test("still emit setCss before define for a styled SFC", () => {
    const script = out.script(APP);
    expect(script.indexOf("__SK__.setCss")).toBeLessThan(script.indexOf("__SK__.setDeps"));
    expect(script).toContain("data-v-");
  });
});

describe("style-only edits", () => {
  const file = join(FIXTURES, "style-app/src/App.vue");
  const original = readFileSync(file, "utf8");
  const APP = id("style-app", "App.vue");
  const hashOf = (script: string) => registration(script)[3];
  const cssOf = (script: string) => /__SK__\.setCss\([^,]+, (".*")\);/.exec(script)![1];

  test("change the css but not the body hash", () => {
    const before = build("style-app").script(APP);
    let after: string;
    try {
      writeFileSync(file, original.replace("color: red;", "color: rebeccapurple;"));
      after = build("style-app").script(APP);
    } finally {
      writeFileSync(file, original);
    }
    expect(cssOf(after)).not.toBe(cssOf(before));
    expect(cssOf(after)).toContain("rebeccapurple");
    expect(hashOf(after)).toBe(hashOf(before));
  }, 30_000);

  test("a template edit DOES change the body hash", () => {
    const before = build("style-app").script(APP);
    let after: string;
    try {
      writeFileSync(file, original.replace("{{ label }}", "{{ label }}!"));
      after = build("style-app").script(APP);
    } finally {
      writeFileSync(file, original);
    }
    expect(hashOf(after)).not.toBe(hashOf(before));
  }, 30_000);
});

describe("css entry modules", () => {
  const CSS_ID = id("css-app", "theme.css");
  const MAIN = id("css-app", "main.ts");

  test("the stylesheet becomes a css-only synthetic module", () => {
    const b = build("css-app");
    expect(b.manifest.modules).toContain(CSS_ID);
    const script = b.script(CSS_ID);
    expect(script).toContain("__SK__.setCss");
    expect(script).toContain("--fixture-fg");
    expect(registration(script)[1]).toBe("");
  });

  test("it is a graph edge of the module that imported it, not an npm package", () => {
    const b = build("css-app");
    expect(b.manifest.deps[MAIN]).toContain(CSS_ID);
    expect(b.stderr).not.toContain("cannot resolve");
    expect(b.stderr).not.toContain("resolves to no built module");
  });

  test("prod.ts puts the same CSS in the same place", () => {
    const out = join(mkdtempSync(join(tmpdir(), "sk-prod-")), "app.prod.js");
    tmpDirs.push(dirname(out));
    const entry = join(FIXTURES, "css-app/src/main.ts");
    const proc = Bun.spawnSync(["bun", "prod.ts", entry, out], { cwd: PKG_DIR });
    if (proc.exitCode !== 0) {
      throw new Error(`prod.ts exited ${proc.exitCode}
${proc.stderr.toString()}`);
    }
    const bundle = readFileSync(out, "utf8");
    expect(bundle).toContain("--fixture-fg");
    expect(bundle).not.toContain(".fixture-card { color");
  });

  test("stylesheets sort ahead of the app's own modules", () => {
    const b = build("css-app");
    const modules = b.manifest.modules;
    expect(modules.indexOf(CSS_ID)).toBeLessThan(modules.indexOf(id("css-app", "App.vue")));
  });
});

describe('a "type": "module" app, in dev', () => {
  const out = build("esm-app");
  const MAIN = id("esm-app", "main.ts");
  const world = () => {
    const w = createLoader();
    for (const mod of out.manifest.modules) {
      new Function("__SK__", "__VUE_HMR_RUNTIME__", out.script(mod))(w.SK, w.sandbox.__VUE_HMR_RUNTIME__);
    }
    return w;
  };

  test("a default import is the module's default export, as in Vite", async () => {
    expect(out.script(MAIN)).not.toMatch(/__toESM\(require\([^)]*\),\s*1\)/);
    const main = (await world().SK.importModule(MAIN)) as unknown as EsmMain;
    expect(main.app.name).toBe("EsmApp");
    expect(main.ns.default("x")).toBe("hello x");
    expect(main.ns.answer).toBe(42);
  });

  test("import.meta.glob expands as in prod, in a module and in an SFC, and stays in the graph", async () => {
    const main = (await world().SK.importModule(MAIN)) as unknown as EsmMain;
    expect(main.eagerNames).toEqual({ "./kinds/a.ts": "a", "./kinds/b.ts": "b" });
    expect(Object.keys(main.lazyKinds)).toEqual(["./kinds/a.ts"]);
    expect((await main.lazyKinds["./kinds/a.ts"]!()).name).toBe("a");
    expect(main.app.parts).toEqual(["./kinds/a.ts", "./kinds/b.ts"]);
    expect(out.manifest.deps[MAIN]).toContain(id("esm-app", "kinds/a.ts"));
    expect(out.manifest.deps[id("esm-app", "App.vue")]).toContain(id("esm-app", "kinds/b.ts"));
  });

  test("import() resolves through the registry, and defineAsyncComponent unwraps it", async () => {
    expect(out.script(MAIN)).toContain('"dynamic":true');
    expect(out.manifest.deps[MAIN]).toContain(id("esm-app", "Lazy.vue"));
    const ns = await ((await world().SK.importModule(MAIN)) as unknown as EsmMain).lazy();
    expect(!!(ns && (ns.__esModule || ns[Symbol.toStringTag] === "Module"))).toBe(true);
    expect(ns.default.name).toBe("LazyPane");
  });

  const ALIAS = id("esm-app", "alias.ts");
  const UTIL = id("esm-app", "util.ts");

  test("a tsconfig paths alias is the module's relative path: a graph edge and one instance", async () => {
    expect(out.script(ALIAS)).toContain('"spec":"./util.ts"');
    expect(out.script(ALIAS)).not.toContain("@/");
    expect(out.manifest.deps[ALIAS]).toEqual([UTIL]);
    expect(out.manifest.modules.some((m: string) => m.startsWith("@/"))).toBe(false);
    expect((await world().SK.importModule(MAIN))["sameInstance"]).toBe(true);
  });

  test("JSON in src/, outside it and through an alias, as Vite gives it", async () => {
    const { json } = (await world().SK.importModule(MAIN)) as unknown as EsmMain;
    expect(json.config).toEqual({ title: "esm", list: [1, 2] });
    expect(json.title).toBe("esm");
    expect(json.meta).toEqual({ outside: "src" });
    expect(json.aliasedJson).toEqual({ title: "esm", list: [1, 2] });
    expect(out.stderr).not.toContain("resolves to no built module");
  });

  test("?raw and a query glob give the text; ?worker is the stub prod gives", async () => {
    const main = (await world().SK.importModule(MAIN)) as unknown as EsmMain;
    expect(main.raw).toBe("a note, read with ?raw\n");
    expect(main.texts).toEqual({ "./data/note.txt": "a note, read with ?raw\n" });
    expect(() => new main.worker()).toThrow();
    const ns = await main.lazyWorker();
    expect(ns[Symbol.toStringTag]).toBe("Module");
    expect(ns.default).toBe(main.worker);
  });
});

// Shared package entries: a package's own npm imports are separate modules, not inlined copies (a duplicated reka-ui breaks injection and registries).
describe("build.ts (dev): npm packages share the packages they import", () => {
  const dir = mkdtempSync(join(FIXTURES, "npm-share-"));
  tmpDirs.push(dir);
  const rel = "packages/cli/test/fixtures/" + dir.split(/[\\/]/).pop();
  const MAIN = `${rel}/src/main.ts`;
  const put = (path: string, text: string) => {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  };
  const pkg = (path: string, name: string, version: string, code: string) => {
    put(`${path}/package.json`, JSON.stringify({ name, version, type: "module", exports: "./index.js" }));
    put(`${path}/index.js`, code);
  };
  put("package.json", JSON.stringify({ name: "npm-share-fixture", private: true }));
  const core = (tag: string) =>
    `export const KEY = Symbol(${JSON.stringify(tag)});\n` +
    "export const formatters = new Map();\n" +
    "export const provide = (scope, value) => ((scope[KEY] = value), scope);\n" +
    "export const inject = (scope) => scope[KEY];\n";
  pkg("node_modules/ctx-core", "ctx-core", "2.0.0", core("ctx"));
  // A workspace package, linked as the client's @argon/* are: its realpath is outside node_modules.
  pkg(
    "packages/ctx-provider",
    "ctx-provider",
    "1.0.0",
    'import { provide, formatters } from "ctx-core";\n' +
      'formatters.set("IArgonEvent", "fmt");\n' +
      "export const providing = (value) => provide({}, value);\n",
  );
  mkdirSync(join(dir, "node_modules"), { recursive: true });
  symlinkSync(join(dir, "packages/ctx-provider"), join(dir, "node_modules/ctx-provider"), "junction");
  pkg(
    "node_modules/ctx-consumer",
    "ctx-consumer",
    "1.0.0",
    'import { inject } from "ctx-core";\nexport const injecting = (scope) => inject(scope);\n',
  );
  pkg("node_modules/old-user", "old-user", "1.0.0", 'import { KEY } from "ctx-core";\nexport const oldKey = KEY;\n');
  pkg("node_modules/old-user/node_modules/ctx-core", "ctx-core", "1.0.0", core("old"));
  put(
    "node_modules/env-pick/package.json",
    JSON.stringify({
      name: "env-pick",
      version: "1.0.0",
      exports: { ".": { node: "./node.cjs", default: "./browser.cjs" } },
    }),
  );
  put("node_modules/env-pick/node.cjs", 'exports.which = "node";\n');
  put("node_modules/env-pick/browser.cjs", 'exports.which = "browser";\n');
  put(
    "src/main.ts",
    'import { providing } from "ctx-provider";\n' +
      'import { injecting } from "ctx-consumer";\n' +
      'import { formatters, KEY } from "ctx-core";\n' +
      'import { oldKey } from "old-user";\n' +
      'import { which } from "env-pick";\n' +
      "export const env = which;\n" +
      'export const injected = injecting(providing("ctx-value"));\n' +
      'export const registered = formatters.get("IArgonEvent");\n' +
      "export const keys = { app: KEY, old: oldKey };\n",
  );

  const dist = mkdtempSync(join(tmpdir(), "sk-build-npm-"));
  tmpDirs.push(dist);
  const read = (f: string) => {
    try {
      return readFileSync(join(dist, f), "utf8");
    } catch {
      return "";
    }
  };
  const script = (id: string) => read(join("modules", id.split("/").join("__") + ".js"));
  const until = async (ok: () => boolean) => {
    for (let i = 0; i < 150 && !ok(); i++) await Bun.sleep(100);
    return ok();
  };
  const world = (manifest: Manifest) => {
    const w = createLoader();
    for (const mod of manifest.modules) {
      new Function("__SK__", "__VUE_HMR_RUNTIME__", script(mod))(w.SK, w.sandbox.__VUE_HMR_RUNTIME__);
    }
    return w;
  };

  test("one instance across packages and the app; another version stays its own; an HMR pass keeps it", async () => {
    const proc = await watchBuild(join(dir, "src/main.ts"), dist);
    try {
      const manifest = JSON.parse(read("manifest.json")) as Manifest;
      const npm = manifest.modules.filter((m: string) => !m.startsWith(rel));
      expect(npm.filter((m: string) => m === "ctx-core")).toEqual(["ctx-core"]);
      expect(npm.filter((m: string) => m.startsWith("ctx-core~"))).toHaveLength(1); // old-user's nested 1.0.0
      expect(script("ctx-provider")).toContain('"spec":"ctx-core"');
      expect(script("ctx-provider")).not.toContain('Symbol("ctx")');

      const w = world(manifest);
      const main = (await w.SK.importModule(MAIN)) as unknown as NpmMain;
      expect(main.injected).toBe("ctx-value"); // provided in one package, injected in another
      expect(main.registered).toBe("fmt"); // registered by a package, read by the app
      expect(main.keys.old).not.toBe(main.keys.app);
      expect(main.keys.old.description).toBe("old");
      expect(main.env).toBe("browser");
      const core = await w.SK.importModule("ctx-core");

      const coreScript = script("ctx-core");
      writeFileSync(
        join(dir, "src/main.ts"),
        readFileSync(join(dir, "src/main.ts"), "utf8") + "export const edited = 1;\n",
      );
      expect(await until(() => script(MAIN).includes("edited"))).toBe(true);
      // The module is written before changes.json.
      expect(await until(() => read("changes.json").includes(MAIN))).toBe(true);
      const changes = JSON.parse(read("changes.json")) as Changes;
      expect(changes.changed).toContainEqual({ id: MAIN, kind: "reload" });
      expect(changes.changed.filter((c: { id: string }) => npm.includes(c.id))).toEqual([]);
      expect(script("ctx-core")).toBe(coreScript);
      new Function("__SK__", "__VUE_HMR_RUNTIME__", script("ctx-core"))(w.SK, w.sandbox.__VUE_HMR_RUNTIME__);
      new Function("__SK__", "__VUE_HMR_RUNTIME__", script(MAIN))(w.SK, w.sandbox.__VUE_HMR_RUNTIME__);
      expect(await w.SK.importModule("ctx-core")).toBe(core);
      const again = (await w.SK.importModule(MAIN)) as unknown as NpmMain;
      expect(again.edited).toBe(1);
      expect(again.injected).toBe("ctx-value");
    } finally {
      proc.kill();
      await proc.exited;
    }
  }, 30000);
});

// Entries of one package that share module state through relative imports (`svelte` and `svelte/internal/client`).
describe("build.ts (dev): two entries of one package share its modules", () => {
  const dir = mkdtempSync(join(FIXTURES, "npm-group-"));
  tmpDirs.push(dir);
  const rel = "packages/cli/test/fixtures/" + dir.split(/[\\/]/).pop();
  const MAIN = `${rel}/src/main.ts`;
  const put = (path: string, text: string) => {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  };
  put("package.json", JSON.stringify({ name: "npm-group-fixture", private: true }));
  put(
    "node_modules/multi/package.json",
    JSON.stringify({
      name: "multi",
      version: "1.0.0",
      type: "module",
      exports: { ".": "./index.js", "./client": "./client.js", "./extra": "./extra.js", "./legacy": "./legacy.cjs" },
      imports: { "#state": "./state.js" },
    }),
  );
  put("node_modules/multi/state.js", "export const registry = new Map();\n");
  put(
    "node_modules/multi/index.js",
    'import { registry } from "#state";\nexport const register = (k, v) => registry.set(k, v);\n',
  );
  put(
    "node_modules/multi/client.js",
    'import { registry } from "./state.js";\nexport const lookup = (k) => registry.get(k);\n',
  );
  put("node_modules/multi/extra.js", 'import { lookup } from "multi/client";\nexport default () => lookup("k");\n');
  put("node_modules/multi/legacy.cjs", 'module.exports = function legacy() { return "cjs"; };\n');
  put(
    "node_modules/uses-runtime/package.json",
    JSON.stringify({ name: "uses-runtime", version: "1.0.0", type: "module", exports: "./index.js" }),
  );
  put("node_modules/uses-runtime/index.js", 'export { mountDocument } from "@skathi-native/runtime";\n');
  put(
    "src/main.ts",
    'import { register } from "multi";\n' +
      'import { lookup } from "multi/client";\n' +
      'import legacy from "multi/legacy";\n' +
      'import { mountDocument } from "uses-runtime";\n' +
      'register("k", "shared");\n' +
      'export const found = lookup("k");\n' +
      "export const cjs = legacy();\n" +
      "export const mount = mountDocument;\n",
  );

  const dist = mkdtempSync(join(tmpdir(), "sk-build-group-"));
  tmpDirs.push(dist);
  const read = (f: string) => {
    try {
      return readFileSync(join(dist, f), "utf8");
    } catch {
      return "";
    }
  };
  const script = (id: string) => read(join("modules", id.split("/").join("__") + ".js"));
  const until = async (ok: () => boolean) => {
    for (let i = 0; i < 150 && !ok(); i++) await Bun.sleep(100);
    return ok();
  };

  test("one copy of the package's modules; a new entry rebuilds it and asks for a restart", async () => {
    const proc = await watchBuild(join(dir, "src/main.ts"), dist);
    try {
      const manifest = JSON.parse(read("manifest.json")) as Manifest;
      const group = manifest.modules.filter((m) => m.startsWith("multi~pkg~"));
      expect(group).toHaveLength(1);
      expect(script(group[0]!).match(/new Map\b/g)).toHaveLength(1);
      expect(script("multi")).toContain(JSON.stringify(group[0]));
      expect(script("uses-runtime")).toContain('"spec":"@skathi-native/runtime"');

      const w = createLoader();
      for (const mod of manifest.modules) {
        new Function("__SK__", "__VUE_HMR_RUNTIME__", script(mod))(w.SK, w.sandbox.__VUE_HMR_RUNTIME__);
      }
      const main = (await w.SK.importModule(MAIN)) as unknown as { found: string; cjs: string };
      expect(main.found).toBe("shared");
      expect(main.cjs).toBe("cjs");

      writeFileSync(
        join(dir, "src/main.ts"),
        readFileSync(join(dir, "src/main.ts"), "utf8") + 'export { default as extra } from "multi/extra";\n',
      );
      expect(await until(() => read("changes.json").includes("restart"))).toBe(true);
      const changes = JSON.parse(read("changes.json")) as Changes;
      expect(changes.changed).toContainEqual({ id: "multi", kind: "restart" });
      expect(changes.changed).toContainEqual({ id: "multi/extra", kind: "reload" });
      expect(script(group[0]!).match(/new Map\b/g)).toHaveLength(1);
    } finally {
      proc.kill();
      await proc.exited;
    }
  }, 30000);
});

describe("build.ts (dev): an edit to an inlined file rebuilds its importer", () => {
  const dir = mkdtempSync(join(FIXTURES, "inline-app-"));
  tmpDirs.push(dir);
  const rel = "packages/cli/test/fixtures/" + dir.split(/[\\/]/).pop() + "/src/main.ts";

  test("a .json the module inlined", async () => {
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(join(dir, "src/data.json"), '{ "v": 1 }\n');
    writeFileSync(join(dir, "src/main.ts"), 'import data from "./data.json";\nexport const v = data.v;\n');
    const dist = mkdtempSync(join(tmpdir(), "sk-build-inline-"));
    tmpDirs.push(dist);
    const proc = await watchBuild(join(dir, "src/main.ts"), dist);
    const read = (f: string) => {
      try {
        return readFileSync(join(dist, f), "utf8");
      } catch {
        return "";
      }
    };
    const until = async (ok: () => boolean) => {
      for (let i = 0; i < 300 && !ok(); i++) await Bun.sleep(100);
      return ok();
    };
    try {
      const script = () => read(join("modules", rel.split("/").join("__") + ".js"));
      expect(script()).toMatch(/v:\s*1\b/);
      writeFileSync(join(dir, "src/data.json"), '{ "v": 2 }\n');
      expect(await until(() => /v:\s*2\b/.test(script()))).toBe(true);
      expect(await until(() => read("changes.json").includes(rel))).toBe(true);
      const changes = JSON.parse(read("changes.json")) as Changes;
      expect(changes.changed).toContainEqual({ id: rel, kind: "reload" });
    } finally {
      proc.kill();
      await proc.exited;
    }
  }, 60_000);
});

describe("module namespaces in a prod bundle", () => {
  const dir = mkdtempSync(join(FIXTURES, "ns-app-"));
  tmpDirs.push(dir);

  test("import() and import * are Module-tagged, null-prototype namespaces", async () => {
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(join(dir, "src/pane.ts"), 'export default { name: "Pane" };\nexport const extra = 1;\n');
    writeFileSync(
      join(dir, "src/main.ts"),
      'import * as all from "./pane";\n' + '(globalThis as any).__nsProbe = { all, lazy: () => import("./pane") };\n',
    );
    const out = join(mkdtempSync(join(tmpdir(), "sk-prod-ns-")), "app.prod.js");
    tmpDirs.push(dirname(out));
    const proc = Bun.spawnSync(["bun", "prod.ts", join(dir, "src/main.ts"), out], { cwd: PKG_DIR });
    if (proc.exitCode !== 0) throw new Error(`prod.ts exited ${proc.exitCode}\n${proc.stderr.toString()}`);
    new Function(readFileSync(out, "utf8"))();
    const probe = (globalThis as any).__nsProbe;
    const lazy = await probe.lazy();
    for (const ns of [probe.all, lazy]) {
      expect(Object.prototype.toString.call(ns)).toBe("[object Module]");
      expect(Object.getPrototypeOf(ns)).toBe(null);
      expect(ns.default.name).toBe("Pane");
      expect(ns.extra).toBe(1);
    }
    expect(lazy).toBe(probe.all);
  });
});

test("an app outside the checkout gets ids relative to its own root", async () => {
  const app = mkdtempSync(join(tmpdir(), "sk-outside-"));
  tmpDirs.push(app);
  mkdirSync(join(app, "src"));
  writeFileSync(join(app, "package.json"), '{ "name": "outside", "private": true }\n');
  writeFileSync(join(app, "src/util.ts"), "export const answer = 42;\n");
  writeFileSync(join(app, "src/main.ts"), 'import { answer } from "./util";\nconsole.log(answer);\n');
  const dist = join(app, "dist");
  const proc = Bun.spawnSync(["bun", "build.ts", join(app, "src/main.ts"), dist], { cwd: PKG_DIR });
  expect(proc.exitCode).toBe(0);
  const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8")) as Manifest;
  expect(manifest.entry).toBe("src/main.ts");
  expect(manifest.modules.sort()).toEqual(["src/main.ts", "src/util.ts"]);
  expect(readFileSync(join(dist, "modules", "src__util.ts.js"), "utf8")).toContain("42");
});
