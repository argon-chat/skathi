import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { effectFindings, runCheck } from "../check";
import { ALL_CODES, REGISTRY } from "../check/codes";
import { analyzeCss, analyzeRules, analyzeUnit, FIXTURES } from "./check-fixtures";

const CHECK_DIR = join(import.meta.dir, "..", "check");

describe("the code registry", () => {
  test("codes are unique", () => {
    const codes = ALL_CODES.map((c) => c.code);
    expect(new Set(codes).size).toBe(codes.length);
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
    add(analyzeUnit(["broken.ts", "missing.ts"]).diagnostics);
    for (const app of ["check-badconfig", "check-unresolved"]) {
      add((await runCheck({ target: join(FIXTURES, app), toolVersion: "test" })).diagnostics);
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
