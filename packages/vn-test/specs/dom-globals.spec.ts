// The DOM's collections and interface objects in the real engine
// (docs/RUNTIME.md §5.24, research/reports/m26/dom-globals-1.md).
//
// In the bun harness (packages/vue-native/test/runtime-collections.test.ts)
// the runtime builds Node, Element and HTMLElement itself. Here they are
// redjs's: `Element`, `HTMLElement` and `SVGElement` are native classes, and
// `Node` was an alias of `Element`, so what this checks is the runtime's
// re-wiring of the engine's own objects: Node under EventTarget and over
// Element, the text mirrors under CharacterData, `instanceof HTMLElement`
// false for an `<svg>`, redjs's bootstrap HTMLVideoElement under
// HTMLMediaElement. The expected values are Chrome 156's answers for the same
// page, except `Array.isArray(childNodes)`, the documented divergence.
//
//   bun test packages/vn-test/specs/dom-globals.spec.ts

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/dom-globals/src/main.ts",
  window: [420, 300],
  label: "dom-globals",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;

describe("DOM interfaces in the real engine", () => {
  test(
    "Node, NodeList, HTMLCollection, the SVG and media interfaces",
    async () => {
      await app.locator("#icon").waitFor();
      const p = JSON.parse(String(await app.eval("__VN_TEST_DOM__.probe()")));
      expect(p.nodeIsOwn).toBe(true);
      expect(p.constants).toEqual([1, 3, 8, 11]);
      expect(p.chain).toEqual([true, true, true, true]);
      expect(p.textIsNode).toEqual([true, true, true, true]);
      expect(p.elementIsNode).toEqual([true, true, true, true]);
      expect(p.childNodes).toEqual([true, "[object NodeList]", true, true]);
      expect(p.children).toEqual([true, true, true, true, "0,1"]);
      expect(p.live).toEqual([
        { kids: 2, byTag: 2 },
        { kids: 3, byTag: 3, nodes: 4 },
      ]);
      expect(p.qsa).toEqual([true, false, 2, 2, 3]);
      expect(p.byClass).toEqual([2, 2]);
      expect(p.byName).toEqual([true, 1]);
      expect(p.svg).toEqual(["svg", "path", "http://www.w3.org/2000/svg", true, false, true, true, true]);
      expect(p.svgPainted).toBe(20);
      expect(p.svgNS).toBe("svg");
      expect(p.media).toEqual([true, true, true, false, true]);
      expect(p.brands).toEqual([true, true, "function", "function", true]);
      expect(p.events).toEqual(["opacity", true]);
      expect(p.touch).toEqual([false, false, false, 0, "function", "function", "function", 1]);
    },
    T,
  );
});
