// `<canvas>`'s element half (src/canvas.ts): width/height reflection, the
// context it asks redjs for, resizes, toDataURL/toBlob, and the source hook
// drawImage(canvasElement) goes through. The engine's natives are faked here
// (bun has no redjs); the pixels themselves are redjs's tests'
// (vendor/redjs webapi/canvas2d/jsc_tests.rs).

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

interface Ctx {
  kind: string;
  canvas: unknown;
  size: number[];
  options?: unknown;
  key: number;
}

// The element surface src/canvas.ts installs on a canvas mirror.
type CanvasElement = MirrorElement & {
  width: number;
  height: number;
  getContext(kind: string, options?: unknown): Ctx | null;
  toDataURL(type?: string, quality?: number): string;
  toBlob(callback: ((blob: unknown) => void) | null, type?: string, quality?: number): void;
};
type ImgElement = MirrorElement & { src: string };
interface CanvasDocument {
  createElement(tag: "canvas"): CanvasElement;
  createElement(tag: "img"): ImgElement;
  createElement(tag: string): MirrorElement;
}

const globals = globalThis as unknown as Record<string, unknown>;

/** A fake of redjs's `__rjs_canvas_*` natives that records every call. */
function fakeNatives() {
  const calls: unknown[][] = [];
  let hook: ((v: unknown) => unknown) | null = null;
  const n = {
    calls,
    hook: () => hook,
    __rjs_canvas_context(canvas: unknown, w: number, h: number, options: unknown, key: number) {
      const ctx: Ctx = { kind: "ctx", canvas, size: [w, h], options, key };
      calls.push(["context", w, h, key]);
      return ctx;
    },
    __rjs_canvas_resize(ctx: Ctx, w: number, h: number) {
      ctx.size = [w, h];
      calls.push(["resize", w, h]);
    },
    __rjs_canvas_data_url(ctx: Ctx | null, w: number, h: number, type: string, q: unknown) {
      calls.push(["dataUrl", ctx ? "ctx" : null, w, h, type, q]);
      return `data:${type};fake`;
    },
    __rjs_canvas_blob(ctx: Ctx | null, w: number, h: number, type: string) {
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
  "__rjs_webgpu_context",
  "__rjs_webgpu_resize",
  "__rjs_webgpu_data_url",
  "__rjs_webgpu_blob",
];

let natives: ReturnType<typeof fakeNatives>;
beforeEach(() => {
  natives = fakeNatives();
  for (const k of NAMES) globals[k] = (natives as unknown as Record<string, unknown>)[k];
});
afterEach(() => {
  for (const k of NAMES) delete globals[k];
});

function world() {
  const rt = createRuntime();
  return { rt, doc: rt.global.document as unknown as CanvasDocument };
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
    const c = rt.nodeOps.createElement("canvas") as CanvasElement;
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
    const ctx = c.getContext("2d", { willReadFrequently: true })!;
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
    for (const k of NAMES) delete globals[k];
    expect(doc.createElement("canvas").getContext("2d")).toBeNull();
  });

  test("setting width or height resizes the bitmap, even to the same value", () => {
    const { rt, doc } = world();
    const c = doc.createElement("canvas");
    const ctx = c.getContext("2d")!;
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
    let got: unknown = "not yet";
    c.toBlob((b) => (got = b), "image/webp");
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
    expect(hook(c)).toBeNull();
    expect(natives.calls.filter((x) => x[0] === "context")).toEqual([]);
    const ctx = c.getContext("2d");
    expect(hook(c)).toBe(ctx);
    expect(hook(doc.createElement("video"))).toBeNull();
    expect(hook(doc.createElement("img"))).toBeNull();
    expect(hook({})).toBeUndefined();
    expect(hook(doc.createElement("div"))).toBeUndefined();
  });
});

