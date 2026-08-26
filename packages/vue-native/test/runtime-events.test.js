// The renderer's event-prop plumbing: which `on*` props reach the host, which
// names they normalise to, and — the compat-first part — that a prop the host
// cannot deliver says so instead of vanishing.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness.js";

describe("patchProp: on* props", () => {
  test("a known event registers its listener flag under the host's code", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(7), "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 7, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("an event the host does not deliver warns instead of vanishing", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(), "onPointerdown", null, () => {});
    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    // Same shape as the addEventListener advisory next to it.
    expect(advisories[0]).toContain("@pointerdown");
    expect(advisories[0]).toContain("not delivered by the native host yet");
    expect(advisories[0]).toContain("will never fire");
    // …and no listener flag was set, because there is no code to set it under.
    expect(rt.host.listeners).toEqual([]);
  });

  test("the advisory is one per event name per session, not one per patch", () => {
    const rt = createRuntime();
    for (let i = 0; i < 5; i++) rt.patchProp(rt.el(i), "onPointerdown", null, () => {});
    rt.patchProp(rt.el(), "onPointermove", null, () => {});
    expect(rt.advisories().length).toBe(2);
    // A *different* world starts over — the dedup set is per module instance.
    const other = createRuntime();
    other.patchProp(other.el(), "onPointerdown", null, () => {});
    expect(other.advisories().length).toBe(1);
  });

  test("addEventListener and the prop path share one dedup key", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(3), "onPointerdown", null, () => {});
    const node = rt.nodeOps.createElement("div");
    node.addEventListener("pointerdown", () => {});
    expect(rt.advisories().length).toBe(1);
  });

  test("component-emit props that leak onto an element stay silent", () => {
    // `<input v-model>` compiles to an `onUpdate:modelValue` prop on the
    // ELEMENT. A browser binds it to an event that never fires, silently; the
    // advisory is for real DOM event names, not for this.
    const rt = createRuntime();
    rt.patchProp(rt.el(), "onUpdate:modelValue", null, () => {});
    expect(rt.advisories()).toEqual([]);
  });

  test("modifiers are stripped before the name is looked up", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(2), "onClickCaptureOnce", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 2, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });
});

describe("<lottie> event names", () => {
  test("the host codes are in lockstep with crates/vn-tree/src/tree.rs", () => {
    const rt = createRuntime();
    expect(rt.EV_CODES.complete).toBe(11);
    expect(rt.EV_CODES.loopComplete).toBe(12);
    expect(rt.EV_CODES.enterFrame).toBe(13);
  });

  test("the `on:` form the compiler emits for a custom element keeps its case", () => {
    // `@loopComplete` on <lottie> compiles to the prop `on:loopComplete` —
    // compiler-dom's case-preserving spelling for a custom element. That is the
    // shape this element actually receives, so it is the one that must work.
    const rt = createRuntime();
    rt.patchProp(rt.el(4), "on:loopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("the hyphenated spelling folds onto the same name", () => {
    // If the tag is ever classified as a plain element, `@loopComplete` becomes
    // `onLoopComplete` and runtime-dom hyphenates it to "loop-complete" — the
    // well-known reason a camelCase custom-element event is missed in the
    // browser. We fold it back rather than dropping the listener.
    const rt = createRuntime();
    expect(rt.hyphenate("LoopComplete")).toBe("loop-complete");
    expect(rt.canonicalEvent("loop-complete")).toBe("loopComplete");
    rt.patchProp(rt.el(4), "onLoopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("an unknown `on:` event still gets its advisory", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(), "on:someCustomThing", null, () => {});
    expect(rt.advisories().length).toBe(1);
    expect(rt.advisories()[0]).toContain("@someCustomThing");
  });

  test("addEventListener uses lottie-web's own camelCase spelling", () => {
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("lottie");
    node.addEventListener("loopComplete", () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete).length).toBe(1);
    expect(rt.advisories()).toEqual([]);
  });

  test("both spellings land in ONE bucket, so removing works across them", () => {
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("lottie");
    const fn = () => {};
    node.addEventListener("enter-frame", fn);
    expect(rt.flagsFor(rt.EV_CODES.enterFrame).at(-1).on).toBe(true);
    node.removeEventListener("enterFrame", fn);
    expect(rt.flagsFor(rt.EV_CODES.enterFrame).at(-1).on).toBe(false);
  });
});
