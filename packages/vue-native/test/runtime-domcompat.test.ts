// runtime-dom's names that are not in runtime-core (render, useCssVars, the
// custom-element and SSR entry points) and the IntersectionObserver updated
// from painted bounds after layout — research/reports/m26/client-gap.md §3.2.
//
//   bun test test/runtime-domcompat.test.ts      (from packages/vue-native)

import { describe, expect, test } from "bun:test";
import { defineComponent, h, nextTick, ref } from "@vue/runtime-core";
import { createRuntime } from "./runtime-harness";

const frame = () => new Promise((r) => setTimeout(r, 4));
const frames = async (n: number) => {
  for (let i = 0; i < n; i++) await frame();
};

describe("runtime-dom's exports", () => {
  test("every name runtime-dom adds over runtime-core is exported", () => {
    const rt = createRuntime();
    for (const name of [
      "render",
      "useCssVars",
      "useCssModule",
      "VueElement",
      "defineCustomElement",
      "createSSRApp",
      "hydrate",
      "useHost",
      "useShadowRoot",
      "initDirectivesForSSR",
      "nodeOps",
      "patchProp",
    ]) {
      expect(rt[name]).toBeDefined();
    }
  });

  test("render() mounts a vnode into a mirror container and unmounts it with null", () => {
    const rt = createRuntime();
    const container = rt.nodeOps.createElement("div");
    rt.render(h("span", { class: "x" }, "hi"), container);
    expect(container.childNodes.length).toBe(1);
    expect(container.childNodes[0].tagName).toBe("SPAN");
    rt.render(null, container);
    expect(container.childNodes.length).toBe(0);
  });

  test("the custom-element and SSR entry points say what they are instead of failing to build", () => {
    const rt = createRuntime();
    const El = rt.defineCustomElement({ render: () => null });
    expect(() => new El()).toThrow(/custom elements/);
    expect(rt.useHost()).toBeNull();
    expect(rt.useShadowRoot()).toBeNull();
    const app = rt.createSSRApp({ render: () => null });
    expect(typeof app.mount).toBe("function");
    rt.initDirectivesForSSR();
    const advisories = rt.advisories().join("\n");
    expect(advisories).toContain("defineCustomElement");
    expect(advisories).toContain("createSSRApp");
  });
});

describe("useCssVars", () => {
  test("writes the v-bind() custom properties onto the component's root and follows the state", async () => {
    const rt = createRuntime();
    const color = ref("red");
    let el: any = null;
    const Comp = defineComponent({
      setup() {
        // What compileScript emits for `color: v-bind(color)` in a <style>.
        rt.useCssVars(() => ({ "abc123-color": color.value, "abc123-size": 4 }));
        return () => h("div", { ref: (e: unknown) => (el = e) }, "x");
      },
    });
    rt.createApp(Comp).mount();
    await nextTick();
    const styleOf = (key: string) => rt.host.styles.filter((s) => s.id === el.__vnId && s.key === key).at(-1)?.value;
    expect(styleOf("--abc123-color")).toBe("red");
    expect(styleOf("--abc123-size")).toBe("4");

    color.value = "blue";
    await nextTick();
    await nextTick();
    expect(styleOf("--abc123-color")).toBe("blue");
  });

  test("a fragment root gets the variables on every element child; null becomes `initial`", async () => {
    const rt = createRuntime();
    const ids: number[] = [];
    const Comp = defineComponent({
      setup() {
        rt.useCssVars(() => ({ "f-gap": null }));
        return () => [
          h("i", { ref: (e: any) => e && ids.push(e.__vnId) }),
          h("b", { ref: (e: any) => e && ids.push(e.__vnId) }),
        ];
      },
    });
    rt.createApp(Comp).mount();
    await nextTick();
    for (const id of new Set(ids)) {
      expect(rt.host.styles.find((s) => s.id === id && s.key === "--f-gap")?.value).toBe("initial");
    }
  });
});

