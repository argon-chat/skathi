import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { effectFindings, runCheck } from "../check";
import { ALL_CODES, REGISTRY, RETIRED } from "../check/codes";
import { parseCheckConfig } from "../check/config";
import { explain } from "../check/report";
import { analyzeCss, analyzeFrameworks, analyzeRules, analyzeUnit, FIXTURES } from "./check-fixtures";

const CHECK_DIR = join(import.meta.dir, "..", "check");

describe("the code registry", () => {
  test("codes are unique", () => {
    const codes = ALL_CODES.map((c) => c.code);
    expect(new Set(codes).size).toBe(codes.length);
  });

  test("a retired code is not reused, stays valid in check.allow, and sk explain says it is retired", () => {
    for (const code of RETIRED.keys()) expect(REGISTRY.has(code)).toBe(false);
    expect(parseCheckConfig({ allow: { SK5005: ["some-package"] } }).problems).toEqual([]);
    expect(explain("sk5005")).toContain("retired");
  });

  test("every code the checker emits is registered", () => {
    const emitted = new Set<string>();
    for (const name of readdirSync(CHECK_DIR).filter((f) => f.endsWith(".ts") && f !== "codes.ts")) {
      for (const match of readFileSync(join(CHECK_DIR, name), "utf8").matchAll(/"(SK\d{4})"/g)) emitted.add(match[1]!);
    }
    expect(emitted.size).toBeGreaterThan(10);
    for (const code of emitted) expect(REGISTRY.has(code)).toBe(true);
  });

  test("every registered code has a fixture that triggers it", async () => {
    const seen = new Set<string>();
    const add = (codes: { code: string }[]) => codes.forEach((d) => seen.add(d.code));
    add(analyzeRules().diagnostics);
    add(analyzeCss().diagnostics);
    add(analyzeFrameworks().diagnostics);
    add(analyzeUnit(["broken.ts", "missing.ts"]).diagnostics);
    for (const app of ["check-badconfig", "check-unresolved"]) {
      add((await runCheck({ target: join(FIXTURES, app), toolVersion: "test" })).diagnostics);
    }
    const native = mkdtempSync(join(FIXTURES, "check-native-"));
    try {
      mkdirSync(join(native, "src"));
      writeFileSync(join(native, "sk.config.json"), '{"svelte":{"renderer":"native"}}');
      writeFileSync(join(native, "src/main.ts"), 'import App from "./App.svelte"; console.log(App)');
      writeFileSync(join(native, "src/App.svelte"), '<script>let x = $state("");</script>\n<input bind:value={x} />');
      const check = () => runCheck({ target: native, toolVersion: "test" });
      const incompatible = (await check()).diagnostics;
      expect(incompatible).toContainEqual(
        expect.objectContaining({
          code: "SK5015",
          file: "src/App.svelte",
          range: expect.objectContaining({ start: { line: 1, character: 7 } }),
        }),
      );
      add(incompatible);
      const old = join(native, "node_modules/svelte");
      mkdirSync(old, { recursive: true });
      writeFileSync(
        join(old, "package.json"),
        '{"name":"svelte","type":"module","exports":{"./compiler":"./compiler.js"}}',
      );
      writeFileSync(join(old, "compiler.js"), 'export const VERSION = "5.58.0"');
      const version = (await check()).diagnostics;
      expect(version).toContainEqual(expect.objectContaining({ code: "SK5016" }));
      add(version);
    } finally {
      rmSync(native, { recursive: true, force: true });
    }
    const shaders = join(FIXTURES, "check-shaders");
    add(await effectFindings(shaders));
    const saved = process.env.SK_REDSLANGC;
    process.env.SK_REDSLANGC = join(shaders, "no-redslangc");
    try {
      add(await effectFindings(shaders));
    } finally {
      if (saved === undefined) delete process.env.SK_REDSLANGC;
      else process.env.SK_REDSLANGC = saved;
    }
    expect([...REGISTRY.keys()].filter((code) => !seen.has(code))).toEqual([]);
  }, 60_000);
});
