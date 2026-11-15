// The sk-devtools command table has two consumers, `ctl.ts` (the CLI) and `mcp.ts` (the MCP server), both generated
// from `devtools/commands.ts`. This file asserts the generated surface is complete and that neither front-end has
// grown a command list of its own.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildRequest, splitArgs, commandNames, mcpTools, toolName, UsageError } from "../devtools/commands";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEVTOOLS = resolve(HERE, "..", "devtools");
const read = (path: string): string => readFileSync(path, "utf8");

describe("the table itself", () => {
  test("every command is declared once", () => {
    const names = commandNames();
    expect(new Set(names).size).toBe(names.length);
    expect(names.length).toBeGreaterThan(0);
  });
});

describe("MCP parity", () => {
  const tools = mcpTools();

  test("build() emits the wire cmd and drops what the caller left out", () => {
    const rules = tools.find((t) => t.name === "sk_rules")!;
    expect(rules.build({ selector: ".tab", index: 2 })).toEqual({
      cmd: "rules",
      selector: ".tab",
      index: 2,
    });
    expect(rules.build({ node: 41 })).toEqual({ cmd: "rules", node: 41 });
    expect(rules.build({ selector: ".tab", nonsense: 1 })).toEqual({
      cmd: "rules",
      selector: ".tab",
    });
  });

  test("`window` rides through on the MCP path too", () => {
    const windows = tools.find((t) => t.name === "sk_windows")!;
    expect(windows.build({})).toEqual({ cmd: "windows" });
    const perf = tools.find((t) => t.name === "sk_perf")!;
    expect(perf.build({ window: 4294967298 })).toEqual({ cmd: "perf", window: 4294967298 });
  });
});

describe("CLI parity", () => {
  test("actual mouse argv preserves held and shift switches without consuming following flags", () => {
    const { positional, flags } = splitArgs([
      "--json",
      "mouse",
      "--x",
      "35",
      "--y",
      "45",
      "--kind",
      "move",
      "--held",
      "--shift",
    ]);
    expect(flags.json).toBe(true);
    expect(buildRequest(positional, flags)).toEqual({
      cmd: "mouse",
      x: 35,
      y: 45,
      kind: "move",
      held: true,
      shift: true,
    });
  });

  test("wheel modifiers and IME commit use their declared boolean types", () => {
    const wheel = splitArgs(["wheel", ".list", "--dy", "10", "--ctrl", "--alt"]);
    expect(buildRequest(wheel.positional, wheel.flags)).toMatchObject({ cmd: "wheel", ctrl: true, alt: true, dy: 10 });
    const ime = splitArgs(["ime", "input", "text", "--commit"]);
    expect(buildRequest(ime.positional, ime.flags)).toMatchObject({ cmd: "ime", commit: true });
  });

  test("the two front-ends produce the same request for the same command", () => {
    const tools = mcpTools();
    const cases: [string[], Record<string, string | boolean>, Record<string, unknown>][] = [
      [["query", ".tab"], {}, { cmd: "query", selector: ".tab" }],
      [["rules", ".tab"], { index: "2" }, { cmd: "rules", selector: ".tab", index: 2 }],
      [["perf"], {}, { cmd: "perf" }],
      [["windows"], {}, { cmd: "windows" }],
      [["raise"], {}, { cmd: "raise" }],
      [["net"], { since: "12" }, { cmd: "net", since: 12 }],
      [["wheel", ".list"], { dy: "200" }, { cmd: "wheel", selector: ".list", dx: 0, dy: 200 }],
      [
        ["storage", "local", "set"],
        { key: "theme", value: "dark" },
        { cmd: "storage", area: "local", op: "set", key: "theme", value: "dark" },
      ],
      [["dialog"], {}, { cmd: "dialog", op: "list" }],
      [
        ["dialog", "answer", "3", "C:\\a.txt", "C:\\b.txt"],
        {},
        { cmd: "dialog", op: "answer", id: 3, paths: "C:\\a.txt|C:\\b.txt" },
      ],
      [["dialog", "cancel"], { id: "4" }, { cmd: "dialog", op: "cancel", id: 4 }],
      [["dialog", "answer", "C:\\a.txt"], { id: "5" }, { cmd: "dialog", op: "answer", id: 5, paths: "C:\\a.txt" }],
    ];
    for (const [argv, flags, wire] of cases) {
      expect(buildRequest(argv, flags)).toEqual(wire);
      const { cmd, ...args } = wire;
      const tool = tools.find((t) => t.name === toolName(cmd as string))!;
      expect(tool.build(args)).toEqual(wire);
    }
  });

  test("an unknown verb is a UsageError, not a thrown TypeError", () => {
    expect(() => buildRequest(["nosuchcommand"], {})).toThrow(UsageError);
    expect(() => buildRequest(["query"], {})).toThrow(UsageError);
    expect(() => buildRequest(["mouse"], {})).toThrow(UsageError);
    expect(() => buildRequest(["dialog", "cancel"], {})).toThrow(UsageError);
    expect(() => buildRequest(["dialog", "answer", "3"], {})).toThrow(UsageError);
  });

  test("`highlight --clear` is the one command whose flag replaces its target", () => {
    expect(buildRequest(["highlight"], { clear: true })).toEqual({ cmd: "highlight", clear: true });
    expect(buildRequest(["highlight", ".card"], { ttl: "3000", label: "this one" })).toEqual({
      cmd: "highlight",
      selector: ".card",
      ttlMs: 3000,
      label: "this one",
    });
  });
});

