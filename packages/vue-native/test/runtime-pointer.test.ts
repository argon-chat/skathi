// The pointer family (compat roadmap G4).
//
// The host sources three things — a move while hovered and the two edges of
// "the pointer is over me" — and the whole W3C pointer set is derived from
// those plus mousedown/mouseup. These tests pin the two halves that are easy to
// get wrong: which host listener a `@pointerdown` registers, and the ORDER and
// BUBBLING of what comes back out.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

function tree(rt) {
  const root = rt.nodeOps.createElement("root");
  const outer = rt.nodeOps.createElement("div");
  const inner = rt.nodeOps.createElement("button");
  rt.nodeOps.insert(outer, root, null);
  rt.nodeOps.insert(inner, outer, null);
  return { root, outer, inner };
}

describe("registration", () => {
  test("the host codes are in lockstep with crates/vn-tree/src/tree.rs", () => {
    const rt = createRuntime();
    expect(rt.EV_CODES.mousemove).toBe(14);
    expect(rt.EV_CODES.mouseenter).toBe(15);
    expect(rt.EV_CODES.mouseleave).toBe(16);
  });

  test("a pointer listener registers the host listener it is derived from", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(5), "onPointerdown", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.mousedown)).toEqual([{ id: 5, code: 2, on: true }]);
    expect(rt.advisories()).toEqual([]);

    rt.patchProp(rt.el(6), "onPointerenter", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.mouseenter)).toEqual([{ id: 6, code: 15, on: true }]);
  });

  test("dropping one name does not silence the other that shares the code", () => {
    // `pointerdown` and `mousedown` are ONE host listener; turning the pointer
    // one off while a mouse one is still bound must keep the flag set.
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("div");
    const fn = () => {};
    rt.patchProp(node, "onPointerdown", null, fn);
    rt.patchProp(node, "onMousedown", null, fn);
    expect(rt.flagsFor(rt.EV_CODES.mousedown).at(-1).on).toBe(true);
    rt.patchProp(node, "onPointerdown", fn, null);
    expect(rt.flagsFor(rt.EV_CODES.mousedown).at(-1).on).toBe(true);
    rt.patchProp(node, "onMousedown", fn, null);
    expect(rt.flagsFor(rt.EV_CODES.mousedown).at(-1).on).toBe(false);
  });

  test("focusin/focusout ride the focus/blur listeners", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(9), "onFocusin", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.focus)).toEqual([{ id: 9, code: 7, on: true }]);
  });
});

describe("dispatch", () => {
  test("pointerdown fires BEFORE mousedown, on the same target", () => {
    const rt = createRuntime();
    const { inner } = tree(rt);
    const order = [];
    rt.patchProp(inner, "onPointerdown", null, (e) => order.push(`pointerdown:${e.type}`));
    rt.patchProp(inner, "onMousedown", null, (e) => order.push(`mousedown:${e.type}`));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 12, 34, 0, 0, undefined);
    expect(order).toEqual(["pointerdown:pointerdown", "mousedown:mousedown"]);
  });

  test("a pointer event carries the pointer fields and the mouse coordinates", () => {
    const rt = createRuntime();
    const { inner } = tree(rt);
    let seen;
    rt.patchProp(inner, "onPointerdown", null, (e) => (seen = e));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 12, 34, 0, 0, undefined);
    expect(seen.clientX).toBe(12);
    expect(seen.clientY).toBe(34);
    expect(seen.pointerType).toBe("mouse");
    expect(seen.isPrimary).toBe(true);
    expect(seen.pointerId).toBe(1);
  });

  test("pointerdown bubbles; pointerenter/pointerleave do not", () => {
    const rt = createRuntime();
    const { outer, inner } = tree(rt);
    const seen = [];
    rt.patchProp(outer, "onPointerdown", null, () => seen.push("down@outer"));
    rt.patchProp(outer, "onPointerenter", null, () => seen.push("enter@outer"));
    rt.patchProp(inner, "onPointerenter", null, () => seen.push("enter@inner"));

    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mouseenter, 0, 0, -1, 0, undefined);
    // The press bubbles to the ancestor; the enter does NOT — which is what
    // keeps a menu from closing as the pointer crosses its own items.
    expect(seen).toEqual(["down@outer", "enter@inner"]);
  });

  test("mouseenter and pointerenter both fire, pointer first", () => {
    const rt = createRuntime();
    const { inner } = tree(rt);
    const order = [];
    rt.patchProp(inner, "onMouseenter", null, () => order.push("mouseenter"));
    rt.patchProp(inner, "onPointerenter", null, () => order.push("pointerenter"));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mouseenter, 0, 0, -1, 0, undefined);
    expect(order).toEqual(["pointerenter", "mouseenter"]);
  });

  test("focusin fires after focus and DOES bubble (unlike focus)", () => {
    const rt = createRuntime();
    const { outer, inner } = tree(rt);
    const order = [];
    rt.patchProp(inner, "onFocus", null, () => order.push("focus@inner"));
    rt.patchProp(inner, "onFocusin", null, () => order.push("focusin@inner"));
    rt.patchProp(outer, "onFocus", null, () => order.push("focus@outer"));
    rt.patchProp(outer, "onFocusin", null, () => order.push("focusin@outer"));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.focus, 0, 0, 0, 0, undefined);
    expect(order).toEqual(["focus@inner", "focusin@inner", "focusin@outer"]);
  });

  test("preventDefault on the pointer event cancels the source, as in a browser", () => {
    const rt = createRuntime();
    const { inner } = tree(rt);
    rt.patchProp(inner, "onPointerdown", null, (e) => e.preventDefault());
    const proceed = rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    expect(proceed).toBe(false);
  });

  test("an unheard pointer name still warns — the set is finite and named", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(), "onPointerover", null, () => {});
    expect(rt.advisories().length).toBe(1);
    expect(rt.advisories()[0]).toContain("@pointerover");
  });
});

