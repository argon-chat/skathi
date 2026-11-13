// `document.elementFromPoint` / `elementsFromPoint` — the JS half. The host's
// half (the topmost element, `pointer-events`, `visibility`, paint order) is
// crates/vn-tree/src/pointer.rs and is driven end to end by
// packages/vn-test/specs/hit-target.spec.ts; here the host is `host.hit`.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

function world() {
  const rt = createRuntime();
  rt.createApp({ render: () => null }).mount();
  const doc = rt.global.document;
  const body = doc.body;
  const outer = rt.nodeOps.createElement("div");
  rt.nodeOps.insert(outer, body, null);
  const inner = rt.nodeOps.createElement("span");
  rt.nodeOps.insert(inner, outer, null);
  const text = rt.nodeOps.createText("t");
  rt.nodeOps.insert(text, inner, null);
  return { rt, doc, body, outer, inner, text };
}

describe("document.elementFromPoint", () => {
  test("answers the host's topmost element, and null outside the viewport", () => {
    const { rt, doc, outer, inner } = world();
    rt.host.hit.answer = (x, _y, all) => (x < 0 ? "" : all ? `${inner.__vnId},${outer.__vnId}` : `${inner.__vnId}`);
    expect(doc.elementFromPoint(10, 20)).toBe(inner);
    expect(doc.elementFromPoint(-5, 20)).toBeNull();
    expect(rt.host.hit.calls).toEqual([
      [10, 20, 0],
      [-5, 20, 0],
    ]);
  });

  test("elementsFromPoint lists them topmost first; elements only", () => {
    const { rt, doc, outer, inner, text } = world();
    rt.host.hit.answer = () => `${inner.__vnId},${text.__vnId},${outer.__vnId}`;
    expect(doc.elementsFromPoint(1, 2)).toEqual([inner, outer]);
    rt.host.hit.answer = () => "";
    expect(doc.elementsFromPoint(1, 2)).toEqual([]);
  });

  test("a non-finite coordinate throws, as a WebIDL double does", () => {
    const { doc } = world();
    expect(() => doc.elementFromPoint(Number.NaN, 0)).toThrow(TypeError);
    expect(() => doc.elementsFromPoint(0, Number.POSITIVE_INFINITY)).toThrow(TypeError);
    // Strings convert, as `Number()` would.
    expect(() => doc.elementFromPoint("3" as unknown as number, "4" as unknown as number)).not.toThrow();
  });
});
