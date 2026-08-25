// build.ts: the dependency graph it records, and the transport shape the Rust
// host reads. Runs the real builder against test/fixtures/* into a temp dist.
//
//   bun test           (from packages/vue-native)

import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
  const entry = join(FIXTURES, fixture, "src/main.js");
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
  const ENTRY = id("graph-app", "main.js");
  const APP = id("graph-app", "App.vue");
  const COUNTER = id("graph-app", "components/Counter.vue");
  const UTIL = id("graph-app", "util.js");
  const LIB = id("graph-app", "lib/index.js");

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
    expect(out.manifest.deps[UTIL]).toEqual([LIB]); // "./lib" -> "lib/index.js"
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

describe("module scripts", () => {
  const out = build("graph-app");
  const APP = id("graph-app", "App.vue");
  const COUNTER = id("graph-app", "components/Counter.vue");
  const UTIL = id("graph-app", "util.js");

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