describe("drawImage of an <img>", () => {
  /** A fake `__vn_img_bitmap` that hands out numbered bitmaps and records each ask. */
  function fakeHost(answer: (id: number) => unknown = () => undefined) {
    const asks: unknown[][] = [];
    const closed: number[] = [];
    let next = 0;
    globals.__vn_img_bitmap = (id: number, clean: boolean) => {
      asks.push([id, clean]);
      const given = answer(id);
      if (given !== undefined) return given;
      const n = ++next;
      return { n, close: () => closed.push(n) };
    };
    return { asks, closed };
  }
  afterEach(() => delete globals.__vn_img_bitmap);

  /** The hook, which the first canvas installs. */
  const hookOf = (doc: CanvasDocument) => (doc.createElement("canvas"), natives.hook()!);

  /** The host's `load` (or `error`) answer for the element's src, as the dispatcher delivers it. */
  const answer = (rt: ReturnType<typeof createRuntime>, img: ImgElement, name: "load" | "error", w = 0, h = 0) =>
    rt.host.dispatcher(img.__vnId, rt.EV_CODES[name], w, h, 0, 0, img.getAttribute("src"));

  test("a loaded image is the host's bitmap, decoded once per src", () => {
    const { rt, doc } = world();
    const host = fakeHost();
    const hook = hookOf(doc);
    const img = doc.createElement("img");
    img.src = "blob:app://x/1";
    expect(hook(img)).toBeNull(); // still loading: nothing to draw, nothing asked
    expect(host.asks).toEqual([]);
    answer(rt, img, "load", 64, 48);
    const first = hook(img) as { n: number };
    expect(first.n).toBe(1);
    expect(hook(img)).toBe(first);
    expect(host.asks).toEqual([[img.__vnId, true]]);
    img.src = "blob:app://x/2";
    expect(hook(img)).toBeNull();
    answer(rt, img, "load", 10, 10);
    expect((hook(img) as { n: number }).n).toBe(2);
    expect(host.closed).toEqual([1]); // the old src's pixels go
  });

  test("a broken image throws InvalidStateError; no src draws nothing", () => {
    const { rt, doc } = world();
    fakeHost();
    const hook = hookOf(doc);
    const img = doc.createElement("img");
    expect(hook(img)).toBeNull();
    img.src = "nope.png";
    answer(rt, img, "error");
    let thrown: { name?: string; message?: unknown } | undefined;
    try {
      hook(img);
    } catch (e) {
      thrown = e as { name?: string; message?: unknown };
    }
    expect(thrown?.name).toBe("InvalidStateError");
    expect(String(thrown?.message)).toContain("'broken' state");
  });

  test("an http(s) image without crossorigin is not origin-clean", () => {
    const { rt, doc } = world();
    const host = fakeHost();
    const hook = hookOf(doc);
    const remote = doc.createElement("img");
    remote.src = "https://cdn.example/a.png";
    answer(rt, remote, "load", 1, 1);
    hook(remote);
    const cors = doc.createElement("img");
    cors.setAttribute("crossorigin", "anonymous");
    cors.src = "https://cdn.example/a.png";
    answer(rt, cors, "load", 1, 1);
    hook(cors);
    const local = doc.createElement("img");
    local.src = "assets/a.png";
    answer(rt, local, "load", 1, 1);
    hook(local);
    expect(host.asks.map((a) => a[1])).toEqual([false, true, true]);
  });

  test("a src the host cannot decode draws nothing, with one advisory", () => {
    const { rt, doc } = world();
    const host = fakeHost(() => "the bytes are not an image");
    const hook = hookOf(doc);
    const warn = console.warn;
    const said: string[] = [];
    console.warn = (m: string) => said.push(m);
    try {
      const img = doc.createElement("img");
      img.src = "data:image/png;base64,AAAA";
      answer(rt, img, "load", 2, 2);
      expect(hook(img)).toBeNull();
      expect(hook(img)).toBeNull();
    } finally {
      console.warn = warn;
    }
    expect(host.asks.length).toBe(2); // not cached: it may decode later
    expect(said.filter((m) => m.includes("drawImage(<img>) draws nothing: the bytes are not an image"))).toHaveLength(
      1,
    );
  });
});

/** Native GPU presentation/capture remains opaque to the element layer. */
function fakeGPU() {
  const calls: unknown[][] = [];
  const n = {
    __rjs_webgpu_context(canvas: unknown, w: number, h: number, key: number) {
      const ctx = { kind: "gpu", canvas, size: [w, h], key };
      calls.push(["context", w, h, key]);
      return ctx;
    },
    __rjs_webgpu_resize(ctx: Ctx, w: number, h: number) {
      ctx.size = [w, h];
      calls.push(["resize", ctx, w, h]);
    },
    __rjs_webgpu_data_url(ctx: Ctx, type: string, q: unknown) {
      calls.push(["dataUrl", ctx, type, q]);
      return `data:${type};gpu`;
    },
    __rjs_webgpu_blob(ctx: Ctx, type: string, q: unknown) {
      calls.push(["blob", ctx, type, q]);
      return ctx.size.includes(0) ? null : { gpuBlob: type };
    },
  };
  Object.assign(globalThis, n);
  return { calls, ...n };
}

