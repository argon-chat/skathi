// WebIDL interface objects the realm lacks. Each is a browser's: a constructor that throws "Illegal constructor"
// (unless a browser lets you construct it), with `instanceof` true for the objects this realm already has. Libraries
// record them at class definition (tsyringe's `design:paramtypes` = `[Window]`).

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Ctor = { new (...a: unknown[]): unknown; (...a: unknown[]): unknown; prototype: object; name: string };
const g = (rt: ReturnType<typeof createRuntime>, name: string) => (rt.global as Record<string, Ctor>)[name];

describe("Window", () => {
  test("exists, cannot be constructed, and the global is one", () => {
    const rt = createRuntime();
    const Window = g(rt, "Window");
    expect(typeof Window).toBe("function");
    expect(Window.name).toBe("Window");
    expect(() => new Window()).toThrow("Illegal constructor");
    expect(() => Window()).toThrow("Illegal constructor");
    expect(rt.global instanceof Window).toBe(true);
    expect({} instanceof Window).toBe(false);
    expect((rt.global as { constructor: unknown }).constructor).toBe(Window);
    expect(Window.prototype.constructor).toBe(Window);
  });

  test("the tsyringe pattern: a class whose parameter types name Window", () => {
    const rt = createRuntime();
    const Window = g(rt, "Window");
    const meta = new Map<unknown, unknown[]>();
    // `__metadata("design:paramtypes", [Window])`, then `container.resolve(Window)`
    // with `{ provide: Window, useValue: window }` registered.
    class Pay {
      constructor(public w: unknown) {}
    }
    meta.set(Pay, [Window]);
    const registry = new Map<unknown, unknown>([[Window, rt.global]]);
    const pay = new Pay(...(meta.get(Pay) ?? []).map((t) => registry.get(t)));
    expect(pay.w instanceof Window).toBe(true);
  });
});

describe("the other interfaces", () => {
  test("Document / HTMLDocument / Location answer for the realm's objects", () => {
    const rt = createRuntime();
    const doc = (rt.global as { document?: unknown }).document;
    if (doc) {
      expect(doc instanceof g(rt, "Document")).toBe(true);
      expect(doc instanceof g(rt, "HTMLDocument")).toBe(true);
    }
    expect({} instanceof g(rt, "Document")).toBe(false);
    expect(() => new (g(rt, "HTMLDocument"))()).toThrow("Illegal constructor");
    for (const name of ["Location", "History", "Screen", "CharacterData", "ShadowRoot", "CSSStyleDeclaration"]) {
      expect(typeof g(rt, name)).toBe("function");
      expect(() => new (g(rt, name))()).toThrow("Illegal constructor");
    }
  });

  test("Text / Comment / CharacterData tell mirror text from comments, and Text constructs one", () => {
    const rt = createRuntime();
    const text = rt.nodeOps.createText("hi");
    const comment = rt.nodeOps.createComment("");
    const div = rt.nodeOps.createElement("div");
    expect(text instanceof g(rt, "Text")).toBe(true);
    expect(text instanceof g(rt, "CharacterData")).toBe(true);
    expect(comment instanceof g(rt, "Text")).toBe(false);
    expect(comment instanceof g(rt, "Comment")).toBe(true);
    expect(div instanceof g(rt, "CharacterData")).toBe(false);
    const made = new (g(rt, "Text"))("made") as { nodeType: number; textContent: string };
    expect(made instanceof g(rt, "Text")).toBe(true);
    expect(made.textContent).toBe("made");
  });

  test("CSSStyleDeclaration is what el.style is; DOMRect is what getBoundingClientRect returns", () => {
    const rt = createRuntime();
    const div = rt.nodeOps.createElement("div") as unknown as {
      style: unknown;
      getBoundingClientRect(): Record<string, unknown>;
    };
    expect(div.style instanceof g(rt, "CSSStyleDeclaration")).toBe(true);
    const r = div.getBoundingClientRect();
    expect(r instanceof g(rt, "DOMRect")).toBe(true);
    expect(r instanceof g(rt, "DOMRectReadOnly")).toBe(true);
    expect({ ...r }.width).toBe(0);
    const made = new (g(rt, "DOMRect"))(1, 2, 3, 4) as Record<string, number> & { toJSON(): unknown };
    expect([made.x, made.y, made.right, made.bottom]).toEqual([1, 2, 4, 6]);
    expect(made.toJSON()).toEqual({ x: 1, y: 2, width: 3, height: 4, top: 2, right: 4, bottom: 6, left: 1 });
  });
});
