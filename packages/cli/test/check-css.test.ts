import { describe, expect, test } from "bun:test";
import { runCheck } from "../check";
import { declarationProblem, selectorProblems } from "../check/css";
import type { CheckReport } from "../check/schema";
import { analyzeCss, CSS_APP } from "./check-fixtures";

let cached: CheckReport | undefined;
const report = () => (cached ??= analyzeCss());

/** CSS diagnostics of one file as `line:column:code`, one-based. */
const css = (r: CheckReport, file: string) =>
  r.diagnostics
    .filter((d) => d.file === file && d.code.startsWith("SK4"))
    .map((d) => `${d.range.start.line + 1}:${d.range.start.character + 1}:${d.code}`);

describe("CSS the engine does not apply (SK4xxx)", () => {
  test("unsupported properties, values, selectors and at-rules in a stylesheet", () => {
    expect(css(report(), "src/main.css")).toEqual([
      "3:3:SK4001",
      "4:3:SK4002",
      "6:1:SK4004",
      "11:1:SK4004",
      "16:1:SK4003",
    ]);
    expect(css(report(), "src/more.css")).toEqual(["1:1:SK4003"]);
    expect(css(report(), "index.html")).toEqual(["5:7:SK4004"]);
  });

  test("component styles and style attributes are reported at their place in the file", () => {
    expect(css(report(), "src/App.vue")).toEqual(["6:40:SK4001", "13:3:SK4002", "15:1:SK4003"]);
    expect(css(report(), "src/Widget.svelte")).toEqual(["5:11:SK4001", "9:5:SK4002"]);
  });

  test("supported CSS, defaults, var() and @supports fallbacks are not reported", () => {
    expect(css(report(), "src/fine.css")).toEqual([]);
  });

  test("a value is refused only as the catalog refuses it", () => {
    const code = (property: string, value: string) => declarationProblem(property, value)?.code ?? null;
    expect(code("transform", "translateX(1px) rotateY(3deg)")).toBe("SK4002");
    expect(code("transform", "translate3d(0, 0, 0) scale(2)")).toBeNull();
    expect(code("-webkit-backdrop-filter", "saturate(180%) blur(8px)")).toBe("SK4002");
    expect(code("filter", "url(#shadow)")).toBeNull();
    expect(code("filter", "drop-shadow(0 1px 2px red)")).toBeNull();
    expect(code("width", "FIT-CONTENT")).toBe("SK4002");
    expect(code("float", "inherit")).toBeNull();
    expect(code("-webkit-text-size-adjust", "100%")).toBeNull();
    expect(code("-moz-appearance", "none")).toBeNull();
  });

  test("selectors: only what the matcher cannot answer", () => {
    const count = (selector: string) => selectorProblems(selector).length;
    expect(count(".a:not(:hover)")).toBe(1);
    expect(count("li:nth-child(2 of .x)")).toBe(1);
    expect(count("input:-webkit-autofill")).toBe(1);
    expect(count("p::first-line, p:first-letter")).toBe(2);
    expect(count(".a:hover:not(.b), .c:is(.d) .e:focus-within")).toBe(0);
    expect(count('[data-state="open"]:hover, .peer-open\\:block, input::-moz-placeholder')).toBe(0);
  });

  test("check.allow suppresses by glob and by package name", () => {
    expect(css(report(), "src/legacy/old.css")).toEqual([]);
    expect(report().diagnostics.some((d) => d.package?.name === "quiet-ui")).toBe(false);
    const unfiltered = analyzeCss({ allow: new Map() });
    expect(css(unfiltered, "src/legacy/old.css")).toEqual(["2:3:SK4001"]);
    expect(unfiltered.diagnostics.filter((d) => d.package?.name === "quiet-ui").map((d) => d.code)).toEqual(["SK4005"]);
  });

  test("a dependency's CSS is one SK4005 per package, and nothing without deps", () => {
    const fancy = report().diagnostics.filter((d) => d.package?.name === "fancy-ui");
    expect(fancy.map((d) => d.code)).toEqual(["SK4005"]);
    expect(fancy[0]!.related.map((r) => r.range.start.line + 1)).toEqual([2, 4]);
    expect(analyzeCss({ deps: false }).diagnostics.filter((d) => d.package)).toEqual([]);
  });

  test("sk check finds the CSS the build includes: imports, @imports and index.html", async () => {
    const r = await runCheck({ target: CSS_APP, toolVersion: "test" });
    const files = new Set(r.diagnostics.filter((d) => d.code.startsWith("SK4")).map((d) => d.file));
    expect([...files].sort()).toEqual([
      "index.html",
      "node_modules/fancy-ui/style.css",
      "src/App.vue",
      "src/main.css",
      "src/more.css",
    ]);
  }, 60_000);
});
