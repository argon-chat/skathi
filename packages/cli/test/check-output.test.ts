import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { exitCode, formatPretty } from "../check";
import { formatDiagnostic } from "../check/report";
import type { CheckReport } from "../check/schema";
import { analyzeRules, analyzeUnit, FIXTURES, RULES, sk, UNIT } from "./check-fixtures";

describe("JSON report", () => {
  const report = analyzeRules();

  test("has the versioned shape", () => {
    expect(Object.keys(report)).toEqual(["schema", "version", "tool", "root", "diagnostics", "summary"]);
    expect(report.schema).toBe("skathi.check");
    expect(report.version).toBe(1);
    expect(report.tool).toEqual({ name: "sk", version: "test" });
    expect(report.root).toBe(RULES.split("\\").join("/"));
    expect(Object.keys(report.summary)).toEqual(["errors", "warnings", "suppressed", "files", "packages", "typing"]);
    for (const d of report.diagnostics) {
      expect(Object.keys(d)).toEqual(["code", "severity", "message", "file", "package", "range", "help", "related"]);
      expect(d.file).not.toContain("\\");
      expect(d.file.startsWith("/")).toBe(false);
      for (const p of [d.range.start, d.range.end]) {
        expect(Number.isInteger(p.line) && p.line >= 0).toBe(true);
        expect(Number.isInteger(p.character) && p.character >= 0).toBe(true);
      }
      for (const r of d.related) expect(Object.keys(r)).toEqual(["file", "range", "message"]);
    }
  });

  test("is sorted by file, then position", () => {
    const keys = report.diagnostics.map((d) => [d.file, d.range.start.line, d.range.start.character] as const);
    const sorted = [...keys].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0) || a[1] - b[1] || a[2] - b[2]);
    expect(keys).toEqual(sorted);
  });

  test("positions are zero-based UTF-16 code units", () => {
    const [d] = analyzeUnit(["utf16.ts"]).diagnostics;
    const text = readFileSync(join(UNIT, "utf16.ts"), "utf8");
    expect(d!.code).toBe("SK2001");
    expect(d!.range.start).toEqual({ line: 0, character: text.indexOf("eval") });
    expect(text.indexOf("eval")).toBe([...text.slice(0, text.indexOf("eval"))].length + 1);
  });

  test("an unreadable file is SK0001", () => {
    const d = analyzeUnit(["missing.ts"]).diagnostics[0]!;
    expect(d.code).toBe("SK0001");
    expect(d.file).toBe("missing.ts");
  });
});

describe("pretty output", () => {
  test("rustc-style, without colour", () => {
    const text = formatPretty(analyzeUnit(["fine.ts", "utf16.ts", "broken.ts"]), { color: false });
    expect(text).toBe(
      [
        "warning[SK0002]: this file does not parse, so it was not checked: Expression expected.",
        " --> broken.ts:2:23",
        "  |",
        "2 | export const broken = ;",
        "  |                       ^",
        "  |",
        "  = help: Fix the syntax error; if the file is valid for the bundler, report it as a bug in sk check.",
        "  = see: sk explain SK0002",
        "",
        "warning[SK2001]: `eval` runs interpreted under AOT, without a JIT",
        " --> utf16.ts:1:44",
        "  |",
        '1 | export const run = (face = "😀") => face + eval("1");',
        "  |                                            ^^^^",
        "  |",
        "  = help: Replace eval with ordinary code (a lookup table, JSON.parse, a small parser).",
        "  = see: sk explain SK2001",
        "",
        "0 errors, 2 warnings in 3 files, 0 packages",
        "",
        "typing quality (worst first)",
        "  app  100% typed  0 of 3 use any",
      ].join("\n"),
    );
    expect(text).not.toContain("\x1b[");
  });

  test("names the package, lists related places, and colours only when asked", () => {
    const report = analyzeRules();
    const d = report.diagnostics.find((x) => x.code === "SK1001" && x.package)!;
    const plain = formatDiagnostic(d, readFileSync(join(RULES, d.file), "utf8"), false);
    expect(plain.split("\n").slice(0, 2)).toEqual([
      "error[SK1001]: `Array.prototype.push` is replaced; AOT code cannot change built-in methods",
      " --> node_modules/patchy/index.js:1:1 (patchy@1.2.3)",
    ]);
    expect(formatDiagnostic(d, null, true)).toContain("\x1b[");
    const typing = report.diagnostics.find((x) => x.code === "SK3003" && x.file === "src/typing.ts")!;
    expect(formatDiagnostic(typing, null, false)).toContain("= note: `as any` (src/typing.ts:3:26)");
  });

  test("a code-by-package table once there are many diagnostics, and the typing section", () => {
    const text = formatPretty(analyzeRules(), { color: false });
    expect(text).toMatch(/^code\s+package\s+count$/m);
    expect(text).toMatch(/^SK1001\s+patchy@1\.2\.3\s+1$/m);
    const typing = text.slice(text.indexOf("typing quality (worst first)"));
    expect(typing).toMatch(/^ {2}patchy@1\.2\.3\s+untyped\s+no declarations$/m);
    expect(typing).toMatch(/^ {2}loose-types@2\.0\.0\s+50% typed\s+2 of 4 use any$/m);
  });
});

