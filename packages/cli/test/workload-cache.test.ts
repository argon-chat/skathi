import { afterAll, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  cacheRoots,
  formatInventory,
  formatPrune,
  inventory,
  needed,
  prune,
  type CacheRoots,
} from "../cli/workload-cache";
import { PIN } from "../effects/redslangc";

const scratch = mkdtempSync(join(tmpdir(), "sk-workload-cache-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

const CURRENT = "0.2.0";
let roots: CacheRoots;

function file(path: string, bytes: number) {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, new Uint8Array(bytes));
}

beforeEach(() => {
  const skathi = join(mkdtempSync(join(scratch, "c-")), "skathi");
  roots = {
    workloads: join(skathi, "hosts"),
    shaderCompilers: join(skathi, "redslangc"),
    nativeImages: join(skathi, "native-aot", "v1"),
    legacy: [join(skathi, "aot-toolchain")],
  };
  const w = roots.workloads;
  file(join(w, CURRENT, "standard", "sk-host.exe"), 1000);
  file(join(w, CURRENT, "standard", "host.json"), 10);
  file(join(w, CURRENT, "dev", "sk-host.exe.partial"), 300);
  file(join(w, CURRENT, "aot", "sk-aot.exe"), 500);
  file(join(w, CURRENT, "aot", "compiler.json"), 5);
  file(join(w, "0.1.0", "standard", "sk-host.exe"), 900);
  file(join(w, "0.1.0", "no-devtools", "sk-host.exe"), 800);
  file(join(w, "0.1.0-beta.1", "dev", "sk-host.exe"), 700);
  file(join(w, "stray.partial"), 3);
  file(join(roots.shaderCompilers, PIN.version, "redslangc.exe"), 50);
  file(join(roots.shaderCompilers, "0.0.9", "redslangc.exe"), 40);
  file(join(roots.shaderCompilers, `${PIN.version}.partial`, "LICENSE"), 6);
  file(join(roots.nativeImages!, "a".repeat(64), "app.image"), 200);
  file(join(roots.nativeImages!, "b".repeat(64), "inspector.image"), 100);
  file(join(roots.legacy[0]!, "frontend-x", "bun.tgz"), 4000);
});

const summary = () =>
  inventory(roots, "windows").map((e) => [e.kind, e.version ?? "", e.variant ?? "", e.bytes, e.complete ?? null]);

describe("the inventory", () => {
  test("names every workload and compiler per version, newest first, with sizes and completeness", () => {
    expect(summary()).toEqual([
      ["workload", "0.2.0", "dev", 300, false],
      ["workload", "0.2.0", "standard", 1010, true],
      ["compiler", "0.2.0", "", 505, true],
      ["workload", "0.1.0", "no-devtools", 800, true],
      ["workload", "0.1.0", "standard", 900, true],
      ["workload", "0.1.0-beta.1", "dev", 700, true],
      ["other", "", "", 3, null],
      ["shader-compiler", `${PIN.version}.partial`, "", 6, false],
      ["shader-compiler", PIN.version, "", 50, true],
      ["shader-compiler", "0.0.9", "", 40, true],
      ["native-images", "", "", 300, null],
      ["legacy", "", "", 4000, null],
    ]);
    expect(inventory(roots, "windows").find((e) => e.kind === "native-images")?.items).toBe(2);
  });

  test("an empty or missing cache is an empty inventory", () => {
    const none = join(scratch, "none");
    expect(inventory({ workloads: none, shaderCompilers: none, nativeImages: null, legacy: [] })).toEqual([]);
  });

  test("this version's complete workloads are needed; --keep adds versions, --all needs nothing", () => {
    const entries = inventory(roots, "windows");
    const kept = (options: Parameters<typeof needed>[1]) =>
      entries.filter((e) => needed(e, options)).map((e) => `${e.version}/${e.variant ?? e.kind}`);
    const pinned = `${PIN.version}/shader-compiler`;
    expect(kept({ current: CURRENT })).toEqual(["0.2.0/standard", "0.2.0/compiler", pinned]);
    expect(kept({ current: CURRENT, keep: ["0.1.0"] })).toEqual([
      "0.2.0/standard",
      "0.2.0/compiler",
      "0.1.0/no-devtools",
      "0.1.0/standard",
      pinned,
    ]);
    expect(kept({ current: CURRENT, all: true })).toEqual([]);
  });

  test("the listing marks what this sk uses and says what prune frees", () => {
    const text = formatInventory(inventory(roots, "windows"), roots, { current: CURRENT }, false);
    expect(text).toMatch(/^ {2}0\.2\.0 +standard workload +0\.0 MiB +this sk$/m);
    expect(text).toMatch(/^ {2}0\.2\.0 +dev workload +0\.0 MiB +incomplete$/m);
    expect(text).toContain("native images (2)");
    expect(text).toContain("old AOT toolchain");
    expect(text).not.toContain("\x1b[");
    expect(text).toMatch(/in all; sk prune frees 0\.0 MiB$/);
  });
});

describe("prune", () => {
  test("a dry run removes nothing and reports what would go", () => {
    const options = { current: CURRENT, dryRun: true, roots };
    const result = prune(inventory(roots, "windows"), options);
    expect(result.bytes).toBe(300 + 800 + 900 + 700 + 3 + 6 + 40 + 300 + 4000);
    expect(summary()).toHaveLength(12);
    expect(formatPrune(result, options, false)).toContain("would free");
  });

  test("by default keeps only this version's complete workloads and empties the rest", () => {
    const result = prune(inventory(roots, "windows"), { current: CURRENT, roots });
    expect(result.failed).toEqual([]);
    expect(summary()).toEqual([
      ["workload", "0.2.0", "standard", 1010, true],
      ["compiler", "0.2.0", "", 505, true],
      ["shader-compiler", PIN.version, "", 50, true],
    ]);
    expect(readdirSync(roots.workloads)).toEqual([CURRENT]);
    expect(readdirSync(roots.shaderCompilers)).toEqual([PIN.version]);
    expect(existsSync(roots.nativeImages!)).toBe(true);
    expect(readdirSync(roots.nativeImages!)).toEqual([]);
    expect(existsSync(roots.legacy[0]!)).toBe(false);
    expect(prune(inventory(roots, "windows"), { current: CURRENT, roots }).removed).toEqual([]);
    expect(formatPrune({ removed: [], failed: [], bytes: 0 }, { current: CURRENT }, false)).toBe(
      "nothing to prune; the workloads of 0.2.0 stay",
    );
  });

  test("--keep spares another version; --all spares nothing", () => {
    prune(inventory(roots, "windows"), { current: CURRENT, keep: ["0.1.0"], roots });
    expect(readdirSync(roots.workloads).sort()).toEqual(["0.1.0", CURRENT]);
    prune(inventory(roots, "windows"), { current: CURRENT, all: true, roots });
    expect(readdirSync(roots.workloads)).toEqual([]);
  });
});

test("the cache lives under LOCALAPPDATA on Windows, beside the native image cache", () => {
  const saved = { LOCALAPPDATA: process.env.LOCALAPPDATA, SK_AOT_CACHE_DIR: process.env.SK_AOT_CACHE_DIR };
  try {
    process.env.LOCALAPPDATA = join(scratch, "local");
    delete process.env.SK_AOT_CACHE_DIR;
    const r = cacheRoots("windows");
    expect(r.workloads).toBe(join(scratch, "local", "skathi", "hosts"));
    expect(r.nativeImages).toBe(join(scratch, "local", "skathi", "native-aot", "v1"));
    expect(r.legacy).toEqual([join(scratch, "local", "skathi", "aot-toolchain")]);
    expect(r.shaderCompilers).toBe(join(scratch, "local", "skathi", "redslangc"));
    expect(cacheRoots("linux").nativeImages).toBeNull();
    process.env.SK_AOT_CACHE_DIR = join(scratch, "images");
    expect(cacheRoots("windows").nativeImages).toBe(join(scratch, "images"));
  } finally {
    for (const [k, v] of Object.entries(saved)) {
      if (v === undefined) delete process.env[k];
      else process.env[k] = v;
    }
  }
});
