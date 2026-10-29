// `<canvas>`'s element half (src/canvas.ts): width/height reflection, the
// context it asks redjs for, resizes, toDataURL/toBlob, and the source hook
// drawImage(canvasElement) goes through. The engine's natives are faked here
// (bun has no redjs); the pixels themselves are redjs's tests'
// (vendor/redjs webapi/canvas2d/jsc_tests.rs).

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

/** A fake of redjs's `__rjs_canvas_*` natives that records every call. */
function fakeNatives() {
  const calls: Any[] = [];
  let hook: ((v: unknown) => unknown) | null = null;
  const n = {
    calls,
    hook: () => hook,
    __rjs_canvas_context(canvas: Any, w: number, h: number, options: Any, key: number) {
      const ctx = { kind: "ctx", canvas, size: [w, h], options, key };
      calls.push(["context", w, h, key]);
      return ctx;
    },
    __rjs_canvas_resize(ctx: Any, w: number, h: number) {
      ctx.size = [w, h];
      calls.push(["resize", w, h]);
    },
    __rjs_canvas_data_url(ctx: Any, w: number, h: number, type: string, q: unknown) {
      calls.push(["dataUrl", ctx ? "ctx" : null, w, h, type, q]);
      return `data:${type};fake`;
    },
    __rjs_canvas_blob(ctx: Any, w: number, h: number, type: string) {
      calls.push(["blob", ctx ? "ctx" : null, w, h, type]);
      return { blob: type };
    },
    __rjs_canvas_source_hook(f: (v: unknown) => unknown) {
      hook = f;
    },
  };
  return n;
}

const NAMES = [
  "__rjs_canvas_context",
  "__rjs_canvas_resize",
  "__rjs_canvas_data_url",
  "__rjs_canvas_blob",
  "__rjs_canvas_source_hook",
];

let natives: ReturnType<typeof fakeNatives>;
beforeEach(() => {
  natives = fakeNatives();
  for (const k of NAMES) (globalThis as Any)[k] = (natives as Any)[k];
});
afterEach(() => {
  for (const k of NAMES) delete (globalThis as Any)[k];
});

function world() {
  const rt: Any = createRuntime();
  return { rt, doc: rt.global.document };
}

describe("width and height", () => {
  test("default to 300 x 150 and reflect the attributes", () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    expect([c.width, c.height]).toEqual([300, 150]);
    c.setAttribute("width", " 64px");
    expect(c.width).toBe(64);
    c.setAttribute("height", "nope");
    expect(c.height).toBe(150);
    c.width = 12.9;
    expect(c.getAttribute("width")).toBe("12");
    c.removeAttribute("width");
    expect(c.width).toBe(300);
  });

  test("a renderer-made canvas reads its :width / :height props", () => {
    const { rt } = world();
    const c = rt.nodeOps.createElement("canvas");
    rt.patchProp(c, "width", null, 40);
    rt.patchProp(c, "height", null, "30");
    expect([c.width, c.height]).toEqual([40, 30]);
  });
});

describe("getContext", () => {
  test("asks redjs once, at the element's size, keyed by its host node id", () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    c.width = 20;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    expect(ctx.kind).toBe("ctx");
    expect(ctx.canvas).toBe(c);
    expect(ctx.options).toEqual({ willReadFrequently: true });
    expect(c.getContext("2d")).toBe(ctx);
    expect(natives.calls.filter((x) => x[0] === "context")).toEqual([["context", 20, 150, c.__vnId]]);
  });

  test("other kinds are null, and so is 2d on an engine without canvas", () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    expect(c.getContext("webgl")).toBeNull();
    for (const k of NAMES) delete (globalThis as Any)[k];
    expect(doc.createElement("canvas").getContext("2d")).toBeNull();
  });

  test("setting width or height resizes the bitmap, even to the same value", () => {
    const { rt, doc } = world();
    const c = doc.createElement("canvas");
    const ctx = c.getContext("2d");
    c.width = 10;
    expect(ctx.size).toEqual([10, 150]);
    c.width = c.width;
    rt.patchProp(c, "height", null, 7);
    expect(natives.calls.filter((x) => x[0] === "resize")).toEqual([
      ["resize", 10, 150],
      ["resize", 10, 150],
      ["resize", 10, 7],
    ]);
  });
});

describe("toDataURL and toBlob", () => {
  test("toDataURL encodes the element's bitmap (a blank one before any context)", () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    expect(c.toDataURL()).toBe("data:image/png;fake");
    c.getContext("2d");
    expect(c.toDataURL("image/jpeg", 0.5)).toBe("data:image/jpeg;fake");
    expect(natives.calls.filter((x) => x[0] === "dataUrl")).toEqual([
      ["dataUrl", null, 300, 150, "image/png", undefined],
      ["dataUrl", "ctx", 300, 150, "image/jpeg", 0.5],
    ]);
  });

  test("toBlob calls back on a later task, never synchronously", async () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    let got: Any = "not yet";
    c.toBlob((b: Any) => (got = b), "image/webp");
    expect(got).toBe("not yet");
    await new Promise((r) => setTimeout(r, 5));
    expect(got).toEqual({ blob: "image/webp" });
    expect(() => c.toBlob(null)).toThrow(TypeError);
  });
});

describe("the source hook", () => {
  test("maps a canvas to its context, a video or img to nothing yet, anything else to not-a-source", () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    const hook = natives.hook()!;
    expect(typeof hook).toBe("function");
    const ctx = hook(c) as Any;
    expect(ctx.kind).toBe("ctx");
    expect(c.getContext("2d")).toBe(ctx);
    expect(hook(doc.createElement("video"))).toBeNull();
    expect(hook(doc.createElement("img"))).toBeNull();
    expect(hook({})).toBeUndefined();
    expect(hook(doc.createElement("div"))).toBeUndefined();
  });
});
