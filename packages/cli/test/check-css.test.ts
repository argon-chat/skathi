import { afterAll, describe, expect, test } from "bun:test";
import { appendFileSync, cpSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runCheck } from "../check";
import { cssFindings, fileFindings } from "../check/css";
import { Packages } from "../check/packages";
import type { CheckReport } from "../check/schema";
import { buildApp } from "../cli/build-app";
import { packageDir } from "../runtime-path";
import { analyzeCss, CSS_APP } from "./check-fixtures";

const scratch: string[] = [];
const watchers: Bun.Subprocess[] = [];
afterAll(async () => {
  for (const w of watchers) w.kill();
  await Promise.all(watchers.map((w) => w.exited));
  for (const dir of scratch) rmSync(dir, { recursive: true, force: true });
});
const tempDir = (prefix: string) => {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  scratch.push(dir);
  return dir;
};

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
    const code = (property: string, value: string) =>
      fileFindings("sample.css", `.box { ${property}: ${value} }`, false)[0]?.code ?? null;
    expect(code("transform", "translateX(1px) rotateY(3deg)")).toBe("SK4002");
    expect(code("transform", "translate3d(0, 0, 0) scale(2)")).toBeNull();
    expect(code("-webkit-backdrop-filter", "saturate(180%) blur(8px)")).toBe("SK4002");
    expect(code("filter", "url(#shadow)")).toBeNull();
    expect(code("filter", "drop-shadow(0 1px 2px red)")).toBe("SK4006");
    expect(code("width", "FIT-CONTENT")).toBe("SK4002");
    expect(code("float", "--side(left)")).toBe("SK4001");
    expect(code("float", "inherit")).toBeNull();
    expect(code("-webkit-text-size-adjust", "100%")).toBeNull();
    expect(code("-moz-appearance", "none")).toBeNull();
    expect(code("font-smoothing", "antialiased")).toBeNull();
    expect(code("-webkit-font-smoothing", "antialiased")).toBe("SK4008");
    expect(code("font-smoothing", "none")).toBe("SK4002");
    expect(code("line-metrics", "snapped")).toBeNull();
    expect(code("line-leading", "above")).toBeNull();
    expect(code("line-leading", "center")).toBe("SK4002");
    expect(code("font-size-adjust", "from-font")).toBeNull();
    expect(code("font-size-adjust", "cap-height 0.7")).toBeNull();
    expect(code("font-size-adjust", "ic-height 1")).toBe("SK4002");
  });

  test("content-sized fields and metric-based block trimming use the native CSS grammar", () => {
    const checks = (css: string) => fileFindings("fields.css", css, false).map((d) => d.code);
    expect(checks("textarea { field-sizing: content } p { text-box: trim-both cap alphabetic }")).toEqual([]);
    expect(checks("p { text-box-edge: ideographic } input { field-sizing: auto }")).toEqual(["SK4002", "SK4002"]);
  });

  test("starting styles, discrete transitions and sibling functions reach the native CSS parser", () => {
    const checks = (css: string) => fileFindings("motion.css", css, false).map((d) => d.code);
    expect(
      checks(`
      .toast {
        transition: opacity 150ms, display 150ms allow-discrete;
        transition-behavior: normal, allow-discrete;
        transition-delay: calc(sibling-index() * 40ms);
        rotate: calc(360deg / sibling-count() * sibling-index());
        @starting-style { opacity: 0; }
      }
      @starting-style { .dialog { opacity: 0; translate: 0 6px; } }
    `),
    ).toEqual([]);
    expect(checks(".toast { transition-behavior: auto; opacity: sibling-index(2) }")).toEqual(["SK4002", "SK4002"]);
  });

  test("plus-lighter blends view transition snapshots", () => {
    const check = (selector: string, value = "plus-lighter") =>
      fileFindings("fade.css", `${selector} { mix-blend-mode: ${value} }`, false).map((d) => d.code);
    expect(check("::view-transition-old(card), ::view-transition-new(card)")).toEqual([]);
    expect(check("::view-transition-group(card)")).toEqual(["SK4006"]);
    expect(check(".card, ::view-transition-old(card)")).toEqual(["SK4006"]);
    expect(check(".card", "normal")).toEqual([]);
    expect(check(".card", "multiply")).toEqual(["SK4002"]);
  });

  test("@container: only queries the engine cannot evaluate", () => {
    const containerProblem = (prelude: string) =>
      fileFindings("layout.css", `@container ${prelude} { p { color: red } }`, false)[0] ?? null;
    expect(containerProblem("card (width >= 24rem) and style(--mode: 50%)")).toBeNull();
    expect(containerProblem("(orientation: portrait) or (min-height: 10em)")).toBeNull();
    expect(containerProblem("scroll-state(stuck: top)")).not.toBeNull();
    expect(containerProblem("style(color: red)")).not.toBeNull();
    expect(containerProblem("(min-width: 50%)")).not.toBeNull();
  });

  test("selectors: only what the matcher cannot answer", () => {
    const count = (selector: string) => fileFindings("selector.css", `${selector} { color: red }`, false).length;
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
      "src/compat.css",
      "src/main.css",
      "src/more.css",
    ]);
  }, 60_000);

  test("all eleven reported patterns point to their source declaration or selector", () => {
    expect(css(report(), "src/compat.css")).toEqual([
      "1:1:SK4004",
      "7:3:SK4007",
      "9:1:SK4007",
      "14:3:SK4007",
      "17:3:SK4002",
      "20:3:SK4007",
      "23:3:SK4006",
      "26:3:SK4006",
      "29:3:SK4006",
      "32:3:SK4007",
      "35:3:SK4008",
    ]);
  });

  test("a host-version advisory is for an app whose runtime range admits an older host", () => {
    const codes = (range: string) => {
      const root = tempDir("sk-check-floor-");
      writeFileSync(join(root, "package.json"), JSON.stringify({ dependencies: { "@skathi-native/runtime": range } }));
      const sources = [{ file: join(root, "a.css"), text: ".a { position: relative } .b { float: left }" }];
      return cssFindings({ sources, packages: new Packages(root), deps: true, tailwind: false }).map((f) => f.code);
    };
    expect(codes("^0.5.0")).toEqual(["SK4007", "SK4001"]);
    expect(codes("^0.7.0")).toEqual(["SK4001"]);
    expect(codes(">=0.6.2")).toEqual(["SK4001"]);
    expect(codes("^1.0.0")).toEqual(["SK4001"]);
    expect(codes("workspace:*")).toEqual(["SK4007", "SK4001"]);
  });

  test("source ranges in static Svelte SVG styles count UTF-16 code units", () => {
    const source = '<p>🌷</p><svg><stop style="stop-color: var(--tone)"/></svg>';
    const [stop] = fileFindings("Logo.svelte", source, false);
    expect(stop?.code).toBe("SK4007");
    expect(stop?.range.start.character).toBe(source.indexOf("stop-color"));
  });

  test("the conditional advisories respect explicit paint and layout", () => {
    const source =
      ".card { filter: drop-shadow(0 1px 2px black); background: red } svg { filter: drop-shadow(0 1px 2px black) } .sized { display:inline-flex; width:10px }";
    expect(fileFindings("explicit.css", source, false)).toEqual([]);
    expect(fileFindings("Icon.svelte", '<svg style="filter:drop-shadow(0 1px 2px black)"/>', false)).toEqual([]);
  });

  test("Svelte global and Vue scoped selectors are checked as the framework emits them", () => {
    expect(fileFindings("Card.svelte", "<style>:global(.card:hover) .title { color: red }</style>", false)).toEqual([]);
    expect(fileFindings("Card.vue", "<style scoped>.card :deep(.title) { color: red }</style>", false)).toEqual([]);
    expect(
      fileFindings("Card.svelte", "<style>:global(.card:lang(en)) { color: red }</style>", false).map((d) => d.code),
    ).toEqual(["SK4003"]);
  });
});