describe("no front-end may grow a command list of its own", () => {
  const ctl = read(join(DEVTOOLS, "ctl.ts"));
  const mcp = read(join(DEVTOOLS, "mcp.ts"));

  test("neither writes a wire `cmd` down", () => {
    // The drift this file prevents: a `cmd: "..."` literal or a `case "...":` verb in a front-end rather than in commands.ts.
    for (const [name, src] of [
      ["ctl.ts", ctl],
      ["mcp.ts", mcp],
    ] as const) {
      expect(src, `${name} declares a wire cmd`).not.toMatch(/\bcmd:\s*"/);
      expect(src, `${name} switches on a verb`).not.toMatch(/^\s*case\s+"/m);
    }
  });
});

describe("paged recordings and native inspection arguments", () => {
  const parse = (argv: string[]) => {
    const parts = splitArgs(argv);
    return buildRequest(parts.positional, parts.flags);
  };
  test("legacy trace spellings keep their payloads", () => {
    expect(parse(["trace", "start", "--cap", "20000"])).toEqual({ cmd: "trace", op: "start", cap: 20000 });
    expect(parse(["trace", "stop", "--frames"])).toEqual({ cmd: "trace", op: "stop", frames: true });
    expect(parse(["trace", "mark", "drag", "completed"])).toEqual({
      cmd: "trace",
      op: "mark",
      label: "drag completed",
    });
    expect(parse(["trace"])).toEqual({ cmd: "trace", op: "peek" });
  });
  test("ordered trace pages map kebab flags onto the wire without losing zero offsets", () => {
    expect(parse(["trace", "frame", "--frame-id", "42", "--offset", "0", "--limit", "400"])).toEqual({
      cmd: "trace",
      op: "frame",
      frameId: 42,
      offset: 0,
      limit: 400,
    });
    expect(parse(["trace", "frames", "--limit", "240"])).toEqual({ cmd: "trace", op: "frames", limit: 240 });
    expect(parse(["trace", "recording-start", "--cap", "1200"])).toEqual({
      cmd: "trace",
      op: "recording-start",
      cap: 1200,
    });
    expect(parse(["trace", "recording-stop"])).toEqual({ cmd: "trace", op: "recording-stop" });
    expect(() => parse(["trace", "frame"])).toThrow(UsageError);
    expect(() => parse(["trace", "frame", "--frame-id", "1.5"])).toThrow(UsageError);
    expect(() => parse(["trace", "frames", "--limit", "241"])).toThrow(UsageError);
    expect(() => parse(["trace", "frames", "--offset", "garbage"])).toThrow(UsageError);
  });
  test("native picker is optional and bare polling stays compatible", () => {
    expect(parse(["pick"])).toEqual({ cmd: "pick" });
    expect(parse(["picker", "start"])).toEqual({ cmd: "pick", op: "start" });
    expect(parse(["pick", "status"])).toEqual({ cmd: "pick", op: "status" });
    expect(parse(["pick", "stop"])).toEqual({ cmd: "pick", op: "stop" });
    expect(() => parse(["pick", "click"])).toThrow(UsageError);
  });
  test("IndexedDB keys and cursor remain strings, including empty legal names", () => {
    expect(
      parse(["indexeddb", "records", "chat", "messages", "--index", "by-author", "--after", "00ff10", "--limit", "50"]),
    ).toEqual({
      cmd: "indexeddb",
      op: "records",
      database: "chat",
      store: "messages",
      index: "by-author",
      after: "00ff10",
      limit: 50,
    });
    expect(buildRequest(["indexeddb", "schema"], { database: "" })).toEqual({
      cmd: "indexeddb",
      op: "schema",
      database: "",
    });
    expect(() => parse(["indexeddb", "records", "chat"])).toThrow(UsageError);
    expect(() => parse(["indexeddb", "records", "chat", "messages", "--after", "bad-cursor"])).toThrow(UsageError);
    expect(() => parse(["indexeddb", "records", "chat", "messages", "--limit", "101"])).toThrow(UsageError);
    expect(() => parse(["indexeddb", "records", "chat", "messages", "--search", "x".repeat(257)])).toThrow(UsageError);
    expect(parse(["indexeddb", "records", "chat", "messages", "--search", "🙂".repeat(256)]).search).toBe(
      "🙂".repeat(256),
    );
  });
  test("allocation filtering and bounded sampling reject invalid capture scopes", () => {
    expect(parse(["allocations", "start", "--sample-every", "16", "--max-events", "65536"])).toEqual({
      cmd: "allocations",
      op: "start",
      sampleEvery: 16,
      maxEvents: 65536,
    });
    expect(
      parse(["allocations", "snapshot", "--min-size", "0", "--max-size", "4096", "--site", "text", "--limit", "100"]),
    ).toEqual({ cmd: "allocations", op: "snapshot", minSize: 0, maxSize: 4096, site: "text", limit: 100 });
    expect(() => parse(["allocations", "start", "--sample-every", "0"])).toThrow(UsageError);
    expect(() => parse(["allocations", "snapshot", "--min-size", "100", "--max-size", "10"])).toThrow(UsageError);
    expect(() => parse(["allocations", "start", "--max-events", "65537"])).toThrow(UsageError);
  });
});
