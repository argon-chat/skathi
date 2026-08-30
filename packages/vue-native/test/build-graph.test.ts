// build.ts: the dependency graph it records, and the transport shape the Rust
// host reads. Runs the real builder against test/fixtures/* into a temp dist.
//
//   bun test           (from packages/vue-native)

import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_DIR = resolve(HERE, "..");
const WS_ROOT = resolve(PKG_DIR, "../..");
const FIXTURES = join(HERE, "fixtures");

const tmpDirs = [];
afterAll(() => {
  for (const dir of tmpDirs) {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {}
  }
});

/** Run the real builder once; returns the parsed manifest + a script reader. */
function build(fixture) {
  const dist = mkdtempSync(join(tmpdir(), "vn-build-"));
  tmpDirs.push(dist);
  const entry = join(FIXTURES, fixture, "src/main.ts");
  const proc = Bun.spawnSync(["bun", "build.ts", entry, dist], { cwd: PKG_DIR });
  if (proc.exitCode !== 0) {
    throw new Error(
      `build.ts exited ${proc.exitCode}\n${proc.stdout.toString()}\n${proc.stderr.toString()}`,
    );
  }
  const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8"));
  const changes = JSON.parse(readFileSync(join(dist, "changes.json"), "utf8"));
  return {
    dist,
    manifest,
    changes,
    stderr: proc.stderr.toString(),
    script: (id) => readFileSync(join(dist, "modules", id.split("/").join("__") + ".js"), "utf8"),
  };
}

const id = (fixture, rel) =>
  ["packages/vue-native/test/fixtures", fixture, "src", rel].filter(Boolean).join("/");

describe("manifest", () => {
  const out = build("graph-app");
  const ENTRY = id("graph-app", "main.ts");
  const APP = id("graph-app", "App.vue");
  const COUNTER = id("graph-app", "components/Counter.vue");
  const UTIL = id("graph-app", "util.ts");
  const LIB = id("graph-app", "lib/index.ts");

  test("keeps the fields the Rust host reads", () => {
    // crates/vn-app/src/main.rs read_manifest(): vendorRev, entry, modules
    expect(typeof out.manifest.rev).toBe("number");
    expect(typeof out.manifest.vendorRev).toBe("number");
    expect(out.manifest.entry).toBe(ENTRY);
    expect(out.manifest.modules.slice().sort()).toEqual([APP, COUNTER, LIB, ENTRY, UTIL].sort());
    // poll_dev_changes(): rev, vendorRev, changed[].id — unchanged shape
    expect(Object.keys(out.changes).sort()).toEqual(["changed", "rev", "vendorRev"]);
    expect(out.changes.changed).toEqual([]);
  });

  test("records one edge list per module", () => {
    expect(Object.keys(out.manifest.deps).sort()).toEqual(out.manifest.modules.slice().sort());
    expect(out.manifest.deps[ENTRY]).toEqual([APP]);
    expect(out.manifest.deps[APP].slice().sort()).toEqual([COUNTER, UTIL].sort());
    expect(out.manifest.deps[COUNTER]).toEqual([UTIL]);
    expect(out.manifest.deps[UTIL]).toEqual([LIB]); // "./lib" -> "lib/index.ts"
    expect(out.manifest.deps[LIB]).toEqual([]);
  });

  test("leaves vendor specifiers out of the graph", () => {
    for (const list of Object.values(out.manifest.deps)) {
      expect(list).not.toContain("vue");
      expect(list).not.toContain("vue-native");
    }
    // ...even though the modules really do require them
    expect(out.script(ENTRY)).toContain('require("vue-native")');
    expect(out.script(COUNTER)).toContain('require("vue")');
  });

  test("builds cleanly — no unresolved-import advisories", () => {
    expect(out.stderr).not.toContain("resolves to no built module");
  });
});

// This repo writes no JavaScript any more — the fixtures above are `.ts`, like
// everything else. An APP is not bound by that rule: `SOURCE_EXT` and
// `CANDIDATE_EXTS` both still list `.js`, `vn-test` resolves `apps/<x>/src/
// main.js` by default, and someone porting a Vite app will hand us a tree full
// of it. Converting the fixtures therefore took the only coverage that path
// had, so it is rebuilt here — as a temporary tree rather than a checked-in
// one, which is what keeps "no `.js` in this repository" literally true.
describe("a JavaScript app still builds", () => {
  const dir = mkdtempSync(join(FIXTURES, "js-app-"));
  tmpDirs.push(dir);
  const rel = "packages/vue-native/test/fixtures/" + dir.split(/[\\/]/).pop();

  test("resolves an extensionless import to a .js file and graphs it", () => {
    mkdirSync(join(dir, "src"), { recursive: true });
    // No `.vue` and no type annotations: plain ES2015, exactly what a ported
    // Vite entry looks like before anyone touches it.
    writeFileSync(
      join(dir, "src/util.js"),
      'export const greet = (who) => "hello " + who;\n',
    );
    writeFileSync(
      join(dir, "src/main.js"),
      'import { createApp, h } from "vue-native";\n' +
        'import { greet } from "./util";\n' +
        'createApp({ render: () => h("div", null, greet("js")) }).mount();\n',
    );

    const dist = mkdtempSync(join(tmpdir(), "vn-build-js-"));
    tmpDirs.push(dist);
    const proc = Bun.spawnSync(["bun", "build.ts", join(dir, "src/main.js"), dist], {
      cwd: PKG_DIR,
    });
    expect(proc.stderr.toString()).not.toContain("resolves to no built module");
    expect(proc.exitCode).toBe(0);

    const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8"));
    expect(manifest.entry).toBe(`${rel}/src/main.js`);
    expect(manifest.modules).toContain(`${rel}/src/util.js`);
    // `./util` -> `util.js`: the `.js` candidate is still ahead of `.ts` in
    // `CANDIDATE_EXTS`, and both the builder and `loader.ts` must agree on it.
    expect(manifest.deps[`${rel}/src/main.js`]).toEqual([`${rel}/src/util.js`]);
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
      `__VN__.setDeps && __VN__.setDeps(${JSON.stringify(APP)}, ${JSON.stringify([COUNTER, UTIL])});`,
    );
    expect(script.indexOf("__VN__.setDeps")).toBeLessThan(script.indexOf("__VN__.define"));
  });

  test("close define() with a body hash", () => {
    for (const module of [APP, COUNTER, UTIL]) {
      expect(out.script(module)).toMatch(/\n}, "[0-9a-f]+"\);\n?$/);
    }
  });

  test("still emit setCss before define for a styled SFC", () => {
    const script = out.script(APP);
    expect(script.indexOf("__VN__.setCss")).toBeLessThan(script.indexOf("__VN__.setDeps"));
    expect(script).toContain("data-v-");
  });
});

