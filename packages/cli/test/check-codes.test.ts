import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { runCheck } from "../check";
import { ALL_CODES, RANGES, REGISTRY } from "../check/codes";
import { analyzeRules, analyzeUnit, FIXTURES } from "./check-fixtures";

const CHECK_DIR = join(import.meta.dir, "..", "check");

describe("the code registry", () => {
  test("codes are unique, well-formed and inside their range", () => {
    const codes = ALL_CODES.map((c) => c.code);
    expect(new Set(codes).size).toBe(codes.length);
    for (const info of ALL_CODES) {
      expect(info.code).toMatch(/^SK\d{4}$/);
      const n = Number(info.code.slice(2));
      const range = RANGES.find((r) => n >= r.from && n <= r.to);
      expect(range).toBeDefined();
      if (n >= 1000 && n < 2000) expect(info.severity).toBe("error");
      if (n >= 2000) expect(info.severity).toBe("warning");
      for (const text of [info.title, info.explanation, info.example, info.fix])
        expect(text.trim().length).toBeGreaterThan(0);
    }
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
    add(analyzeUnit(["broken.ts", "missing.ts"]).diagnostics);
    for (const app of ["check-badconfig", "check-unresolved"]) {
      add((await runCheck({ target: join(FIXTURES, app), toolVersion: "test" })).diagnostics);
    }
    expect([...REGISTRY.keys()].filter((code) => !seen.has(code))).toEqual([]);
  }, 60_000);
});
