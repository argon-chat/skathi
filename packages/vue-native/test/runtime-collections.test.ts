// DOM collections and the interface objects code tests nodes against
// (research/reports/m26/dom-globals-1.md).
//
// motion's `animate()` resolves its subject with `subject instanceof NodeList`,
// a ReferenceError while the global was missing; the client's AudioManager
// tests `instanceof HTMLAudioElement` inside a MutationObserver callback; emojix
// compares `nodeType` with `Node.TEXT_NODE`, which was undefined because redjs
// aliases `Node` to `Element`. Each expectation below is what Chrome answers
// (Chrome 156, probed for this report), except where a test says otherwise.

import { describe, expect, test } from "bun:test";
import { h } from "@vue/runtime-core";
import { createRuntime } from "./runtime-harness";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

function world() {
  const rt: Any = createRuntime();
  const g: Any = rt.global;
  const doc: Any = g.document;
  const vue = (tag: string, parent?: Any, attrs: Record<string, string> = {}) => {
    const el = rt.nodeOps.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
    if (parent) rt.nodeOps.insert(el, parent, null);
    return el;
  };
  const text = (s: string, parent?: Any) => {
    const t = rt.nodeOps.createText(s);
    if (parent) rt.nodeOps.insert(t, parent, null);
    return t;
  };
  return { rt, g, doc, vue, text };
}

describe("NodeList and HTMLCollection, the interfaces", () => {
  test("exist, cannot be constructed, and carry WebIDL's members", () => {
    const { g } = world();
    for (const name of ["NodeList", "HTMLCollection", "DOMRectList"]) {
      expect(typeof g[name]).toBe("function");
      expect(g[name].name).toBe(name);
      expect(() => new g[name]()).toThrow("Illegal constructor");
      expect(g[name].prototype.constructor).toBe(g[name]);
      expect(Object.getPrototypeOf(g[name].prototype)).toBe(Object.prototype);
    }
    const nl = g.NodeList.prototype;
    expect(Object.keys(nl).sort()).toEqual(["entries", "forEach", "item", "keys", "length", "values"]);
    expect(nl.forEach).toBe(Array.prototype.forEach);
    expect(nl.values).toBe(Array.prototype.values);
    expect(nl[Symbol.iterator]).toBe(Array.prototype.values);
    expect(Object.keys(g.HTMLCollection.prototype).sort()).toEqual(["item", "length", "namedItem"]);
    expect(g.HTMLCollection.prototype[Symbol.iterator]).toBe(Array.prototype.values);
    expect(nl.item.length).toBe(1);
  });
});

describe("childNodes: a live NodeList, the renderer's own list", () => {
  test("instanceof NodeList, item(), the toString tag, and the same object on every read", () => {
    const { g, vue, text } = world();
    const p = vue("p");
    const t = text("a", p);
    const list = p.childNodes;
    expect(list instanceof g.NodeList).toBe(true);
    expect(list instanceof g.HTMLCollection).toBe(false);
    expect(Object.prototype.toString.call(list)).toBe("[object NodeList]");
    expect(list.item(0)).toBe(t);
    expect(list.item(99)).toBe(null);
    expect(list[99]).toBe(undefined);
    expect(() => list.item()).toThrow("1 argument required");
    expect(list.item).toBe(g.NodeList.prototype.item);
    expect(p.childNodes).toBe(list);
    // Live: it IS the list the renderer splices.
    const b = vue("b", p);
    expect(list.length).toBe(2);
    expect([...list]).toEqual([t, b]);
    const seen: unknown[] = [];
    list.forEach((n: unknown, i: number) => seen.push(i, n));
    expect(seen).toEqual([0, t, 1, b]);
    expect([...list.entries()]).toEqual([
      [0, t],
      [1, b],
    ]);
    // A text node has one too, empty.
    expect(t.childNodes instanceof g.NodeList).toBe(true);
    expect(t.childNodes.length).toBe(0);
  });

  test("divergence (docs/RUNTIME.md §5.24): it is an Array underneath", () => {
    const { g, vue } = world();
    const list = vue("div").childNodes;
    expect(Array.isArray(list)).toBe(true);
    expect(Object.getPrototypeOf(list)).toBe(Array.prototype);
    expect(Object.keys(list)).toEqual([]);
    expect([] instanceof g.NodeList).toBe(false);
    expect(() => g.NodeList.prototype.item.call([1], 0)).toThrow("Illegal invocation");
  });

  test("framer-motion's resolveSubjects takes it, and a querySelectorAll answer, as a NodeList", () => {
    const { rt, g, vue } = world();
    const root = vue("div");
    const a = vue("i", root);
    const b = vue("i", root);
    rt.host.queryAnswers.set("i|0", `${a.__vnId},${b.__vnId}`);
    // framer-motion/dist/es/animation/animate/resolve-subjects.mjs, verbatim.
    const NodeList = g.NodeList;
    function resolveSubjects(subject: Any) {
      if (subject instanceof NodeList) return Array.from(subject);
      else if (Array.isArray(subject)) return subject;
      return [subject];
    }
    expect(resolveSubjects(root.querySelectorAll("i"))).toEqual([a, b]);
    expect(resolveSubjects(root.childNodes)).toEqual([a, b]);
  });
});