describe("a <style> injected into document.head", () => {
  // style-inject's exact sequence (vue3-spinners runs it at module scope):
  // insert the empty <style>, THEN append the text node.
  function inject(rt: any, css: string) {
    const d = rt.global.document;
    const s = d.createElement("style");
    s.setAttribute?.("type", "text/css");
    d.getElementsByTagName("head")[0].insertAdjacentElement("beforeend", s);
    s.appendChild(d.createTextNode(css));
    return s;
  }

  test("before mount it rides the first stylesheet, after the bundle's CSS", async () => {
    const rt = createRuntime({ globals: { __VN_CSS__: [".bundle{}"] } });
    inject(rt, ".spinner{}");
    expect(rt.host.sheets).toEqual([]);
    rt.createApp({ render: () => null }).mount();
    expect(rt.host.sheets).toEqual([".bundle{}\n.spinner{}"]);
  });

  test("after mount: an edit re-sets the sheet once per microtask, and removal takes it out", async () => {
    const rt = createRuntime({ globals: { __VN_CSS__: [".bundle{}"] } });
    rt.createApp({ render: () => null }).mount();
    const s = inject(rt, ".a{}");
    s.appendChild(rt.global.document.createTextNode(".b{}"));
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n.a{}.b{}");
    expect(rt.host.sheets.length).toBe(2); // the mount's, then ONE for both edits
    s.textContent = ".c{}";
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n.c{}");
    expect(s.parentNode).toBe(rt.global.document.head);
    s.parentNode.removeChild(s);
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}");
    expect(s.parentNode).toBeNull();
  });

  test("each injected sheet reaches the host as its own text, not glued onto the bundle's", async () => {
    // The host parses (and keeps) one parse per distinct text: a concatenation
    // re-parsed the whole bundle sheet whenever an injected one changed.
    const rt = createRuntime({ globals: { __VN_CSS__: [".bundle{}"] } });
    inject(rt, ".a{}");
    rt.createApp({ render: () => null }).mount();
    inject(rt, ".b{}");
    await Promise.resolve();
    expect(rt.host.sheetParts).toEqual([
      [".bundle{}", ".a{}"],
      [".bundle{}", ".a{}", ".b{}"],
    ]);
  });

  test("getElementsByTagName and createTextNode are there, and a <link> stylesheet is an advisory", () => {
    const rt = createRuntime();
    const d = rt.global.document;
    expect(d.getElementsByTagName("head")[0]).toBe(d.head);
    expect(d.getElementsByTagName("head").item(1)).toBeNull();
    const t = d.createTextNode("hi");
    expect(t.textContent).toBe("hi");
    const link = Object.assign(d.createElement("link"), { rel: "stylesheet", href: "https://x.invalid/a.css" });
    d.head.appendChild(link);
    expect(rt.advisories().join("\n")).toContain("not loaded");
  });
});