describe("WebGPU canvas selection", () => {
  test("returns one native context with the element identity and rejects switching to 2D", () => {
    const gpu = fakeGPU();
    const { doc } = world();
    const c = doc.createElement("canvas");
    c.width = 40;
    c.height = 30;
    const context = c.getContext("webgpu")!;
    expect(context.canvas).toBe(c);
    expect(c.getContext("webgpu", { ignored: true })).toBe(context);
    expect(c.getContext("2d")).toBeNull();
    expect(gpu.calls).toEqual([["context", 40, 30, c.__vnId]]);
    expect(natives.calls.filter((x) => x[0] === "context")).toEqual([]);
  });

  test("native option getters cannot choose another mode during creation", () => {
    const gpu = fakeGPU();
    const { doc } = world();
    const c = doc.createElement("canvas");
    c.width = 20;
    const create = natives.__rjs_canvas_context;
    globals.__rjs_canvas_context = (
      canvas: unknown,
      w: number,
      h: number,
      options: { alpha?: unknown },
      key: number,
    ) => {
      void options.alpha;
      return create(canvas, w, h, options, key);
    };
    const ctx = c.getContext("2d", {
      get alpha() {
        expect(c.getContext("webgpu")).toBeNull();
        c.width = 80;
        return true;
      },
    })!;
    expect(ctx.kind).toBe("ctx");
    expect(ctx.size).toEqual([80, 150]);
    expect(c.getContext("webgpu")).toBeNull();
    expect(c.getContext("2d")).toBe(ctx);
    expect(gpu.calls).toEqual([]);
  });

  test("a 2D canvas cannot select WebGPU", () => {
    const gpu = fakeGPU();
    const { doc } = world();
    const c = doc.createElement("canvas");
    const context = c.getContext("2d");
    expect(c.getContext("webgpu")).toBeNull();
    expect(c.getContext("2d")).toBe(context);
    expect(gpu.calls).toEqual([]);
  });

  test("unsupported and failed native requests leave the mode available", () => {
    const { doc } = world();
    const c = doc.createElement("canvas");
    expect(c.getContext("webgl")).toBeNull();
    expect(c.getContext("webgpu")).toBeNull();
    expect(c.getContext("2d")!.kind).toBe("ctx");
    fakeGPU();
    globals.__rjs_webgpu_context = () => null;
    const failed = doc.createElement("canvas");
    expect(failed.getContext("webgpu")).toBeNull();
    expect(failed.getContext("2d")!.kind).toBe("ctx");
    globals.__rjs_webgpu_context = () => {
      throw new Error("allocation failed");
    };
    const throwing = doc.createElement("canvas");
    expect(() => throwing.getContext("webgpu")).toThrow("allocation failed");
    expect(throwing.getContext("2d")!.kind).toBe("ctx");
  });

  test("GPU support is detected independently from the native 2D context factory", () => {
    const gpu = fakeGPU();
    delete globals.__rjs_canvas_context;
    const { doc } = world();
    const c = doc.createElement("canvas");
    expect(c.getContext("webgpu")!.kind).toBe("gpu");
    expect(gpu.calls).toHaveLength(1);
  });

  test("resizes GPU contexts for property, renderer and repeated-dimension writes", () => {
    const gpu = fakeGPU();
    const { rt, doc } = world();
    const c = doc.createElement("canvas");
    const context = c.getContext("webgpu")!;
    c.width = 80;
    c.width = 80;
    rt.patchProp(c, "height", null, 50);
    c.removeAttribute("height");
    expect(gpu.calls.filter((x) => x[0] === "resize").map((x) => x.slice(2))).toEqual([
      [80, 150],
      [80, 150],
      [80, 50],
      [80, 150],
    ]);
    expect(context.size).toEqual([80, 150]);
    expect(c.getContext("webgpu")).toBe(context);
    expect(natives.calls.filter((x) => x[0] === "resize")).toEqual([]);
  });

  test("source reads preserve a GPU context and do not select a mode for a blank canvas", () => {
    const gpu = fakeGPU();
    const { doc } = world();
    const c = doc.createElement("canvas");
    const hook = natives.hook()!;
    expect(hook(c)).toBeNull();
    const context = c.getContext("webgpu");
    expect(hook(c)).toBe(context);
    expect(gpu.calls).toHaveLength(1);
    expect(natives.calls.filter((x) => x[0] === "context")).toEqual([]);
  });

  test("encodes GPU snapshots through native capture and invokes blob callbacks asynchronously", async () => {
    const gpu = fakeGPU();
    const { doc } = world();
    const c = doc.createElement("canvas");
    const context = c.getContext("webgpu");
    expect(c.toDataURL("image/jpeg", 0.25)).toBe("data:image/jpeg;gpu");
    let got: unknown = "pending";
    c.toBlob((value) => (got = value), "image/webp", 0.75);
    expect(got).toBe("pending");
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(got).toEqual({ gpuBlob: "image/webp" });
    expect(gpu.calls.filter((x) => x[0] !== "context")).toEqual([
      ["dataUrl", context, "image/jpeg", 0.25],
      ["blob", context, "image/webp", 0.75],
    ]);
    expect(natives.calls.filter((x) => x[0] === "dataUrl" || x[0] === "blob")).toEqual([]);
    c.width = 0;
    got = "pending";
    c.toBlob((value) => (got = value));
    expect(got).toBe("pending");
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(got).toBeNull();
  });
});