describe("style-only edits", () => {
  const file = join(FIXTURES, "style-app/src/App.vue");
  const original = readFileSync(file, "utf8");
  const APP = id("style-app", "App.vue");
  const hashOf = (script) => /\n}, "([0-9a-f]+)"\);/.exec(script)[1];
  const cssOf = (script) => /__VN__\.setCss\([^,]+, (".*")\);/.exec(script)[1];

  test("change the css but not the body hash", () => {
    const before = build("style-app").script(APP);
    let after;
    try {
      writeFileSync(file, original.replace("color: red;", "color: rebeccapurple;"));
      after = build("style-app").script(APP);
    } finally {
      writeFileSync(file, original);
    }
    expect(cssOf(after)).not.toBe(cssOf(before));
    expect(cssOf(after)).toContain("rebeccapurple");
    // Same hash => the loader's define() keeps the live exports and the
    // component definition Vue's HMR record already owns.
    expect(hashOf(after)).toBe(hashOf(before));
  });

  test("a template edit DOES change the body hash", () => {
    const before = build("style-app").script(APP);
    let after;
    try {
      writeFileSync(file, original.replace("{{ label }}", "{{ label }}!"));
      after = build("style-app").script(APP);
    } finally {
      writeFileSync(file, original);
    }
    expect(hashOf(after)).not.toBe(hashOf(before));
  });
});

// ---------------------------------------------------------------------------
// CSS entries: `import "./theme.css"` from a JS module
// ---------------------------------------------------------------------------
// A Vite app's global stylesheet is imported from its entry module. Before this
// the .css file was emitted verbatim into a JS module body and died at eval
// with `SyntaxError: Invalid character: '@'` — and for a shadcn-shaped app that
// stylesheet is where the whole palette lives.

describe("css entry modules", () => {
  const CSS_ID = id("css-app", "theme.css");
  const MAIN = id("css-app", "main.ts");

  test("the stylesheet becomes a css-only synthetic module", () => {
    const b = build("css-app");
    expect(b.manifest.modules).toContain(CSS_ID);
    const script = b.script(CSS_ID);
    // Real CSS on the setCss transport…
    expect(script).toContain("__VN__.setCss");
    expect(script).toContain("--fixture-fg");
    // …and an EMPTY factory: importing a stylesheet is a side effect, not a
    // module with exports.
    expect(script).toMatch(/__VN__\.define\([^,]+, function \(require, module, exports\) \{\s*\}/);
  });

  test("it is a graph edge of the module that imported it, not an npm package", () => {
    const b = build("css-app");
    expect(b.manifest.deps[MAIN]).toContain(CSS_ID);
    // It must NOT have been mistaken for a bare specifier and bundled.
    expect(b.stderr).not.toContain("cannot resolve");
    expect(b.stderr).not.toContain("resolves to no built module");
  });

  test("prod.ts puts the same CSS in the same place", () => {
    // The two builders must never disagree about what an app's stylesheet is:
    // prod has no module transport, so the .css file goes through a Bun loader
    // that empties the module and routes its CONTENT into the same `cssById`
    // map a component `<style>` uses.
    const out = join(mkdtempSync(join(tmpdir(), "vn-prod-")), "app.prod.js");
    tmpDirs.push(dirname(out));
    const entry = join(FIXTURES, "css-app/src/main.ts");
    const proc = Bun.spawnSync(["bun", "prod.ts", entry, out], { cwd: PKG_DIR });
    if (proc.exitCode !== 0) {
      throw new Error(`prod.ts exited ${proc.exitCode}
${proc.stderr.toString()}`);
    }
    const bundle = readFileSync(out, "utf8");
    expect(bundle).toContain("--fixture-fg");
    // …and the stylesheet is a SIDE EFFECT: its module body is empty, so the
    // `@` at the top of the file never reaches the JS parser.
    expect(bundle).not.toContain('.fixture-card { color');
    expect(proc.stdout.toString()).toContain("css entry");
  });

  test("stylesheets sort ahead of the app's own modules", () => {
    const b = build("css-app");
    const modules = b.manifest.modules;
    // The loader aggregates CSS in first-setCss order, which is manifest order,
    // so a global stylesheet has to be registered before any component <style>.
    expect(modules.indexOf(CSS_ID)).toBeLessThan(modules.indexOf(id("css-app", "App.vue")));
  });
});