describe("querySelectorAll: a static NodeList", () => {
  test("a snapshot, with read-only index properties, and no Array underneath", () => {
    const { rt, g, vue } = world();
    const root = vue("div");
    const a = vue("i", root);
    rt.host.queryAnswers.set("i|0", String(a.__vnId));
    const list = root.querySelectorAll("i");
    expect(list instanceof g.NodeList).toBe(true);
    expect(Array.isArray(list)).toBe(false);
    expect(Object.getPrototypeOf(list)).toBe(g.NodeList.prototype);
    expect(Object.prototype.toString.call(list)).toBe("[object NodeList]");
    expect(list.length).toBe(1);
    expect(list[0]).toBe(a);
    expect(list.item(0)).toBe(a);
    expect(Object.keys(list)).toEqual(["0"]);
    expect(Object.getOwnPropertyDescriptor(list, 0)).toEqual({ value: a, writable: false, enumerable: true, configurable: true });
    expect(typeof list.map).toBe("undefined");
    // Static: a later child is not in it.
    vue("i", root);
    expect(list.length).toBe(1);
    expect(g.document.querySelectorAll("i") instanceof g.NodeList).toBe(true);
  });

  test("MutationRecord's addedNodes and removedNodes are NodeLists too", async () => {
    const { g, vue } = world();
    const root = vue("div");
    const records: Any[] = [];
    new g.MutationObserver((r: Any[]) => records.push(...r)).observe(root, { childList: true });
    const child = vue("span", root);
    await Promise.resolve();
    expect(records[0].addedNodes instanceof g.NodeList).toBe(true);
    expect(records[0].addedNodes.item(0)).toBe(child);
    expect(records[0].removedNodes.length).toBe(0);
  });

  test("getClientRects() is a DOMRectList", () => {
    const { g, vue } = world();
    const rects = vue("div").getClientRects();
    expect(rects instanceof g.DOMRectList).toBe(true);
    expect(rects.length).toBe(0);
    expect(rects.item(0)).toBe(null);
  });
});