describe("IntersectionObserver, updated after layout and delivered from a task", () => {
  /** An app root painted at 800x600, and one element in it at `bounds`. */
  function world(bounds: string) {
    const rt = createRuntime();
    rt.createApp({ render: () => null }).mount();
    const root = rt.global.document.documentElement;
    rt.host.boundsById.set(root.__vnId, "0,0,800,600");
    const el = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(el, root, null);
    rt.host.boundsById.set(el.__vnId, bounds);
    return { rt, root, el };
  }
  /** One frame: the host's step after layout, then the task it queued. */
  async function frameStep(rt: ReturnType<typeof createRuntime>) {
    rt.host.layout.step();
    await frame();
  }

  test("replaces redjs's inert observer, and reports once on observe and again only on a change", async () => {
    const { rt, el } = world("100,100,200,100");
    const seen: any[] = [];
    const ob = new rt.global.IntersectionObserver((entries: any[]) => seen.push(...entries));
    ob.observe(el);
    expect(rt.host.layout.pending).toBe(true);
    rt.host.layout.step();
    // Queued by the step, delivered from a task: not inside the step.
    expect(seen.length).toBe(0);
    await frame();
    expect(seen.length).toBe(1);
    expect(seen[0].isIntersecting).toBe(true);
    expect(seen[0].intersectionRatio).toBe(1);
    expect(seen[0].target).toBe(el);
    expect(seen[0].boundingClientRect).toMatchObject({ x: 100, y: 100, width: 200, height: 100 });
    expect(seen[0].rootBounds).toMatchObject({ x: 0, y: 0, width: 800, height: 600 });
    expect(seen[0]).toBeInstanceOf(rt.global.IntersectionObserverEntry);
    // No change -> no callback.
    await frameStep(rt);
    expect(seen.length).toBe(1);
    // Scrolled out below the viewport.
    rt.host.boundsById.set(el.__vnId, "100,700,200,100");
    await frameStep(rt);
    expect(seen.length).toBe(2);
    expect(seen[1].isIntersecting).toBe(false);
    expect(seen[1].intersectionRatio).toBe(0);
    ob.disconnect();
  });

  test("intersectionRatio and thresholds: half visible crosses 0.5 but not 0.75", async () => {
    const { rt, el } = world("0,550,100,100"); // 50 of 100 px inside a 600-high viewport
    const seen: any[] = [];
    const ob = new rt.global.IntersectionObserver((e: any[]) => seen.push(...e), { threshold: [0.75, 0, 0.5] });
    expect(ob.thresholds).toEqual([0, 0.5, 0.75]);
    ob.observe(el);
    await frameStep(rt);
    expect(seen.length).toBe(1);
    expect(seen[0].intersectionRatio).toBeCloseTo(0.5);
    expect(seen[0].intersectionRect).toMatchObject({ y: 550, height: 50, width: 100 });
    // 60% visible: still between 0.5 and 0.75 -> same threshold index, no entry.
    rt.host.boundsById.set(el.__vnId, "0,540,100,100");
    await frameStep(rt);
    expect(seen.length).toBe(1);
    // 80%: past 0.75 -> one entry.
    rt.host.boundsById.set(el.__vnId, "0,520,100,100");
    await frameStep(rt);
    expect(seen.length).toBe(2);
    ob.disconnect();
  });

  test("rootMargin grows the root, in px and %, and is reported normalized", async () => {
    const { rt, el } = world("0,650,100,40"); // 50 px below the viewport
    const seen: any[] = [];
    const ob = new rt.global.IntersectionObserver((e: any[]) => seen.push(...e), { rootMargin: "0px 0px 10%" });
    expect(ob.rootMargin).toBe("0px 0px 10% 0px");
    ob.observe(el);
    await frameStep(rt);
    // 10% of 600 = 60 px of margin below: the element at 650..690 is inside 0..660.
    expect(seen[0].isIntersecting).toBe(true);
    expect(seen[0].rootBounds).toMatchObject({ height: 660 });
    expect(() => new rt.global.IntersectionObserver(() => {}, { rootMargin: "10em" })).toThrow(/pixels or percent/);
    expect(() => new rt.global.IntersectionObserver(() => {}, { threshold: 2 })).toThrow(RangeError);
    ob.disconnect();
  });

  test("an element root clips to its own box, and a target outside it never intersects", async () => {
    const { rt, root, el } = world("0,0,100,100");
    const scroller = rt.nodeOps.createElement("section");
    rt.nodeOps.insert(scroller, root, null);
    rt.host.boundsById.set(scroller.__vnId, "0,0,300,300");
    const inside = rt.nodeOps.createElement("p");
    rt.nodeOps.insert(inside, scroller, null);
    rt.host.boundsById.set(inside.__vnId, "0,400,100,50"); // inside the window, below the scroller's box
    const seen: any[] = [];
    const ob = new rt.global.IntersectionObserver((e: any[]) => seen.push(...e), { root: scroller });
    ob.observe(inside);
    ob.observe(el); // not a descendant of the root
    await frameStep(rt);
    expect(seen.find((e) => e.target === inside).isIntersecting).toBe(false);
    expect(seen.find((e) => e.target === el).isIntersecting).toBe(false);
    rt.host.boundsById.set(inside.__vnId, "0,200,100,50");
    await frameStep(rt);
    expect(seen.filter((e) => e.target === inside).at(-1).isIntersecting).toBe(true);
    ob.disconnect();
  });

  test("a target with no box after the frame's layout is not intersecting, at once", async () => {
    const { rt, root } = world("0,0,1,1");
    const late = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(late, root, null);
    const hidden = rt.nodeOps.createElement("div"); // detached: never painted
    const seen: any[] = [];
    const ob = new rt.global.IntersectionObserver((e: any[]) => seen.push(...e));
    ob.observe(late);
    ob.observe(hidden);
    await frameStep(rt);
    // The step runs after layout, so "no box" is the answer, not "not yet".
    expect(seen.map((e) => [e.target === late, e.isIntersecting])).toEqual([
      [true, false],
      [false, false],
    ]);
    rt.host.boundsById.set(late.__vnId, "10,10,50,50"); // laid out in a later frame
    await frameStep(rt);
    expect(seen.slice(2).map((e) => [e.target === late, e.isIntersecting])).toEqual([[true, true]]);
    expect(ob.takeRecords()).toEqual([]);
    ob.disconnect();
  });

  test("takeRecords() empties the queue before the task delivers it", async () => {
    const { rt, el } = world("100,100,200,100");
    const seen: any[] = [];
    const ob = new rt.global.IntersectionObserver((e: any[]) => seen.push(...e));
    ob.observe(el);
    rt.host.layout.step();
    const taken = ob.takeRecords();
    expect(taken.length).toBe(1);
    await frame();
    expect(seen.length).toBe(0);
    ob.disconnect();
  });

  test("nothing observed: the step says so, and the host stops calling it", async () => {
    const { rt, el } = world("100,100,200,100");
    const ob = new rt.global.IntersectionObserver(() => {});
    ob.observe(el);
    await frameStep(rt);
    expect(rt.host.layout.observing).toBe(true);
    ob.unobserve(el);
    await frameStep(rt);
    expect(rt.host.layout.observing).toBe(false);
    expect(rt.host.layout.step()).toBe(-1);
  });
});
