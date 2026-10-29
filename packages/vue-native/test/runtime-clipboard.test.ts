// ClipboardEvent: `copy` / `cut` / `paste` from the host
// (crates/vn-tree/src/clipboard.rs). The host builds `clipboardData` (a redjs
// DataTransfer) and hands it to the dispatcher as its eighth argument; the
// runtime wraps it in a ClipboardEvent and walks the path like any other
// host event. The host side (the DataTransfer's modes, the write-back of a
// cancelled copy) is tested in Rust; this file is the JS half.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Ev = {
  type: string;
  bubbles: boolean;
  cancelable: boolean;
  composed: boolean;
  clipboardData: unknown;
  target: unknown;
  currentTarget: unknown;
  preventDefault(): void;
};

/** A mounted app with `<div><input></div>` under the root (`document.body`). */
function world() {
  const rt = createRuntime();
  rt.createApp({ render: () => null }).mount();
  const body = rt.global.document.body;
  const box = rt.nodeOps.createElement("div");
  const input = rt.nodeOps.createElement("input");
  rt.nodeOps.insert(box, body, null);
  rt.nodeOps.insert(input, box, null);
  /** What the host does: one dispatch, `data` riding the eighth argument. */
  const fire = (node: { id: number }, name: "copy" | "cut" | "paste", data: unknown): boolean =>
    rt.host.dispatcher(node.__vnId, rt.EV_CODES[name], 0, 0, 0, 0, undefined, data);
  return { rt, body, box, input, fire };
}

/** Stands in for the host's DataTransfer; identity is what is under test. */
const store = () => ({ types: ["text/plain"], getData: () => "hello" });

describe("ClipboardEvent", () => {
  test("the codes are the host's, and a listener arms the host bit", () => {
    const { rt, input } = world();
    expect([rt.EV_CODES.paste, rt.EV_CODES.copy, rt.EV_CODES.cut]).toEqual([39, 40, 41]);
    rt.host.listeners.length = 0;
    rt.patchProp(input, "onPaste", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.paste)).toEqual([{ id: input.__vnId, code: 39, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("paste: a bubbling, cancelable, composed ClipboardEvent carrying the host's clipboardData", () => {
    const { rt, input, box, fire } = world();
    const data = store();
    const seen: Ev[] = [];
    rt.patchProp(input, "onPaste", null, (e: Ev) => seen.push(e));
    expect(fire(input, "paste", data)).toBe(true);
    expect(seen.length).toBe(1);
    const e = seen[0];
    expect(e).toBeInstanceOf(rt.global.ClipboardEvent);
    expect(e).toMatchObject({ type: "paste", bubbles: true, cancelable: true, composed: true });
    expect(e.clipboardData).toBe(data);
    expect(e.target).toBe(input);
    // A readonly attribute is a prototype getter, as WebIDL has it.
    expect("clipboardData" in rt.global.ClipboardEvent.prototype).toBe(true);
    expect(Object.keys(e)).not.toContain("clipboardData");
    void box;
  });

  test("it bubbles to an ancestor, then to document and window", () => {
    const { rt, input, box, fire } = world();
    const order: string[] = [];
    rt.patchProp(box, "onCopy", null, (e: Ev) => order.push(`box:${(e.target as { __vnId: number }).__vnId === input.__vnId}`));
    rt.global.document.addEventListener("copy", (e: Ev) => order.push(`document:${e.clipboardData !== null}`));
    expect(fire(input, "copy", store())).toBe(true);
    expect(order).toEqual(["box:true", "document:true"]);
  });

  test("preventDefault() is reported back, which is what cancels the host's default action", () => {
    const { rt, input, fire } = world();
    rt.global.document.addEventListener("cut", (e: Ev) => e.preventDefault());
    expect(fire(input, "cut", store())).toBe(false);
    rt.patchProp(input, "onPaste", null, (e: Ev) => e.preventDefault());
    expect(fire(input, "paste", store())).toBe(false);
  });

  test("a document listener rides the root's bit, and the root is the target when nothing is focused", () => {
    const { rt, body, fire } = world();
    rt.host.listeners.length = 0;
    const got: Ev[] = [];
    rt.global.document.addEventListener("paste", (e: Ev) => got.push(e));
    expect(rt.flagsFor(rt.EV_CODES.paste)).toEqual([{ id: body.__vnId, code: 39, on: true }]);
    fire(body, "paste", store());
    expect(got.length).toBe(1);
    expect(got[0].target).toBe(body);
  });

  // Found by packages/vn-test/specs/event-order.spec.ts: module-scope code
  // registers its document listeners BEFORE `mount()`, when there is no root to
  // carry the bit, and the host never delivered the event.
  test("a document listener added before mount() still rides the root's bit", () => {
    const rt = createRuntime();
    rt.global.document.addEventListener("copy", () => {});
    rt.host.listeners.length = 0;
    rt.createApp({ render: () => null }).mount();
    const root = rt.global.document.documentElement;
    expect(rt.flagsFor(rt.EV_CODES.copy)).toEqual([{ id: root.__vnId, code: rt.EV_CODES.copy, on: true }]);
  });

  test("no clipboardData from the host is null, not undefined", () => {
    const { rt, input, fire } = world();
    const seen: Ev[] = [];
    rt.patchProp(input, "onCopy", null, (e: Ev) => seen.push(e));
    fire(input, "copy", undefined);
    expect(seen[0].clipboardData).toBeNull();
  });

  test("a script can construct one, as in a browser", () => {
    const { rt, input } = world();
    const data = store();
    const seen: Ev[] = [];
    input.addEventListener("paste", (e: Ev) => seen.push(e));
    const ev = new rt.global.ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data });
    input.dispatchEvent(ev);
    expect(seen[0].clipboardData).toBe(data);
    expect(new rt.global.ClipboardEvent("copy").clipboardData).toBeNull();
  });
});