describe("children and getElementsBy*: live HTMLCollections", () => {
  test("children: elements only, live, the same object, with named properties by id and name", () => {
    const { g, vue, text } = world();
    const root = vue("div");
    text("x", root);
    const span = vue("span", root, { name: "n" });
    const p = vue("p", root, { id: "p" });
    const kids = root.children;
    expect(kids instanceof g.HTMLCollection).toBe(true);
    expect(kids instanceof g.NodeList).toBe(false);
    expect(Object.prototype.toString.call(kids)).toBe("[object HTMLCollection]");
    expect(root.children).toBe(kids);
    expect(kids.length).toBe(2);
    expect([kids[0], kids[1], kids[2]]).toEqual([span, p, undefined]);
    expect(kids.item(1)).toBe(p);
    expect(kids.item(99)).toBe(null);
    expect(kids.namedItem("p")).toBe(p);
    expect(kids.namedItem("n")).toBe(span);
    expect(kids.namedItem("nope")).toBe(null);
    expect(kids.p).toBe(p);
    expect("p" in kids).toBe(true);
    expect("1" in kids).toBe(true);
    expect("2" in kids).toBe(false);
    // Indices then names; the names are not enumerable.
    expect(Object.getOwnPropertyNames(kids)).toEqual(["0", "1", "n", "p"]);
    expect(Object.keys(kids)).toEqual(["0", "1"]);
    // A name the prototype has stays the prototype's.
    vue("i", root, { id: "item" });
    expect(typeof kids.item).toBe("function");
    // Live.
    expect(kids.length).toBe(3);
    root.removeChild(span);
    expect(kids.length).toBe(2);
    expect(kids.n).toBe(undefined);
    expect([...kids]).toEqual([p, kids[1]]);
  });

  test("index properties are read-only; an expando is fine", () => {
    const { vue } = world();
    const root = vue("div");
    const a = vue("a", root);
    const kids = root.children;
    expect(() => {
      "use strict";
      kids[0] = null;
    }).toThrow(TypeError);
    expect(kids[0]).toBe(a);
    expect(Reflect.defineProperty(kids, "0", { value: 1 })).toBe(false);
    expect(Reflect.deleteProperty(kids, "0")).toBe(false);
    kids.mine = 1;
    expect(kids.mine).toBe(1);
    expect(() => Object.preventExtensions(kids)).toThrow(TypeError);
  });

  test("getElementsByTagName and getElementsByClassName follow the subtree, its classes and its ids", () => {
    const { g, vue } = world();
    const root = vue("div");
    const sec = vue("section", root, { class: "a b" });
    const inner = vue("div", sec, { class: "b" });
    const byTag = root.getElementsByTagName("DIV");
    expect(byTag instanceof g.HTMLCollection).toBe(true);
    expect([...byTag]).toEqual([inner]); // descendants only, HTML names case-insensitively
    expect([...root.getElementsByTagName("*")]).toEqual([sec, inner]);
    const byClass = root.getElementsByClassName(" b ");
    expect([...byClass]).toEqual([sec, inner]);
    expect([...root.getElementsByClassName("a b")]).toEqual([sec]);
    expect(root.getElementsByClassName("").length).toBe(0);
    inner.setAttribute("class", "c");
    expect([...byClass]).toEqual([sec]);
    vue("div", root);
    expect(byTag.length).toBe(2);
  });

  test("document.getElementsByTagName / ClassName / Name cover the mounted tree", () => {
    const { rt, g, doc } = world();
    rt.createApp({
      render: () => h("main", [h("input", { name: "q", class: "f" }), h("input", { name: "r" }), h("b", { class: "f" })]),
    }).mount();
    const inputs = doc.getElementsByTagName("input");
    expect(inputs instanceof g.HTMLCollection).toBe(true);
    expect(inputs.length).toBe(2);
    expect(inputs.namedItem("r").getAttribute("name")).toBe("r");
    expect(doc.getElementsByTagName("head")[0]).toBe(doc.head);
    expect(doc.getElementsByClassName("f").length).toBe(2);
    const named = doc.getElementsByName("q");
    expect(named instanceof g.NodeList).toBe(true);
    expect(named.length).toBe(1);
    expect(named[0].tagName).toBe("INPUT");
  });
});

describe("Node", () => {
  test("is an interface of its own, with the constants, under EventTarget", () => {
    const { g } = world();
    const Node = g.Node;
    expect(typeof Node).toBe("function");
    expect(Node).not.toBe(g.Element);
    expect(() => new Node()).toThrow("Illegal constructor");
    expect([Node.ELEMENT_NODE, Node.TEXT_NODE, Node.COMMENT_NODE, Node.DOCUMENT_NODE, Node.DOCUMENT_FRAGMENT_NODE]).toEqual([1, 3, 8, 9, 11]);
    expect(Node.DOCUMENT_POSITION_CONTAINED_BY).toBe(16);
    expect(Node.prototype.TEXT_NODE).toBe(3);
    expect(Object.getOwnPropertyDescriptor(Node, "TEXT_NODE")).toEqual({ value: 3, writable: false, enumerable: true, configurable: false });
    expect(Object.getPrototypeOf(Node.prototype)).toBe(EventTarget.prototype);
    expect(Object.getPrototypeOf(g.Element.prototype)).toBe(Node.prototype);
    expect(Object.getPrototypeOf(g.HTMLElement.prototype)).toBe(g.Element.prototype);
    expect(Object.getPrototypeOf(g.CharacterData.prototype)).toBe(Node.prototype);
  });

  test("elements, text, comments, attributes and the document are Nodes", () => {
    const { g, doc, vue, text } = world();
    const el = vue("div");
    const t = text("hi");
    const c = doc.createComment("note");
    expect(el instanceof g.Node).toBe(true);
    expect(t instanceof g.Node).toBe(true);
    expect(c instanceof g.Node).toBe(true);
    expect(t instanceof g.CharacterData).toBe(true);
    expect(t instanceof g.Text).toBe(true);
    expect(t instanceof g.Element).toBe(false);
    expect(doc.createAttribute("x") instanceof g.Node).toBe(true);
    expect(doc instanceof g.Node).toBe(true);
    expect({} instanceof g.Node).toBe(false);
    // The constants reach every node through its chain (emojix compares with them).
    expect(t.nodeType === g.Node.TEXT_NODE).toBe(true);
    expect(t.TEXT_NODE).toBe(3);
    expect(el.ELEMENT_NODE).toBe(1);
  });
});

