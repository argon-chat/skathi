// The vn-devtools command table has THREE consumers — `ctl.ts` (the CLI),
// `mcp.ts` (the MCP server) and docs/DEVTOOLS.md — and used to have three
// copies. They drifted: milestone 21 added `windows`, `perf`, `net`, `storage`,
// `wheel`, `raise` and the optional `window` argument to the CLI and to the
// docs, and the MCP server went on exposing the milestone-10 set.
//
// So the table moved into `devtools/commands.ts` and the two front-ends
// generate from it. This file is the lock: it asserts the generated surface is
// complete, that the docs and the code name the same commands, and — the part
// that actually prevents the regression — that neither front-end has grown a
// command list of its own again.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  COMMANDS,
  buildRequest,
  splitArgs,
  commandNames,
  mcpTools,
  toolName,
  usageBlock,
  UsageError,
} from "../devtools/commands";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEVTOOLS = resolve(HERE, "..", "devtools");
const REPO = resolve(HERE, "..", "..", "..");
const read = (path: string): string => readFileSync(path, "utf8");

describe("the table itself", () => {
  test("every command is declared once", () => {
    const names = commandNames();
    expect(new Set(names).size).toBe(names.length);
    expect(names.length).toBeGreaterThan(0);
  });

  test("every command has a description and at least one usage line", () => {
    for (const spec of COMMANDS) {
      expect(spec.description.length).toBeGreaterThan(20);
      expect(spec.usage.length).toBeGreaterThan(0);
      // The usage block is printed verbatim under `commands`, so every line
      // has to be indented into that block.
      for (const line of spec.usage) expect(line.startsWith("  ")).toBe(true);
    }
  });

  test("`ctl --help` lists every command", () => {
    const block = usageBlock();
    for (const cmd of commandNames()) expect(block).toContain(`  ${cmd}`);
  });
});

describe("MCP parity", () => {
  const tools = mcpTools();

  test("one tool per command, named vn_<cmd>", () => {
    expect(tools.map((t) => t.name)).toEqual(commandNames().map(toolName));
  });

  test("the m21 commands are all exposed", () => {
    // Named rather than derived: this is the exact list that was missing, and
    // a test that recomputed it from COMMANDS would have passed back then too.
    for (const cmd of ["windows", "perf", "net", "storage", "wheel", "raise"]) {
      expect(tools.find((t) => t.name === `vn_${cmd}`)).toBeDefined();
    }
  });

  test("every tool takes `window`", () => {
    // docs/DEVTOOLS.md: "Every command also takes an optional `window`".
    for (const tool of tools) {
      expect(tool.inputSchema.properties.window).toBeDefined();
      expect(tool.inputSchema.properties.window.type).toBe("number");
      // …and it is never required — absent means the primary window.
      expect(tool.inputSchema.required ?? []).not.toContain("window");
    }
  });

  test("declared arguments and required arguments match the table", () => {
    for (const spec of COMMANDS) {
      const tool = tools.find((t) => t.name === toolName(spec.cmd))!;
      const declared = Object.keys(spec.args);
      expect(Object.keys(tool.inputSchema.properties)).toEqual([...declared, "window"]);
      const required = declared.filter((n) => spec.args[n].required);
      expect(tool.inputSchema.required ?? []).toEqual(required);
    }
  });

  test("build() emits the wire cmd and drops what the caller left out", () => {
    const rules = tools.find((t) => t.name === "vn_rules")!;
    expect(rules.build({ selector: ".tab", index: 2 })).toEqual({
      cmd: "rules",
      selector: ".tab",
      index: 2,
    });
    // No `selector: undefined`, no `"index": null` reaching the host.
    expect(rules.build({ node: 41 })).toEqual({ cmd: "rules", node: 41 });
    // An argument the tool does not declare is not forwarded.
    expect(rules.build({ selector: ".tab", nonsense: 1 })).toEqual({
      cmd: "rules",
      selector: ".tab",
    });
  });

  test("`window` rides through on the MCP path too", () => {
    const windows = tools.find((t) => t.name === "vn_windows")!;
    expect(windows.build({})).toEqual({ cmd: "windows" });
    const perf = tools.find((t) => t.name === "vn_perf")!;
    expect(perf.build({ window: 4294967298 })).toEqual({ cmd: "perf", window: 4294967298 });
  });
});

describe("CLI parity", () => {
  test("actual mouse argv preserves held and shift switches without consuming following flags", () => {
    const { positional, flags } = splitArgs(["--json", "mouse", "--x", "35", "--y", "45", "--kind", "move", "--held", "--shift"]);
    expect(flags.json).toBe(true);
    expect(buildRequest(positional, flags)).toEqual({ cmd: "mouse", x: 35, y: 45, kind: "move", held: true, shift: true });
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
      // …and the MCP tool for the same command reaches the same wire object
      // from the same arguments.
      const { cmd, ...args } = wire;
      const tool = tools.find((t) => t.name === toolName(cmd as string))!;
      expect(tool.build(args)).toEqual(wire);
    }
  });

  test("an unknown verb is a UsageError, not a thrown TypeError", () => {
    expect(() => buildRequest(["nosuchcommand"], {})).toThrow(UsageError);
    expect(() => buildRequest(["query"], {})).toThrow(UsageError);
    expect(() => buildRequest(["mouse"], {})).toThrow(/--x and --y/);
    expect(() => buildRequest(["dialog", "cancel"], {})).toThrow(UsageError);
    expect(() => buildRequest(["dialog", "answer", "3"], {})).toThrow(/at least one path/);
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

  test("both import the shared table", () => {
    expect(ctl).toContain('from "./commands"');
    expect(mcp).toContain('from "./commands"');
  });

  test("neither writes a wire `cmd` down", () => {
    // The exact shape of the drift this file exists to prevent: a `cmd: "..."`
    // literal (mcp.ts's old tool table) or a `case "...":` verb (ctl.ts's old
    // switch) in a front-end rather than in commands.ts.
    for (const [name, src] of [
      ["ctl.ts", ctl],
      ["mcp.ts", mcp],
    ] as const) {
      expect(src, `${name} declares a wire cmd`).not.toMatch(/\bcmd:\s*"/);
      expect(src, `${name} switches on a verb`).not.toMatch(/^\s*case\s+"/m);
    }
  });

  test("mcp.ts registers the generated table, not a literal one", () => {
    expect(mcp).toContain("mcpTools()");
    expect(mcp).not.toMatch(/name:\s*"vn_/);
  });
});

describe("the docs name the same commands as the code", () => {
  test("docs/DEVTOOLS.md's command table is exactly COMMANDS", () => {
    const md = read(join(REPO, "docs", "DEVTOOLS.md"));
    const section = md.slice(md.indexOf("\n## Commands"));
    const table = section.slice(0, section.indexOf("\n### "));
    const documented = [...table.matchAll(/^\| `([a-z_]+)` \|/gm)].map((m) => m[1]);
    expect(documented.length).toBeGreaterThan(0);
    expect([...documented].sort()).toEqual([...commandNames()].sort());
  });
});
