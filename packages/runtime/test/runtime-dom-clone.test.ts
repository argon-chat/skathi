// `cloneNode` / `document.importNode`, the Node members on `Node.prototype`, and listeners added to `document.body`
// before the root exists: what DOM-driven UI libraries (Svelte's client runtime among them) rely on.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement, type MirrorNode } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
interface CloneDocument {
  createElement(tag: string): MirrorElement;
  createElementNS(ns: string, tag: string): MirrorElement;
  createTextNode(data: string): MirrorNode;
  createComment(data: string): MirrorNode;
  createDocumentFragment(): MirrorElement;
  importNode(node: unknown, deep?: boolean): MirrorNode;
  body: MirrorElement;
}
const docOf = (rt: Rt) => rt.global.document as unknown as CloneDocument;

describe("cloneNode", () => {
  test("a deep clone copies tag, attributes, class, inline style and the subtree into a loose copy", () => {
    const rt = createRuntime();
    const doc = docOf(rt);
    const el = doc.createElement("section");
    el.setAttribute("data-x", "1");
    el.setAttribute("class", "a b");
    el.setAttribute("style", "color: red");
    const p = doc.createElement("p");
    p.append(doc.createTextNode("hi"));
    el.append(p, doc.createComment("anchor"), doc.createTextNode(" "));

    const copy = el.cloneNode(true) as MirrorElement;
    expect(copy).not.toBe(el);
    expect(copy.tag).toBe("section");
    expect(copy.getAttribute("data-x")).toBe("1");
    expect(copy.getAttribute("class")).toBe("a b");
    expect(copy.getAttribute("style")).toBe(el.getAttribute("style"));
    expect(copy.childNodes.map((c) => c.nodeType)).toEqual([1, 8, 3]);
    const pCopy = copy.childNodes[0] as MirrorElement;
    expect(pCopy).not.toBe(p);
    expect(pCopy.textContent).toBe("hi");
    expect((copy.childNodes[1] as MirrorNode & { data: string }).data).toBe("anchor");
    expect(rt.host.tree.shape(copy.__skId)).toEqual(["p", "#comment", "#text: "]);
    expect(rt.looseById.get(copy.__skId)?.deref()).toBe(copy);
    expect(rt.looseById.get(pCopy.__skId)?.deref()).toBe(pCopy);
    expect(copy.parentNode).toBe(null);
    expect(rt.advisories()).toEqual([]);
  });

  test("a shallow clone has no children; an element's own text is a child too", () => {
    const rt = createRuntime();
    const el = docOf(rt).createElement("button");
    rt.nodeOps.setElementText(el, "Save");
    expect((el.cloneNode(false) as MirrorElement).textContent).toBe("");
    expect((el.cloneNode(true) as MirrorElement).textContent).toBe("Save");
  });

  test("text and SVG keep their kind and namespace", () => {
    const rt = createRuntime();
    const doc = docOf(rt);
    const text = doc.createTextNode("x").cloneNode() as MirrorNode & { data: string };
    expect(text.nodeType).toBe(3);
    expect(text.data).toBe("x");
    const svg = doc.createElementNS("http://www.w3.org/2000/svg", "linearGradient");
    const svgCopy = svg.cloneNode() as MirrorElement;
    expect(svgCopy.tag).toBe("linearGradient");
    expect((svgCopy as MirrorElement & { namespaceURI: string }).namespaceURI).toBe("http://www.w3.org/2000/svg");
  });

  test("a cloned fragment is a fragment, and inserting it moves its children", () => {
    const rt = createRuntime();
    const doc = docOf(rt);
    const frag = doc.createDocumentFragment();
    frag.append(doc.createElement("h1"), doc.createTextNode(" "), doc.createElement("button"));
    const copy = frag.cloneNode(true) as MirrorElement;
    expect(copy.nodeType).toBe(11);
    expect(copy.childNodes.length).toBe(3);
    expect(frag.childNodes.length).toBe(3);

    rt.createApp({ render: () => null }).mount();
    doc.body.append(copy);
    expect(copy.childNodes.length).toBe(0);
    expect(rt.host.tree.shape(doc.body.__skId).slice(-3)).toEqual(["h1", "#text: ", "button"]);
  });

  test("document.importNode clones, and refuses what is not a node", () => {
    const rt = createRuntime();
    const doc = docOf(rt);
    const el = doc.createElement("div");
    el.append(doc.createElement("span"));
    const copy = doc.importNode(el, true) as MirrorElement;
    expect(copy).not.toBe(el);
    expect(copy.childNodes.length).toBe(1);
    expect(() => doc.importNode({})).toThrow(TypeError);
  });
});

describe("Node.prototype", () => {
  test("carries the tree accessors, which work on elements and text", () => {
    const rt = createRuntime();
    const doc = docOf(rt);
    const proto = (rt.global.Node as { prototype: object }).prototype;
    const first = Object.getOwnPropertyDescriptor(proto, "firstChild")?.get;
    const next = Object.getOwnPropertyDescriptor(proto, "nextSibling")?.get;
    expect(typeof first).toBe("function");
    expect(typeof next).toBe("function");
    const el = doc.createElement("div");
    const a = doc.createTextNode("a");
    const b = doc.createElement("b");
    el.append(a, b);
    expect(first!.call(el)).toBe(a);
    expect(next!.call(a)).toBe(b);
    expect(next!.call(b)).toBe(null);
    expect(el instanceof (rt.global.Node as unknown as new () => object)).toBe(true);
  });
});

describe("document.body before mount", () => {
  test("a listener added before the root exists is installed on it", () => {
    const rt = createRuntime();
    const body = docOf(rt).body;
    const seen: string[] = [];
    const keep = () => seen.push("kept");
    const dropped = () => seen.push("dropped");
    body.addEventListener("select.select", keep);
    body.addEventListener("select.select", keep);
    body.addEventListener("select.select", dropped);
    body.removeEventListener("select.select", dropped);

    rt.createApp({ render: () => null }).mount();
    const root = docOf(rt).body;
    expect(root).not.toBe(body);
    root.dispatchEvent(new CustomEvent("select.select"));
    expect(seen).toEqual(["kept"]);

    const late = () => seen.push("late");
    body.addEventListener("select.select", late);
    root.dispatchEvent(new CustomEvent("select.select"));
    expect(seen).toEqual(["kept", "kept", "late"]);
  });
});