describe("CSS warnings in sk build and sk dev", () => {
  test("sk build warns about the CSS it bundles, keeps check.allow, and succeeds", async () => {
    const lines: string[] = [];
    const result = await buildApp({
      target: CSS_APP,
      out: tempDir("sk-css-build-"),
      payloadOnly: true,
      log: (l) => lines.push(l),
    });
    expect(result.skpk).toBeTruthy();
    const warned = lines.filter((l) => /^warning: \S+:\d+:\d+ SK4/.test(l));
    expect(warned).toContain(
      "warning: src/main.css:3:3 SK4001 `float` is not supported: this declaration has no effect",
    );
    expect(warned.filter((l) => l.includes(" SK4005 ")).map((l) => l.split(":")[1]!.trim())).toEqual([
      "node_modules/fancy-ui/style.css",
    ]);
    expect(warned.some((l) => l.includes("src/legacy/old.css"))).toBe(false);
  }, 60_000);

  test("sk dev reports a changed file's new warning once", async () => {
    const app = tempDir("sk-css-dev-");
    cpSync(CSS_APP, app, { recursive: true });
    rmSync(join(app, "src/Widget.svelte"));
    symlinkSync(packageDir("vue", import.meta.dir)!, join(app, "node_modules/vue"), "junction");
    const proc = Bun.spawn(
      [process.execPath, join(import.meta.dir, "../build.ts"), join(app, "src/main.ts"), join(app, "dist"), "--watch"],
      {
        cwd: join(import.meta.dir, ".."),
        stdout: "pipe",
        stderr: "pipe",
        env: { ...process.env, NO_COLOR: "1", SK_OWNER_PID: String(process.pid) },
      },
    );
    watchers.push(proc);
    let out = "";
    void (async () => {
      const decoder = new TextDecoder();
      for await (const chunk of proc.stdout) out += decoder.decode(chunk);
    })();
    void (async () => {
      const decoder = new TextDecoder();
      for await (const chunk of proc.stderr) out += decoder.decode(chunk);
    })();
    const until = async (what: string) => {
      for (const deadline = Date.now() + 30_000; !out.includes(what); await Bun.sleep(50))
        if (Date.now() > deadline) throw new Error(`no "${what}" in:\n${out}`);
    };
    const count = (what: string) => out.split(what).length - 1;
    await until("watching");
    expect(count("src/main.css:3:3 SK4001")).toBe(1);

    appendFileSync(join(app, "src/fine.css"), ".added {\n  float: right;\n}\n");
    await until("rev 2");
    await until("src/fine.css:70:3 SK4001");
    appendFileSync(join(app, "src/fine.css"), ".again {\n  color: red;\n}\n");
    await until("rev 3");
    appendFileSync(join(app, "src/main.css"), ".more {\n  color: blue;\n}\n");
    await until("rev 4");
    await Bun.sleep(200);
    expect(count("src/fine.css:70:3 SK4001")).toBe(1);
    expect(count("src/main.css:3:3 SK4001")).toBe(1);
  }, 90_000);
});
