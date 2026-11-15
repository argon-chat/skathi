// runtime-dom's names that are not in runtime-core (render, useCssVars, the custom-element and SSR entry points)
// and the IntersectionObserver updated from painted bounds after layout.

import { describe, expect, test } from "bun:test";
import { defineComponent, h, nextTick, ref } from "@vue/runtime-core";
import {
  createRuntime,
  type MirrorElement,
  type MirrorNode,
  type RealmDocument,
  type RealmObserver,
  type RealmObserverEntry,
} from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
type Parent = MirrorElement & {
  appendChild(child: MirrorNode): MirrorNode;
  removeChild(child: MirrorNode): MirrorNode;
  insertAdjacentElement(where: string, element: MirrorNode): unknown;
};
interface CompatDocument extends RealmDocument {
  head: Parent;
  createElement(tag: string): Parent;
  getElementsByTagName(tag: string): { [index: number]: Parent | undefined; item(index: number): Parent | null };
}
const documentOf = (rt: Rt): CompatDocument => rt.global.document as CompatDocument;

interface IntersectionEntry extends RealmObserverEntry {
  isIntersecting: boolean;
  intersectionRatio: number;
}
interface IntersectionObserverLike extends RealmObserver {
  thresholds: number[];
  rootMargin: string;
  takeRecords(): IntersectionEntry[];
}
type IntersectionObserverCtor = new (
  callback: (entries: IntersectionEntry[]) => unknown,
  options?: unknown,
) => IntersectionObserverLike;
const intersectionObserver = (rt: Rt): IntersectionObserverCtor =>
  rt.global.IntersectionObserver as unknown as IntersectionObserverCtor;

const frame = () => new Promise((r) => setTimeout(r, 4));

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
      expect(rt[name as keyof typeof rt]).toBeDefined();
    }
  });

  test("render() mounts a vnode into a mirror container and unmounts it with null", () => {
    const rt = createRuntime();
    const container = rt.nodeOps.createElement("div");
    rt.render(h("span", { class: "x" }, "hi"), container);
    expect(container.childNodes.length).toBe(1);
    expect((container.childNodes[0] as MirrorElement).tagName).toBe("SPAN");
    rt.render(null, container);
    expect(container.childNodes.length).toBe(0);
  });

  test("the custom-element and SSR entry points say what they are instead of failing to build", () => {
    const rt = createRuntime();
    const El = rt.defineCustomElement({ render: () => null });
    expect(() => new El()).toThrow();
    expect(rt.useHost()).toBeNull();
    expect(rt.useShadowRoot()).toBeNull();
    const app = rt.createSSRApp({ render: () => null });
    expect(typeof app.mount).toBe("function");
    rt.initDirectivesForSSR();
  });
});