describe("namespaces: SVG and MathML elements", () => {
  test("Vue's <svg> subtree: lowercase tagName, the SVG namespace, the SVG interfaces, not HTMLElement", () => {
    const { rt, g } = world();
    let svg: Any;
    rt.createApp({
      render: () =>
        h("div", [
          h("svg", { ref: (e: unknown) => (svg = e) }, [
            h("path", { d: "M0 0" }),
            h("defs", [h("linearGradient", { id: "g" })]),
            h("text", "t"),
          ]),
          h("math", [h("mi", "x")]),
        ]),
    }).mount();
    const [path, defs] = svg.children;
    const grad = defs.children[0];
    const text = svg.children[2];
    expect([svg.tagName, path.tagName, grad.tagName, svg.nodeName]).toEqual(["svg", "path", "linearGradient", "svg"]);
    expect([svg.namespaceURI, svg.localName, grad.localName]).toEqual(["http://www.w3.org/2000/svg", "svg", "linearGradient"]);
    expect(svg instanceof g.SVGElement).toBe(true);
    expect(svg instanceof g.HTMLElement).toBe(false);
    expect(svg instanceof g.Element).toBe(true);
    expect(svg instanceof g.SVGSVGElement).toBe(true);
    expect(svg instanceof g.SVGGraphicsElement).toBe(true);
    expect(path instanceof g.SVGPathElement).toBe(true);
    expect(path instanceof g.SVGGeometryElement).toBe(true);
    expect(path instanceof g.SVGGraphicsElement).toBe(true);
    expect(path instanceof g.SVGSVGElement).toBe(false);
    expect(grad instanceof g.SVGLinearGradientElement).toBe(true);
    expect(grad instanceof g.SVGGradientElement).toBe(true);
    expect(grad instanceof g.SVGGraphicsElement).toBe(false);
    expect(text instanceof g.SVGTextContentElement).toBe(true);
    expect(Object.getPrototypeOf(g.SVGPathElement.prototype)).toBe(g.SVGGeometryElement.prototype);
    const math = svg.parentNode.children[1];
    expect(math instanceof g.MathMLElement).toBe(true);
    expect(math instanceof g.HTMLElement).toBe(false);
    expect(math.namespaceURI).toBe("http://www.w3.org/1998/Math/MathML");
    const div = svg.parentNode;
    expect([div instanceof g.HTMLElement, div instanceof g.SVGElement, div.namespaceURI, div.localName]).toEqual([
      true,
      false,
      "http://www.w3.org/1999/xhtml",
      "div",
    ]);
  });

  test("createElementNS makes the same kind of element", () => {
    const { g, doc } = world();
    const svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
    expect(svg.tagName).toBe("svg");
    expect(svg instanceof g.SVGSVGElement).toBe(true);
    expect(svg instanceof g.HTMLElement).toBe(false);
    const mi = doc.createElementNS("http://www.w3.org/1998/Math/MathML", "mi");
    expect(mi instanceof g.MathMLElement).toBe(true);
    // A fragment is a Node but not an element of either kind.
    const frag = new g.DocumentFragment();
    expect(frag instanceof g.HTMLElement).toBe(false);
    expect(frag instanceof g.Node).toBe(true);
  });
});

describe("interfaces over objects that already existed", () => {
  test("classList, dataset, MutationRecord, TreeWalker and ResizeObserver entries answer instanceof", async () => {
    const { g, doc, vue } = world();
    const el = vue("div");
    expect(el.classList instanceof g.DOMTokenList).toBe(true);
    expect(el.dataset instanceof g.DOMStringMap).toBe(true);
    expect({} instanceof g.DOMTokenList).toBe(false);
    expect(() => new g.DOMTokenList()).toThrow("Illegal constructor");
    const records: Any[] = [];
    new g.MutationObserver((r: Any[]) => records.push(...r)).observe(el, { attributes: true });
    el.setAttribute("x", "1");
    await Promise.resolve();
    expect(records[0] instanceof g.MutationRecord).toBe(true);
    expect(doc.createTreeWalker(el, 1) instanceof g.TreeWalker).toBe(true);
  });

  test("the event interfaces construct, with their members as read-only getters", () => {
    const { g } = world();
    const t = new g.TransitionEvent("transitionend", { propertyName: "opacity", elapsedTime: "0.5", bubbles: true });
    expect(t instanceof g.TransitionEvent).toBe(true);
    expect(t instanceof Event).toBe(true);
    expect([t.type, t.propertyName, t.elapsedTime, t.pseudoElement, t.bubbles]).toEqual(["transitionend", "opacity", 0.5, "", true]);
    expect(Object.keys(t)).not.toContain("propertyName");
    const a = new g.AnimationEvent("animationend", { animationName: "fade" });
    expect([a.animationName, a.elapsedTime]).toEqual(["fade", 0]);
    const c = new g.CompositionEvent("compositionend", { data: "日本" });
    expect(c.data).toBe("日本");
    const p = new g.PopStateEvent("popstate", { state: { page: 2 } });
    expect(p.state).toEqual({ page: 2 });
    expect(new g.ToggleEvent("toggle", { newState: "open" }).newState).toBe("open");
    expect(() => Object.getOwnPropertyDescriptor(g.TransitionEvent.prototype, "propertyName").get.call({})).toThrow("Illegal invocation");
  });
});