// Since m20 the dispatcher only SYNTHESIZES an event somebody is listening for:
// the host arms one bit per code and several names share a code, so a delivery
// says nothing about which name has a handler, and building a W3C event costs
// more than everything else in a dispatch put together (scripts/boundary-cost
// .ts). These are the cases a careless version of that check would break — each
// one is a listener that is NOT on the target, which is the whole difficulty.
describe("only what someone listens for is built", () => {
  test("a listener on an ANCESTOR still receives the derived event", () => {
    const rt = createRuntime();
    const { outer, inner } = tree(rt);
    const seen = [];
    rt.patchProp(outer, "onPointerdown", null, (e) => seen.push(e.type));
    // Nothing on `inner` listens for anything; the event is delivered there.
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    expect(seen).toEqual(["pointerdown"]);
  });

  test("a listener on an ancestor still receives the SOURCE event", () => {
    const rt = createRuntime();
    const { outer, inner } = tree(rt);
    const seen = [];
    rt.patchProp(outer, "onMousedown", null, (e) => seen.push(e.type));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    expect(seen).toEqual(["mousedown"]);
  });

  test("a document-level listener still receives it", () => {
    // Every dismissable layer in the ecosystem listens here and nowhere else,
    // so a check that only walked the mirror tree would close nothing.
    const rt = createRuntime();
    const { root, inner } = tree(rt);
    rt.createApp({ render: () => null }); // no mount: appRoot stays null…
    const seen = [];
    rt.global.document.addEventListener("pointerdown", (e) => seen.push(e.type));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    expect(seen).toEqual(["pointerdown"]);
    expect(root).toBeTruthy();
  });

  test("both names listening still produce both events, in order", () => {
    const rt = createRuntime();
    const { inner } = tree(rt);
    const order = [];
    rt.patchProp(inner, "onMousedown", null, (e) => order.push(e.type));
    rt.patchProp(inner, "onPointerdown", null, (e) => order.push(e.type));
    rt.host.dispatcher(inner.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    expect(order).toEqual(["pointerdown", "mousedown"]);
  });
});


describe("pointer capture", () => {
  const fire = (rt, node, name, x = 10, y = 20, button = 0) =>
    rt.host.dispatcher(node.__vnId, rt.EV_CODES[name], x, y, button, 0, undefined);

  test("captured moves and the outside release target the owner and release capture", () => {
    const rt = createRuntime();
    const { root, inner } = tree(rt);
    const seen = [];
    inner.addEventListener("pointerdown", (e) => inner.setPointerCapture(e.pointerId));
    for (const name of ["gotpointercapture", "pointermove", "pointerup", "lostpointercapture"])
      inner.addEventListener(name, (e) => seen.push([e.type, e.target === inner, e.clientX, e.buttons]));
    fire(rt, inner, "mousedown");
    expect(inner.hasPointerCapture(1)).toBe(true);
    fire(rt, root, "mousemove", -70, 150, 0);
    fire(rt, root, "mouseup", -70, 150, 0);
    expect(inner.hasPointerCapture(1)).toBe(false);
    fire(rt, root, "mousemove", 5, 5, -1);
    expect(seen).toEqual([
      ["gotpointercapture", true, 10, 1], ["pointermove", true, -70, 1],
      ["pointerup", true, -70, 0], ["lostpointercapture", true, -70, 0],
    ]);
    expect(rt.host.pointerCaptureCalls).toEqual([
      { id: inner.__vnId, pointerId: 1, capture: true },
      { id: inner.__vnId, pointerId: 1, capture: false },
    ]);
    expect(rt.advisories()).toEqual([]);
  });

  test("capture transfer has one owner, with lost before the new got event", () => {
    const rt = createRuntime();
    const { inner, outer } = tree(rt);
    const seen = [];
    inner.addEventListener("pointerdown", () => inner.setPointerCapture(1));
    inner.addEventListener("lostpointercapture", (e) => { seen.push(e.type); expect(e.cancelable).toBe(false); });
    outer.addEventListener("gotpointercapture", (e) => { if (e.target === outer) seen.push(e.type); });
    fire(rt, inner, "mousedown");
    outer.setPointerCapture(1);
    expect(seen).toEqual(["lostpointercapture", "gotpointercapture"]);
    expect(inner.hasPointerCapture(1)).toBe(false);
    expect(outer.hasPointerCapture(1)).toBe(true);
    inner.releasePointerCapture(1);
    expect(outer.hasPointerCapture(1)).toBe(true);
    fire(rt, inner, "mouseup");
    expect(outer.hasPointerCapture(1)).toBe(false);
  });

  test("native cancellation and a move without a held button both stop captured gestures", () => {
    for (const ending of ["pointercancel", "mousemove"]) {
      const rt = createRuntime();
      const { root, inner } = tree(rt);
      const seen = [];
      inner.addEventListener("pointerdown", () => inner.setPointerCapture(1));
      inner.addEventListener("pointercancel", (e) => seen.push([e.type, e.target === inner, e.buttons]));
      inner.addEventListener("lostpointercapture", () => seen.push(["lost"]));
      inner.addEventListener("pointermove", () => seen.push(["stale move"]));
      fire(rt, inner, "mousedown");
      fire(rt, root, ending, 99, 99, -1);
      expect(seen).toEqual([["pointercancel", true, 0], ["lost"]]);
      expect(inner.hasPointerCapture(1)).toBe(false);
    }
  });

  test("removing a captured subtree releases native capture before its ids are forgotten", () => {
    const rt = createRuntime();
    const { inner, outer } = tree(rt);
    const lost = [];
    inner.addEventListener("pointerdown", () => inner.setPointerCapture(1));
    inner.addEventListener("lostpointercapture", () => lost.push(inner.__vnId));
    fire(rt, inner, "mousedown");
    const id = inner.__vnId;
    rt.nodeOps.remove(outer);
    expect(lost).toEqual([id]);
    expect(inner.hasPointerCapture(1)).toBe(false);
    expect(rt.host.pointerCaptureCalls.at(-1)).toEqual({ id, pointerId: 1, capture: false });
  });

  test("lost-capture listeners can transfer again without leaving an unannounced owner", () => {
    const rt = createRuntime();
    const { root, outer, inner } = tree(rt);
    const third = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(third, root, null);
    const seen = [];
    inner.addEventListener("pointerdown", () => inner.setPointerCapture(1));
    inner.addEventListener("lostpointercapture", () => { seen.push("inner lost"); third.setPointerCapture(1); });
    outer.addEventListener("gotpointercapture", (e) => { if (e.target === outer) seen.push("outer got"); });
    outer.addEventListener("lostpointercapture", (e) => { if (e.target === outer) seen.push("outer lost"); });
    third.addEventListener("gotpointercapture", () => seen.push("third got"));
    fire(rt, inner, "mousedown");
    outer.setPointerCapture(1);
    expect(outer.hasPointerCapture(1)).toBe(false);
    expect(third.hasPointerCapture(1)).toBe(true);
    expect(seen).toEqual(["inner lost", "outer lost", "third got"]);
  });

  test("a lost-capture listener cannot recapture a subtree being removed", () => {
    const rt = createRuntime();
    const { inner, outer } = tree(rt);
    let lost = 0;
    inner.addEventListener("pointerdown", () => inner.setPointerCapture(1));
    inner.addEventListener("lostpointercapture", () => { lost++; inner.setPointerCapture(1); });
    fire(rt, inner, "mousedown");
    rt.nodeOps.remove(outer);
    expect(lost).toBe(1);
    expect(inner.hasPointerCapture(1)).toBe(false);
  });

  test("only the active mouse pointer can be captured", () => {
    const rt = createRuntime();
    const { inner } = tree(rt);
    inner.setPointerCapture(1);
    expect(inner.hasPointerCapture(1)).toBe(false);
    expect(() => inner.setPointerCapture(42)).toThrow("No active pointer");
  });
});