describe("useCssVars", () => {
  test("writes the v-bind() custom properties onto the component's root and follows the state", async () => {
    const rt = createRuntime();
    const color = ref("red");
    let el!: MirrorElement;
    const Comp = defineComponent({
      setup() {
        rt.useCssVars(() => ({ "abc123-color": color.value, "abc123-size": 4 }));
        return () => h("div", { ref: (e: unknown) => (el = e as MirrorElement) }, "x");
      },
    });
    rt.createApp(Comp).mount();
    await nextTick();
    const styleOf = (key: string) => rt.host.styles.filter((s) => s.id === el.__skId && s.key === key).at(-1)?.value;
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
          h("i", { ref: (e: unknown) => void (e && ids.push((e as MirrorElement).__skId)) }),
          h("b", { ref: (e: unknown) => void (e && ids.push((e as MirrorElement).__skId)) }),
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
  // style-inject's exact sequence (vue3-spinners): insert the empty <style>, THEN append the text node.
  function inject(rt: Rt, css: string) {
    const d = documentOf(rt);
    const s = d.createElement("style");
    s.setAttribute?.("type", "text/css");
    d.getElementsByTagName("head")[0]!.insertAdjacentElement("beforeend", s);
    s.appendChild(d.createTextNode(css));
    return s;
  }

  test("before mount it rides the first stylesheet, after the bundle's CSS", async () => {
    const rt = createRuntime({ globals: { __SK_CSS__: [".bundle{}"] } });
    inject(rt, ".spinner{}");
    expect(rt.host.sheets).toEqual([]);
    rt.createApp({ render: () => null }).mount();
    expect(rt.host.sheets).toEqual([".bundle{}\n.spinner{}"]);
  });

  test("a stylesheet populated before head insertion retains text and keyframes (Vaul order)", async () => {
    const rt = createRuntime({ globals: { __SK_CSS__: [".bundle{}"] } });
    const doc = documentOf(rt);
    const style = doc.createElement("style");
    const css =
      "[data-drawer]{animation:slide .5s}@keyframes slide{from{transform:translateY(100%)}to{transform:translateY(0)}}";
    const child = doc.createTextNode(css);
    expect(style.appendChild(child)).toBe(child);
    expect(style.textContent).toBe(css);
    expect(style.text).toBe(css);
    doc.head.appendChild(style);
    rt.createApp({ render: () => null }).mount();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n" + css);
    style.text = ".replacement{}";
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n.replacement{}");
    expect(style.textContent).toBe(".replacement{}");
  });

  test("after mount: an edit re-sets the sheet once per microtask, and removal takes it out", async () => {
    const rt = createRuntime({ globals: { __SK_CSS__: [".bundle{}"] } });
    rt.createApp({ render: () => null }).mount();
    const s = inject(rt, ".a{}");
    s.appendChild(documentOf(rt).createTextNode(".b{}"));
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n.a{}.b{}");
    expect(rt.host.sheets.length).toBe(2); // the mount's, then ONE for both edits
    s.textContent = ".c{}";
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n.c{}");
    expect(s.parentNode).toBe(documentOf(rt).head);
    (s.parentNode as Parent).removeChild(s);
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}");
    expect(s.parentNode).toBeNull();
  });

  test("each injected sheet reaches the host as its own text, not glued onto the bundle's", async () => {
    const rt = createRuntime({ globals: { __SK_CSS__: [".bundle{}"] } });
    inject(rt, ".a{}");
    rt.createApp({ render: () => null }).mount();
    inject(rt, ".b{}");
    await Promise.resolve();
    expect(rt.host.sheetParts).toEqual([
      [".bundle{}", ".a{}"],
      [".bundle{}", ".a{}", ".b{}"],
    ]);
  });

  test("getElementsByTagName and createTextNode are there", () => {
    const rt = createRuntime();
    const d = documentOf(rt);
    expect(d.getElementsByTagName("head")[0]).toBe(d.head);
    expect(d.getElementsByTagName("head").item(1)).toBeNull();
    const t = d.createTextNode("hi");
    expect(t.textContent).toBe("hi");
  });
});

describe("IntersectionObserver, updated after layout and delivered from a task", () => {
  function world(bounds: string) {
    const rt = createRuntime();
    rt.createApp({ render: () => null }).mount();
    const root = documentOf(rt).documentElement;
    rt.host.boundsById.set(root.__skId, "0,0,800,600");
    const el = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(el, root, null);
    rt.host.boundsById.set(el.__skId, bounds);
    return { rt, root, el };
  }
  async function frameStep(rt: Rt) {
    rt.host.layout.step();
    await frame();
  }

  test("replaces redjs's inert observer, and reports once on observe and again only on a change", async () => {
    const { rt, el } = world("100,100,200,100");
    const seen: IntersectionEntry[] = [];
    const ob = new (intersectionObserver(rt))((entries) => seen.push(...entries));
    ob.observe(el);
    expect(rt.host.layout.pending).toBe(true);
    rt.host.layout.step();
    expect(seen.length).toBe(0);
    await frame();
    expect(seen.length).toBe(1);
    expect(seen[0]!.isIntersecting).toBe(true);
    expect(seen[0]!.intersectionRatio).toBe(1);
    expect(seen[0]!.target).toBe(el);
    expect(seen[0]!.boundingClientRect).toMatchObject({ x: 100, y: 100, width: 200, height: 100 });
    expect(seen[0]!.rootBounds).toMatchObject({ x: 0, y: 0, width: 800, height: 600 });
    expect(seen[0]).toBeInstanceOf(rt.global.IntersectionObserverEntry);
    await frameStep(rt);
    expect(seen.length).toBe(1);
    rt.host.boundsById.set(el.__skId, "100,700,200,100");
    await frameStep(rt);
    expect(seen.length).toBe(2);
    expect(seen[1]!.isIntersecting).toBe(false);
    expect(seen[1]!.intersectionRatio).toBe(0);
    ob.disconnect();
  });

  test("intersectionRatio and thresholds: half visible crosses 0.5 but not 0.75", async () => {
    const { rt, el } = world("0,550,100,100"); // 50 of 100 px inside a 600-high viewport
    const seen: IntersectionEntry[] = [];
    const ob = new (intersectionObserver(rt))((e) => seen.push(...e), { threshold: [0.75, 0, 0.5] });
    expect(ob.thresholds).toEqual([0, 0.5, 0.75]);
    ob.observe(el);
    await frameStep(rt);
    expect(seen.length).toBe(1);
    expect(seen[0]!.intersectionRatio).toBeCloseTo(0.5);
    expect(seen[0]!.intersectionRect).toMatchObject({ y: 550, height: 50, width: 100 });
    rt.host.boundsById.set(el.__skId, "0,540,100,100");
    await frameStep(rt);
    expect(seen.length).toBe(1);
    rt.host.boundsById.set(el.__skId, "0,520,100,100");
    await frameStep(rt);
    expect(seen.length).toBe(2);
    ob.disconnect();
  });

  test("rootMargin grows the root, in px and %, and is reported normalized", async () => {
    const { rt, el } = world("0,650,100,40"); // 50 px below the viewport
    const seen: IntersectionEntry[] = [];
    const ob = new (intersectionObserver(rt))((e) => seen.push(...e), { rootMargin: "0px 0px 10%" });
    expect(ob.rootMargin).toBe("0px 0px 10% 0px");
    ob.observe(el);
    await frameStep(rt);
    expect(seen[0]!.isIntersecting).toBe(true);
    expect(seen[0]!.rootBounds).toMatchObject({ height: 660 });
    expect(() => new (intersectionObserver(rt))(() => {}, { rootMargin: "10em" })).toThrow();
    expect(() => new (intersectionObserver(rt))(() => {}, { threshold: 2 })).toThrow(RangeError);
    ob.disconnect();
  });

  test("an element root clips to its own box, and a target outside it never intersects", async () => {
    const { rt, root, el } = world("0,0,100,100");
    const scroller = rt.nodeOps.createElement("section");
    rt.nodeOps.insert(scroller, root, null);
    rt.host.boundsById.set(scroller.__skId, "0,0,300,300");
    const inside = rt.nodeOps.createElement("p");
    rt.nodeOps.insert(inside, scroller, null);
    rt.host.boundsById.set(inside.__skId, "0,400,100,50"); // inside the window, below the scroller's box
    const seen: IntersectionEntry[] = [];
    const ob = new (intersectionObserver(rt))((e) => seen.push(...e), { root: scroller });
    ob.observe(inside);
    ob.observe(el); // not a descendant of the root
    await frameStep(rt);
    expect(seen.find((e) => e.target === inside)!.isIntersecting).toBe(false);
    expect(seen.find((e) => e.target === el)!.isIntersecting).toBe(false);
    rt.host.boundsById.set(inside.__skId, "0,200,100,50");
    await frameStep(rt);
    expect(seen.filter((e) => e.target === inside).at(-1)!.isIntersecting).toBe(true);
    ob.disconnect();
  });

  test("a target with no box after the frame's layout is not intersecting, at once", async () => {
    const { rt, root } = world("0,0,1,1");
    const late = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(late, root, null);
    const hidden = rt.nodeOps.createElement("div"); // detached: never painted
    const seen: IntersectionEntry[] = [];
    const ob = new (intersectionObserver(rt))((e) => seen.push(...e));
    ob.observe(late);
    ob.observe(hidden);
    await frameStep(rt);
    expect(seen.map((e) => [e.target === late, e.isIntersecting])).toEqual([
      [true, false],
      [false, false],
    ]);
    rt.host.boundsById.set(late.__skId, "10,10,50,50"); // laid out in a later frame
    await frameStep(rt);
    expect(seen.slice(2).map((e) => [e.target === late, e.isIntersecting])).toEqual([[true, true]]);
    expect(ob.takeRecords()).toEqual([]);
    ob.disconnect();
  });

  test("takeRecords() empties the queue before the task delivers it", async () => {
    const { rt, el } = world("100,100,200,100");
    const seen: IntersectionEntry[] = [];
    const ob = new (intersectionObserver(rt))((e) => seen.push(...e));
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
    const ob = new (intersectionObserver(rt))(() => {});
    ob.observe(el);
    await frameStep(rt);
    expect(rt.host.layout.observing).toBe(true);
    ob.unobserve(el);
    await frameStep(rt);
    expect(rt.host.layout.observing).toBe(false);
    expect(rt.host.layout.step()).toBe(-1);
  });
});
