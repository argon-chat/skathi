// The code tables that cross the language boundary, checked against Rust.
//
// Three tables in this repo are declared to be "in lockstep" with a Rust one,
// and until now nothing enforced it:
//
//   src/runtime.ts   `EV_CODES`  <->  crates/vn-tree/src/tree.rs `EVENT_NAMES`
//   src/livekit.ts   `OP` / `EV` <->  crates/vn-voice/src/proto.rs `op` / `ev`
//   src/protocol.d.ts `VnEventCode` <-> the same `EVENT_NAMES`
//
// A drift in the first is an event that silently stops being delivered; a
// drift in the second is, in `proto.rs`'s own words, "a silent misrouting — an
// op that quietly becomes a different op". Both are the kind of bug that no
// unit test on either side can see, because each side is self-consistent.
//
// So this file reads the RUST. It is the one test here that does, and that is
// the point: a comment saying "keep these in step" is a request, and a test
// that parses the other language is an answer. It is also why the JS tables
// are plain object literals rather than a TypeScript `const enum` — a
// `const enum` would give the compiler a private opinion about the numbers and
// still tell it nothing about `tree.rs`.

import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { EV as HARNESS_EV, OP as HARNESS_OP } from "./livekit-harness";
import { createRuntime } from "./runtime-harness";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "../../..");
const read = (rel) => readFileSync(join(REPO, rel), "utf8");

/** `pub const NAME: u32 = 12;` -> { NAME: 12 }, over one slice of a Rust file. */
function rustConsts(source) {
  const out = {};
  for (const m of source.matchAll(/pub const (\w+): u32 = (\d+);/g)) out[m[1]] = Number(m[2]);
  return out;
}

/** The body of `pub mod <name> { … }`, by brace matching from its opening. */
function rustMod(source, name) {
  const start = source.indexOf(`pub mod ${name} {`);
  expect(start, `crates/vn-voice/src/proto.rs has no \`pub mod ${name}\``).toBeGreaterThan(-1);
  let depth = 0;
  for (let i = source.indexOf("{", start); i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) return source.slice(start, i);
  }
  throw new Error(`unterminated \`pub mod ${name}\``);
}

/** `const OP = { A: 1, … };` -> { A: 1, … }, off one of our own .ts sources. */
function tsTable(source, name) {
  const start = source.indexOf(`const ${name} = {`);
  expect(start, `the table \`const ${name} = {\` moved or was renamed`).toBeGreaterThan(-1);
  const end = source.indexOf("\n};", start);
  const body = source.slice(start, end);
  const out = {};
  for (const m of body.matchAll(/^\s*(\w+):\s*(\d+),/gm)) out[m[1]] = Number(m[2]);
  return out;
}

// ---------------------------------------------------------------------------
// the input event codes — src/runtime.ts vs crates/vn-tree/src/tree.rs
// ---------------------------------------------------------------------------

/** `EVENT_NAMES` as `{ "click": 1, … }` — the JS spelling of every host code. */
function rustEventNames() {
  const source = read("crates/vn-tree/src/tree.rs");
  const consts = rustConsts(source);
  const table = source.slice(source.indexOf("pub const EVENT_NAMES"));
  const out = {};
  for (const m of table.matchAll(/\((EV_\w+),\s*"([^"]+)"\)/g)) {
    expect(consts[m[1]], `tree.rs names ${m[1]} in EVENT_NAMES but never defines it`).toBeDefined();
    out[m[2]] = consts[m[1]];
  }
  return out;
}

test("EV_CODES is exactly tree.rs's EVENT_NAMES", () => {
  const { EV_CODES } = createRuntime();
  const rust = rustEventNames();
  // Both directions, and by NAME rather than by count: a code added on one side
  // only is the failure this exists for, and it can go either way — a new host
  // event JS never learns to listen for, or a JS name pointing at a code the
  // host does not source.
  expect(Object.keys(EV_CODES).sort()).toEqual(Object.keys(rust).sort());
  for (const name of Object.keys(rust)) {
    expect(EV_CODES[name], `EV_CODES.${name}`).toBe(rust[name]);
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
  const listed = [...block.slice(0, block.indexOf(";")).matchAll(/\|\s*(\d+)/g)].map((m) =>
    Number(m[1]),
  );
  const rust = Object.values(rustEventNames()).sort((a, b) => a - b);
  expect(listed.sort((a, b) => a - b)).toEqual(rust);
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

// ---------------------------------------------------------------------------
// the voice op/event codes — src/livekit.ts vs crates/vn-voice/src/proto.rs
// ---------------------------------------------------------------------------

const LIVEKIT_SOURCE = readFileSync(join(HERE, "../src/livekit.ts"), "utf8");
const PROTO_SOURCE = read("crates/vn-voice/src/proto.rs");

test("livekit.ts's OP table is proto.rs's `op` module", () => {
  expect(tsTable(LIVEKIT_SOURCE, "OP")).toEqual(rustConsts(rustMod(PROTO_SOURCE, "op")));
});

test("livekit.ts's EV table is proto.rs's `ev` module", () => {
  expect(tsTable(LIVEKIT_SOURCE, "EV")).toEqual(rustConsts(rustMod(PROTO_SOURCE, "ev")));
});

test("the livekit harness carries the same tables as the shim", () => {
  // The harness keeps its own copy so a test can name an op without importing
  // a private table; a third copy is a third thing that can drift.
  expect(HARNESS_OP).toEqual(tsTable(LIVEKIT_SOURCE, "OP"));
  expect(HARNESS_EV).toEqual(tsTable(LIVEKIT_SOURCE, "EV"));
});

test("no op code can be read as an event code", () => {
  // The dispatcher splits on `kind >= KIND_EVENT_BASE`, so an op that landed in
  // the event range would be routed as a room event and its promise would never
  // settle. `SET_PLAYOUT_STREAM` is 64 rather than 16 for exactly this reason.
  const base = rustConsts(PROTO_SOURCE).KIND_EVENT_BASE;
  expect(base).toBe(16);
  const ops = Object.values(tsTable(LIVEKIT_SOURCE, "OP"));
  const evs = Object.values(tsTable(LIVEKIT_SOURCE, "EV"));
  for (const op of ops) expect(evs.includes(op), `op ${op} collides with an event code`).toBe(false);
  for (const ev of evs) expect(ev, `event ${ev} is below KIND_EVENT_BASE`).toBeGreaterThanOrEqual(base);
});
