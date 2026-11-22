import { describe, expect, test } from "bun:test";
import type { CheckReport } from "../check/schema";
import { parseCheckConfig } from "../check/config";
import { analyzeRules, analyzeUnit, lines, only } from "./check-fixtures";

let cached: CheckReport | undefined;
const report = () => (cached ??= analyzeRules());

describe("SK1xxx: what breaks under AOT", () => {
  test("built-in members replaced, deleted or added to a locked object (SK1001)", () => {
    const sk1001 = lines(report(), "src/builtins.ts").filter((l) => l.endsWith("SK1001"));
    expect(sk1001).toEqual([
      "1:SK1001",
      "4:SK1001",
      "5:SK1001",
      "6:SK1001",
      "7:SK1001",
      "9:SK1001",
      "22:SK1001",
      "23:SK1001",
      "23:SK1001",
      "25:SK1001",
      "25:SK1001",
    ]);
  });

  test("redefinition through defineProperty, defineProperties, assign and Reflect (SK1002)", () => {
    expect(lines(report(), "src/builtins.ts").filter((l) => l.endsWith("SK1002"))).toEqual([
      "11:SK1002",
      "12:SK1002",
      "13:SK1002",
      "14:SK1002",
    ]);
  });

  test("rewired prototypes (SK1003) and replaced or deleted globals (SK1004)", () => {
    const l = lines(report(), "src/builtins.ts");
    expect(l.filter((x) => x.endsWith("SK1003"))).toEqual(["16:SK1003", "17:SK1003"]);
    expect(l.filter((x) => x.endsWith("SK1004"))).toEqual(["19:SK1004", "20:SK1004", "21:SK1004"]);
  });

  test("the reported range is the assignment target", () => {
    const push = only(report(), "SK1001").find((d) => d.file === "src/builtins.ts" && d.range.start.line === 0)!;
    expect(push.range).toEqual({ start: { line: 0, character: 0 }, end: { line: 0, character: 20 } });
    expect(push.package).toBeNull();
  });

  test("user classes, shadowed names, new members, polyfill guards, restores and replaceable built-ins are not reported", () => {
    expect(lines(report(), "src/clean.ts")).toEqual([]);
    expect(lines(report(), "src/lazy.ts")).toEqual([]);
  });

  test("JavaScript in node_modules is checked and attributed to its package", () => {
    expect(lines(report(), "node_modules/patchy/index.js")).toEqual(["1:SK1001", "13:SK2001"]);
    const d = only(report(), "SK1001").find((x) => x.file === "node_modules/patchy/index.js")!;
    expect(d.package).toEqual({ name: "patchy", version: "1.2.3" });
  });
});

describe("SK2xxx: what runs slower under AOT", () => {
  test("eval, the Function constructor, string timers and computed import()", () => {
    expect(lines(report(), "src/dynamic.ts")).toEqual([
      "1:SK2001",
      "2:SK2001",
      "3:SK2001",
      "4:SK2002",
      "5:SK2002",
      "6:SK2003",
      "7:SK2004",
    ]);
  });

  test("tight loops: a number parameter as the bound (SK2005), indexed stores into a number[] (SK2006)", () => {
    expect(lines(report(), "src/loops.ts")).toEqual(["3:SK2005", "9:SK2005", "15:SK2006", "16:SK2006"]);
  });

  test("positions inside a .vue file point at the SFC itself", () => {
    const d = only(report(), "SK2001").find((x) => x.file === "src/App.vue")!;
    expect(d.range).toEqual({ start: { line: 5, character: 14 }, end: { line: 5, character: 18 } });
  });

  test("a .svelte file is checked through its component script, not the scripts in its markup", () => {
    const [d, ...rest] = analyzeUnit(["Widget.svelte"]).diagnostics;
    expect(rest).toEqual([]);
    expect(d!.code).toBe("SK2001");
    expect(d!.range).toEqual({ start: { line: 2, character: 16 }, end: { line: 2, character: 20 } });
  });
});

