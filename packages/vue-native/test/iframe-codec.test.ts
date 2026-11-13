// The wire form of an `<iframe>`'s postMessage payloads (src/iframe.ts): what
// crosses, what comes back, and what is refused in the sender.

import { describe, expect, test } from "bun:test";
import { decodeMessage, encodeMessage, fromBase64, toBase64 } from "../src/iframe";

/** Through the text channel, as a message travels. */
const cross = (v: unknown) => decodeMessage(JSON.parse(JSON.stringify(encodeMessage(v))));

describe("what crosses", () => {
  test("JSON's own types come back equal", () => {
    const v = { type: "handshake", id: 7, ok: true, none: null, list: [1, "two", [3]], nested: { a: { b: "c" } } };
    expect(cross(v)).toEqual(v);
  });

  test("what JSON loses survives: undefined, holes, NaN, infinities, -0, bigint", () => {
    const out = cross({
      u: undefined,
      n: NaN,
      i: Infinity,
      m: -Infinity,
      z: -0,
      big: 2n ** 70n,
      hole: [1, , 3],
    }) as Record<string, unknown>;
    expect("u" in out && out.u === undefined).toBe(true);
    expect(Number.isNaN(out.n)).toBe(true);
    expect([out.i, out.m]).toEqual([Infinity, -Infinity]);
    expect(Object.is(out.z, -0)).toBe(true);
    expect(out.big).toBe(2n ** 70n);
    expect(out.hole).toEqual([1, undefined, 3]);
  });

  test("dates, maps and sets keep their kind", () => {
    const when = new Date(1_700_000_000_000);
    const out = cross({ when, bad: new Date(NaN), m: new Map([[1, { a: 1 }]]), s: new Set(["x", "y"]) }) as Record<
      string,
      unknown
    >;
    expect(out.when).toEqual(when);
    expect(Number.isNaN((out.bad as Date).getTime())).toBe(true);
    expect(out.m).toEqual(new Map([[1, { a: 1 }]]));
    expect(out.s).toEqual(new Set(["x", "y"]));
  });

  test("buffers and their views, by kind and by window", () => {
    const bytes = new Uint8Array([0, 1, 2, 253, 254, 255]);
    const back = cross({
      buf: bytes.buffer,
      view: new Uint8Array(bytes.buffer, 2, 3),
      f: new Float32Array([1.5, -2.25]),
      d: new DataView(bytes.buffer, 1, 2),
    }) as Record<string, unknown>;
    expect(new Uint8Array(back.buf as ArrayBuffer)).toEqual(bytes);
    expect(back.view).toEqual(new Uint8Array([2, 253, 254]));
    expect(back.f).toEqual(new Float32Array([1.5, -2.25]));
    expect((back.d as DataView).getUint8(1)).toBe(2);
  });

  test("a class instance crosses as its own properties, as a browser clones it", () => {
    class Point {
      constructor(
        public x: number,
        public y: number,
      ) {}
      norm() {
        return Math.hypot(this.x, this.y);
      }
    }
    const out = cross(new Point(3, 4));
    expect(out).toEqual({ x: 3, y: 4 });
    expect(Object.getPrototypeOf(out)).toBe(Object.prototype);
  });

  test("a key named __proto__ is data, not a prototype", () => {
    const out = cross(JSON.parse('{"__proto__": {"polluted": true}, "a": 1}')) as Record<string, unknown>;
    expect(Object.getPrototypeOf(out)).toBe(Object.prototype);
    expect(Object.keys(out)).toEqual(["__proto__", "a"]);
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });

  test("the same object twice is two copies; only a cycle is refused", () => {
    const shared = { n: 1 };
    expect(cross([shared, shared])).toEqual([{ n: 1 }, { n: 1 }]);
    const loop: Record<string, unknown> = {};
    loop.self = loop;
    expect(() => encodeMessage(loop)).toThrow(/could not be cloned/);
  });
});

describe("what is refused, in the sender", () => {
  test("functions and symbols are a DataCloneError", () => {
    for (const bad of [() => 1, Symbol("s"), { deep: [{ fn() {} }] }]) {
      let caught: unknown;
      try {
        encodeMessage(bad);
      } catch (e) {
        caught = e;
      }
      expect((caught as Error)?.name).toBe("DataCloneError");
    }
  });

  test("a platform object is refused when the caller can name one", () => {
    const node = { nodeType: 1 };
    expect(() => encodeMessage({ el: node }, (o) => o === node)).toThrow(/could not be cloned/);
    expect(cross({ el: node })).toEqual({ el: { nodeType: 1 } });
  });
});

describe("a wire value that encodeMessage did not write", () => {
  test("is a TypeError, never a value", () => {
    for (const bad of [{ plain: "object" }, ["?"], ["t", "Function", ""], ["b", "!!!!"]]) {
      expect(() => decodeMessage(bad)).toThrow(TypeError);
    }
  });
});

describe("base64", () => {
  test("round-trips every tail length and a megabyte", () => {
    for (const n of [0, 1, 2, 3, 4, 5, 1024, 1 << 20]) {
      const bytes = new Uint8Array(n);
      for (let i = 0; i < n; i++) bytes[i] = (i * 31 + 7) & 255;
      const text = toBase64(bytes);
      expect(text.length).toBe(Math.ceil(n / 3) * 4);
      expect(fromBase64(text)).toEqual(bytes);
    }
    expect(toBase64(new TextEncoder().encode("hello"))).toBe("aGVsbG8=");
  });
});