describe("a same-realm reload", () => {
  test("Node, NodeList and the element chain are the new runtime's", () => {
    const first: Any = createRuntime();
    const realm = first.global;
    const OldNode = realm.Node;
    const OldNodeList = realm.NodeList;
    (realm.__VN_UNLOAD__ as () => void)();
    const second: Any = createRuntime({ realm });
    expect(realm.Node).not.toBe(OldNode);
    expect(realm.NodeList).not.toBe(OldNodeList);
    const t = second.nodeOps.createText("x");
    const el = second.nodeOps.createElement("div");
    expect(t instanceof realm.Node).toBe(true);
    expect(el.childNodes instanceof realm.NodeList).toBe(true);
    expect(el.children instanceof realm.HTMLCollection).toBe(true);
    expect(Object.getPrototypeOf(realm.Element.prototype)).toBe(realm.Node.prototype);
  });
});

describe("HTML element interfaces, by tag", () => {
  test("<video> and <audio> are HTMLMediaElements of their own kind", () => {
    const { g, doc } = world();
    const video = doc.createElement("video");
    const audio = doc.createElement("audio");
    expect(video instanceof g.HTMLVideoElement).toBe(true);
    expect(video instanceof g.HTMLMediaElement).toBe(true);
    expect(video instanceof g.HTMLAudioElement).toBe(false);
    expect(audio instanceof g.HTMLAudioElement).toBe(true);
    expect(audio instanceof g.HTMLMediaElement).toBe(true);
    expect(audio instanceof g.HTMLElement).toBe(true);
    expect(Object.getPrototypeOf(g.HTMLVideoElement.prototype)).toBe(g.HTMLMediaElement.prototype);
    expect(Object.getPrototypeOf(g.HTMLMediaElement.prototype)).toBe(g.HTMLElement.prototype);
    expect([g.HTMLMediaElement.HAVE_FUTURE_DATA, g.HTMLMediaElement.prototype.HAVE_ENOUGH_DATA, g.HTMLMediaElement.NETWORK_IDLE]).toEqual([3, 4, 1]);
  });

  test("the AudioManager pattern: instanceof HTMLAudioElement inside a MutationObserver callback", async () => {
    const { g, doc, vue } = world();
    const root = vue("div");
    const found: unknown[] = [];
    new g.MutationObserver((records: Any[]) => {
      for (const r of records) for (const n of r.addedNodes) if (n instanceof g.HTMLAudioElement) found.push(n);
    }).observe(root, { childList: true, subtree: true });
    const audio = doc.createElement("audio");
    root.appendChild(doc.createElement("span"));
    root.appendChild(audio);
    await Promise.resolve();
    expect(found).toEqual([audio]);
  });

  test("the rest answer by tag, and sit on HTMLElement.prototype's chain", () => {
    const { g, doc } = world();
    const cases: [string, string][] = [
      ["h2", "HTMLHeadingElement"],
      ["p", "HTMLParagraphElement"],
      ["ul", "HTMLUListElement"],
      ["li", "HTMLLIElement"],
      ["td", "HTMLTableCellElement"],
      ["th", "HTMLTableCellElement"],
      ["tbody", "HTMLTableSectionElement"],
      ["option", "HTMLOptionElement"],
      ["dialog", "HTMLDialogElement"],
      ["template", "HTMLTemplateElement"],
      ["blockquote", "HTMLQuoteElement"],
    ];
    for (const [tag, name] of cases) {
      expect([tag, doc.createElement(tag) instanceof g[name]]).toEqual([tag, true]);
      expect([name, Object.getPrototypeOf(g[name].prototype) === g.HTMLElement.prototype]).toEqual([name, true]);
    }
    expect(doc.createElement("div") instanceof g.HTMLParagraphElement).toBe(false);
    expect(typeof g.HTMLStyleElement).toBe("function");
  });
});