describe("exit codes", () => {
  const base: CheckReport = {
    schema: "skathi.check",
    version: 1,
    tool: { name: "sk", version: "test" },
    root: "/",
    diagnostics: [],
    summary: { errors: 0, warnings: 0, suppressed: 0, files: 0, packages: 0, typing: [] },
  };
  test("0 clean, 1 errors or denied warnings, 2 fatal", () => {
    expect(exitCode(base, true)).toBe(0);
    const warned = { ...base, summary: { ...base.summary, warnings: 1 } };
    expect(exitCode(warned, false)).toBe(0);
    expect(exitCode(warned, true)).toBe(1);
    expect(exitCode({ ...base, summary: { ...base.summary, errors: 1 } }, false)).toBe(1);
    const fatal = analyzeRules().diagnostics[0]!;
    expect(exitCode({ ...base, diagnostics: [{ ...fatal, code: "SK0004" }] }, false)).toBe(2);
  });
});

describe("the sk CLI", () => {
  test("sk check on an app with errors prints pure JSON and exits 1", () => {
    const r = sk(["check", RULES, "--format", "json"]);
    expect(r.status).toBe(1);
    const report = JSON.parse(r.stdout) as CheckReport;
    expect(report.schema).toBe("skathi.check");
    expect(report.diagnostics.some((d) => d.file === "src/builtins.ts" && d.code === "SK1001")).toBe(true);
  }, 60_000);

  test("code the production bundle keeps behind a false guard, or calls only from there, is not reported", () => {
    const r = sk(["check", join(FIXTURES, "check-guards"), "--format", "json"]);
    const report = JSON.parse(r.stdout) as CheckReport;
    expect(
      report.diagnostics.filter((d) => d.code === "SK1001").map((d) => `${d.file}:${d.range.start.line + 1}`),
    ).toEqual(["node_modules/guarded/index.js:11"]);
  }, 60_000);

  test("warnings exit 0, or 1 with --deny-warnings", () => {
    const app = join(FIXTURES, "check-warn");
    const plain = sk(["check", app]);
    expect(plain.status).toBe(0);
    expect(plain.stdout).toContain("warning[SK2001]");
    expect(plain.stdout).toContain("0 errors, 1 warning in 1 file, 0 packages");
    expect(sk(["check", app, "--deny-warnings"]).status).toBe(1);
  }, 60_000);

  test("configuration, build and usage failures exit 2", () => {
    const bad = sk(["check", join(FIXTURES, "check-badconfig"), "--format", "json"]);
    expect(bad.status).toBe(2);
    const report = JSON.parse(bad.stdout) as CheckReport;
    expect(report.diagnostics.map((d) => [d.code, d.range.start.line])).toEqual([
      ["SK0003", 5],
      ["SK0003", 6],
    ]);
    const unresolved = sk(["check", join(FIXTURES, "check-unresolved"), "--format", "json"]);
    expect(unresolved.status).toBe(2);
    const d = (JSON.parse(unresolved.stdout) as CheckReport).diagnostics[0]!;
    expect([d.code, d.file, d.range.start.line]).toEqual(["SK0004", "src/main.ts", 0]);
    expect(d.message).toContain("a-package-that-is-not-installed");
    expect(sk(["check", RULES, "--format", "xml"]).status).toBe(2);
    expect(sk(["check", join(FIXTURES, "no-such-app")]).status).toBe(2);
  }, 60_000);

  test("sk explain and sk --help", () => {
    const one = sk(["explain", "sk1001"]);
    expect(one.status).toBe(0);
    expect(one.stdout).toContain("SK1001");
    const list = sk(["explain", "--list"]);
    expect(list.status).toBe(0);
    for (const code of ["SK0001", "SK1004", "SK2004", "SK3004"]) expect(list.stdout).toContain(code);
    expect(sk(["explain", "SK9999"]).status).toBe(2);
    expect(sk(["check", "--help"]).stdout).toContain("--deny-warnings");
  }, 60_000);
});