describe("SK3xxx: typing quality", () => {
  test("a package without declarations (SK3001)", () => {
    expect(only(report(), "SK3001").map((d) => [d.file, d.package?.name])).toEqual([
      ["node_modules/patchy/package.json", "patchy"],
    ]);
  });

  test("one aggregated diagnostic per package whose declarations use any (SK3002)", () => {
    const [d, ...rest] = only(report(), "SK3002");
    expect(rest).toEqual([]);
    expect(d!.file).toBe("node_modules/loose-types/index.d.ts");
    expect(d!.related.map((r) => r.message)).toEqual(["loose: 2 × `any`", "Options[index]: 1 × `any`"]);
  });

  test("any, implicit any and ts directives in app code, one diagnostic per file (SK3003)", () => {
    const typing = only(report(), "SK3003").find((d) => d.file === "src/typing.ts")!;
    expect(typing.related.map((r) => `${r.range.start.line + 1}:${r.message}`)).toEqual([
      "1:`@ts-ignore`",
      "2:`any`",
      "3:`as any`",
      "6:an implicitly `any` parameter",
    ]);
    const vue = only(report(), "SK3003").find((d) => d.file === "src/App.vue")!;
    expect(vue.related.map((r) => r.message)).toEqual(["an implicitly `any` parameter"]);
  });

  test("shorthand ambient modules and untyped app JavaScript (SK3004)", () => {
    expect(only(report(), "SK3004").map((d) => [d.file, d.related.map((r) => r.file)])).toEqual([
      ["node_modules/hidden-types/package.json", ["src/main.ts"]],
      ["src/plain.js", ["src/main.ts"]],
      ["src/shims.d.ts", ["src/main.ts"]],
    ]);
  });

  test("summary.typing scores the app and each package", () => {
    const typing = report().summary.typing;
    expect(typing.find((s) => s.package === "loose-types")).toEqual({
      package: "loose-types",
      version: "2.0.0",
      declarations: 4,
      any: 2,
    });
    const app = typing.find((s) => s.package === null)!;
    expect(app.declarations).toBeGreaterThan(app.any);
    expect(app.any).toBeGreaterThan(0);
  });
});

describe("SK6xxx: permissions", () => {
  test("dlopen() names are compared with permissions.ffi", () => {
    expect(lines(report(), "src/native.ts")).toEqual(["10:SK6002", "11:SK6003", "12:SK6002"]);
    expect(only(report(), "SK6002").map((d) => d.message.match(/dlopen\("(\w+)"\)/)?.[1])).toEqual(["curl", "zlib"]);
    expect(lines(report(), "node_modules/native-kit/index.js")).toEqual(["1:SK6005"]);
  });

  test("without permissions the import is the error, and check.allow cannot hide it", () => {
    const none = analyzeRules({ permissions: undefined, allow: new Map([["SK6001", ["src/**"]]]) });
    expect(lines(none, "src/native.ts")).toEqual(["1:SK6001"]);
    expect(parseCheckConfig({ allow: { SK6001: ["src/**"] } }).problems).toHaveLength(1);
  });
});

describe("SK7xxx: skathi:numeric", () => {
  test("two different number types in one operator, and not a type with itself, a literal or a plain number", () => {
    expect(lines(report(), "src/numeric.ts")).toEqual(["6:SK7001", "6:SK7001", "6:SK7001", "6:SK7001", "7:SK7001"]);
    expect(only(report(), "SK7001")[0]!.range.start.character).toBe("export const mixed = [".length);
  });
});

describe("suppression and scope", () => {
  test('"check.allow" hides diagnostics by package name and by glob, and counts them', () => {
    expect(report().summary.suppressed).toBe(2);
    expect(lines(report(), "src/legacy/timers.ts")).toEqual([]);
    expect(report().diagnostics.some((d) => d.package?.name === "evil-eval")).toBe(false);
    const all = analyzeRules({ allow: new Map() });
    expect(all.summary.suppressed).toBe(0);
    expect(lines(all, "src/legacy/timers.ts")).toEqual(["1:SK2003"]);
    expect(lines(all, "node_modules/evil-eval/index.js")).toEqual(["1:SK2001"]);
  });

  test("--no-deps checks the app's own code only", () => {
    const own = analyzeRules({ deps: false });
    expect(own.diagnostics.filter((d) => d.package !== null)).toEqual([]);
    expect(own.diagnostics.some((d) => d.code === "SK3001" || d.code === "SK3002")).toBe(false);
    expect(own.summary.packages).toBe(0);
    expect(own.summary.typing.every((s) => s.package === null)).toBe(true);
    expect(lines(own, "src/builtins.ts")).toEqual(lines(report(), "src/builtins.ts"));
  });
});
