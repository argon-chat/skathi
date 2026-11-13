// The code tables that cross the language boundary, checked against Rust (runtime.ts `EV_CODES` and protocol.d.ts
// `VnEventCode` against tree.rs `EVENT_NAMES`). A drift is an event that silently stops being delivered, so this reads
// the Rust. It is also why the JS tables are plain object literals rather than a `const enum`.

import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { createRuntime } from "./runtime-harness";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "../../..");
const read = (rel: string) => readFileSync(join(REPO, rel), "utf8");

function rustConsts(source: string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const m of source.matchAll(/pub const (\w+): u32 = (\d+);/g)) out[m[1]!] = Number(m[2]);
  return out;
}

function rustEventNames() {
  const source = read("crates/vn-tree/src/tree.rs");
  const consts = rustConsts(source);
  const table = source.slice(source.indexOf("pub const EVENT_NAMES"));
  const out: Record<string, number> = {};
  for (const m of table.matchAll(/\((EV_\w+),\s*"([^"]+)"\)/g)) {
    expect(consts[m[1]!], `tree.rs names ${m[1]} in EVENT_NAMES but never defines it`).toBeDefined();
    out[m[2]!] = consts[m[1]!]!;
  }
  return out;
}

test("EV_CODES is exactly tree.rs's EVENT_NAMES", () => {
  const { EV_CODES } = createRuntime();
  const rust = rustEventNames();
  expect(Object.keys(EV_CODES).sort()).toEqual(Object.keys(rust).sort());
  for (const name of Object.keys(rust)) {
    expect((EV_CODES as Record<string, number>)[name], `EV_CODES.${name}`).toBe(rust[name]!);
  }
});

test("every host code fits the u64 listener bitset", () => {
  // `Node::listeners` is a u64, so a code of 64 or more would set no bit at all
  // and the event would simply never be delivered — silently.
  for (const [name, code] of Object.entries(rustEventNames())) {
    expect(code, `${name} would not fit Node::listeners`).toBeLessThan(64);
    expect(code, `${name}: 0 is the reserved "no event" code`).toBeGreaterThan(0);
  }
});

test("protocol.d.ts's VnEventCode lists the same codes", () => {
  const dts = readFileSync(join(HERE, "../src/protocol.d.ts"), "utf8");
  const block = dts.slice(dts.indexOf("type VnEventCode ="));
  const listed = [...block.slice(0, block.indexOf(";")).matchAll(/\|\s*(\d+)/g)].map((m) => Number(m[1]));
  const rust = Object.values<number>(rustEventNames()).sort((a: number, b: number) => a - b);
  expect(listed.sort((a: number, b: number) => a - b)).toEqual(rust);
});

test("EDIT_TYPES is vn-elements' InputType, code for code", () => {
  // The host sends an edit's `inputType` as its `InputType` discriminant; the
  // runtime names it by indexing EDIT_TYPES. A variant added or renumbered on
  // one side only is an `input` / `beforeinput` with the wrong `inputType`.
  const source = read("crates/vn-elements/src/text_input/mod.rs");
  const body = source.slice(source.indexOf("pub enum InputType {"));
  const variants = [...body.slice(0, body.indexOf("\n}")).matchAll(/^\s*(\w+) = (\d+),/gm)];
  const names = Object.fromEntries([...source.matchAll(/InputType::(\w+) => "(\w+)",/g)].map((m) => [m[1], m[2]]));
  const fromRust: string[] = [""];
  for (const [, variant, code] of variants) fromRust[Number(code)] = names[variant!]!;
  expect(variants.length).toBeGreaterThan(10);
  const runtime = readFileSync(join(HERE, "../src/runtime.ts"), "utf8");
  const table = runtime.slice(runtime.indexOf("const EDIT_TYPES = ["));
  const fromTs = [...table.slice(0, table.indexOf("] as const")).matchAll(/"(\w*)"/g)].map((m) => m[1]);
  expect(fromTs).toEqual(fromRust);
});
