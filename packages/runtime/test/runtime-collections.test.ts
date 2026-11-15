// DOM collections and the interface objects code tests nodes against (`instanceof NodeList`,
// `instanceof HTMLAudioElement`, `Node.TEXT_NODE`). Each expectation is what Chrome answers, except where a test
// says otherwise.

import { describe, expect, test } from "bun:test";
import { h } from "@vue/runtime-core";
import { createRuntime, type MirrorElement } from "./runtime-harness";

type Fn = (this: unknown, ...args: unknown[]) => unknown;

interface Collection extends Iterable<DomNode> {
  [key: string]: unknown;
  length: number;
  item(index?: number): DomNode | null;
  namedItem(name: string): DomNode | null;
  forEach(callback: (node: DomNode, index: number) => void): void;
  entries(): IterableIterator<[number, DomNode]>;
}
interface DomNode {
  [key: string]: unknown;
  tagName: string;
  nodeName: string;
  nodeType: number;
  localName: string;
  namespaceURI: string;
  parentNode: DomNode | null;
  childNodes: Collection;
  children: Collection;
  touches: Collection;
  targetTouches: Collection;
  changedTouches: Collection;
  addedNodes: Collection;
  removedNodes: Collection;
  setAttribute(name: string, value: string): void;
  getAttribute(name: string): string | null;
  appendChild(node: DomNode): DomNode;
  removeChild(node: DomNode): DomNode;
  querySelectorAll(selector: string): Collection;
  getElementsByTagName(tag: string): Collection;
  getElementsByClassName(names: string): Collection;
  getClientRects(): Collection;
}
interface DomDocument extends DomNode {
  head: DomNode;
  createElement(tag: string): DomNode;
  createElementNS(namespace: string, tag: string): DomNode;
  createComment(data: string): DomNode;
  createAttribute(name: string): DomNode;
  createTreeWalker(root: DomNode, what: number): unknown;
  getElementsByName(name: string): Collection;
}
interface Ctor {
  new (...args: unknown[]): DomNode;
  prototype: { [key: PropertyKey]: unknown; item: Fn };
  name: string;
  [key: string]: unknown;
}
interface Observer {
  observe(target: DomNode, options: unknown): void;
}
interface Realm {
  AnimationEvent: Ctor;
  CharacterData: Ctor;
  CompositionEvent: Ctor;
  DOMRectList: Ctor;
  DOMStringMap: Ctor;
  DOMTokenList: Ctor;
  DocumentFragment: Ctor;
  Element: Ctor;
  HTMLAudioElement: Ctor;
  HTMLCollection: Ctor;
  HTMLElement: Ctor;
  HTMLMediaElement: Ctor;
  HTMLParagraphElement: Ctor;
  HTMLStyleElement: Ctor;
  HTMLVideoElement: Ctor;
  MathMLElement: Ctor;
  MutationRecord: Ctor;
  Node: Ctor;
  NodeList: Ctor;
  PopStateEvent: Ctor;
  SVGElement: Ctor;
  SVGGeometryElement: Ctor;
  SVGGradientElement: Ctor;
  SVGGraphicsElement: Ctor;
  SVGLinearGradientElement: Ctor;
  SVGPathElement: Ctor;
  SVGSVGElement: Ctor;
  SVGTextContentElement: Ctor;
  Text: Ctor;
  ToggleEvent: Ctor;
  Touch: Ctor;
  TouchEvent: Ctor;
  TouchList: Ctor;
  TransitionEvent: Ctor;
  TreeWalker: Ctor;
  MutationObserver: new (callback: (records: DomNode[]) => unknown) => Observer;
  document: DomDocument;
}
const mirror = (node: DomNode) => node as unknown as MirrorElement;
const ctorOf = (g: Realm, name: string): Ctor => {
  const ctor = (g as unknown as Record<string, Ctor | undefined>)[name];
  if (!ctor) throw new Error(`${name} is not a realm global`);
  return ctor;
};

function world() {
  const rt = createRuntime();
  const g = rt.global as unknown as Realm;
  const doc = g.document;
  const vue = (tag: string, parent?: DomNode, attrs: Record<string, string> = {}) => {
    const raw = rt.nodeOps.createElement(tag);
    const el = raw as unknown as DomNode;
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
    if (parent) rt.nodeOps.insert(raw, mirror(parent), null);
    return el;
  };
  const text = (s: string, parent?: DomNode) => {
    const raw = rt.nodeOps.createText(s);
    const t = raw as unknown as DomNode;
    if (parent) rt.nodeOps.insert(raw, mirror(parent), null);
    return t;
  };
  return { rt, g, doc, vue, text };
}

describe("NodeList and HTMLCollection, the interfaces", () => {
  test("exist, cannot be constructed, and carry WebIDL's members", () => {
    const { g } = world();
    for (const name of ["NodeList", "HTMLCollection"]) {
      const ctor = ctorOf(g, name);
      expect(typeof ctor).toBe("function");
      expect(ctor.name).toBe(name);
      expect(() => new ctor()).toThrow();
      expect(ctor.prototype.constructor).toBe(ctor);
      expect(Object.getPrototypeOf(ctor.prototype)).toBe(Object.prototype);
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
    expect(() => list.item()).toThrow();
    expect(list.item as unknown).toBe(g.NodeList.prototype.item);
    expect(p.childNodes).toBe(list);
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
    expect(t.childNodes instanceof g.NodeList).toBe(true);
    expect(t.childNodes.length).toBe(0);
  });

  test("framer-motion's resolveSubjects takes it, and a querySelectorAll answer, as a NodeList", () => {
    const { rt, g, vue } = world();
    const root = vue("div");
    const a = vue("i", root);
    const b = vue("i", root);
    rt.host.queryAnswers.set("i|0", `${a.__skId},${b.__skId}`);
    const NodeList = g.NodeList;
    function resolveSubjects(subject: unknown) {
      if (subject instanceof NodeList) return Array.from(subject as unknown as Collection);
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
    rt.host.queryAnswers.set("i|0", String(a.__skId));
    const list = root.querySelectorAll("i");
    expect(list instanceof g.NodeList).toBe(true);
    expect(Array.isArray(list)).toBe(false);
    expect(Object.getPrototypeOf(list)).toBe(g.NodeList.prototype);
    expect(Object.prototype.toString.call(list)).toBe("[object NodeList]");
    expect(list.length).toBe(1);
    expect(list[0]).toBe(a);
    expect(list.item(0)).toBe(a);
    expect(Object.keys(list)).toEqual(["0"]);
    expect(Object.getOwnPropertyDescriptor(list, 0)).toEqual({
      value: a,
      writable: false,
      enumerable: true,
      configurable: true,
    });
    expect(typeof list.map).toBe("undefined");
    vue("i", root);
    expect(list.length).toBe(1);
    expect(g.document.querySelectorAll("i") instanceof g.NodeList).toBe(true);
  });

  test("MutationRecord's addedNodes and removedNodes are NodeLists too", async () => {
    const { g, vue } = world();
    const root = vue("div");
    const records: DomNode[] = [];
    new g.MutationObserver((r) => records.push(...r)).observe(root, { childList: true });
    const child = vue("span", root);
    await Promise.resolve();
    const record = records[0]!;
    expect(record.addedNodes instanceof g.NodeList).toBe(true);
    expect(record.addedNodes.item(0)).toBe(child);
    expect(record.removedNodes.length).toBe(0);
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
    expect(Object.getOwnPropertyNames(kids)).toEqual(["0", "1", "n", "p"]);
    expect(Object.keys(kids)).toEqual(["0", "1"]);
    vue("i", root, { id: "item" });
    expect(typeof kids.item).toBe("function");
    expect(kids.length).toBe(3);
    root.removeChild(span);
    expect(kids.length).toBe(2);
    expect(kids.n).toBe(undefined);
    expect([...kids]).toEqual([p, kids[1] as DomNode]);
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
      render: () =>
        h("main", [h("input", { name: "q", class: "f" }), h("input", { name: "r" }), h("b", { class: "f" })]),
    }).mount();
    const inputs = doc.getElementsByTagName("input");
    expect(inputs instanceof g.HTMLCollection).toBe(true);
    expect(inputs.length).toBe(2);
    expect(inputs.namedItem("r")?.getAttribute("name")).toBe("r");
    expect(doc.getElementsByTagName("head")[0]).toBe(doc.head);
    expect(doc.getElementsByClassName("f").length).toBe(2);
    const named = doc.getElementsByName("q");
    expect(named instanceof g.NodeList).toBe(true);
    expect(named.length).toBe(1);
    expect(named.item(0)?.tagName).toBe("INPUT");
  });
});

describe("Node", () => {
  test("is an interface of its own, under EventTarget", () => {
    const { g } = world();
    const Node = g.Node;
    expect(typeof Node).toBe("function");
    expect(Node).not.toBe(g.Element);
    expect(() => new Node()).toThrow();
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
    expect(t.nodeType === g.Node.TEXT_NODE).toBe(true);
    expect(t.TEXT_NODE).toBe(3);
    expect(el.ELEMENT_NODE).toBe(1);
  });
});

describe("namespaces: SVG and MathML elements", () => {
  test("Vue's <svg> subtree: lowercase tagName, the SVG namespace, the SVG interfaces, not HTMLElement", () => {
    const { rt, g } = world();
    const found: { svg?: DomNode } = {};
    rt.createApp({
      render: () =>
        h("div", [
          h("svg", { ref: (e: unknown) => (found.svg = e as DomNode) }, [
            h("path", { d: "M0 0" }),
            h("defs", [h("linearGradient", { id: "g" })]),
            h("text", "t"),
          ]),
          h("math", [h("mi", "x")]),
        ]),
    }).mount();
    const svg = found.svg!;
    const [path, defs] = svg.children as unknown as [DomNode, DomNode];
    const grad = defs.children[0] as DomNode;
    const text = svg.children[2];
    expect([svg.tagName, path.tagName, grad.tagName, svg.nodeName]).toEqual(["svg", "path", "linearGradient", "svg"]);
    expect([svg.namespaceURI, svg.localName, grad.localName]).toEqual([
      "http://www.w3.org/2000/svg",
      "svg",
      "linearGradient",
    ]);
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
    const div = svg.parentNode!;
    const math = div.children[1] as DomNode;
    expect(math instanceof g.MathMLElement).toBe(true);
    expect(math instanceof g.HTMLElement).toBe(false);
    expect(math.namespaceURI).toBe("http://www.w3.org/1998/Math/MathML");
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
    expect(() => new g.DOMTokenList()).toThrow();
    const records: DomNode[] = [];
    new g.MutationObserver((r) => records.push(...r)).observe(el, { attributes: true });
    el.setAttribute("x", "1");
    await Promise.resolve();
    expect(records[0] instanceof g.MutationRecord).toBe(true);
    expect(doc.createTreeWalker(el, 1) instanceof g.TreeWalker).toBe(true);
  });
});

describe("touch, as desktop Chrome shows it on a machine without a touch screen", () => {
  test("nothing says touch is there", () => {
    const { rt, g, vue } = world();
    const el = vue("div");
    expect("ontouchstart" in g).toBe(false);
    expect("ontouchstart" in el).toBe(false);
    void rt;
    const navigator: { permissions: { query: () => Promise<unknown> }; maxTouchPoints?: number } = {
      permissions: { query: () => Promise.resolve(Object.assign(new EventTarget(), { state: "granted" })) },
    };
    createRuntime({ navigator });
    expect(navigator.maxTouchPoints).toBe(0);
  });
});

describe("a same-realm reload", () => {
  test("Node, NodeList and the element chain are the new runtime's", () => {
    const first = createRuntime();
    const realm = first.global as unknown as Realm;
    const OldNode = realm.Node;
    const OldNodeList = realm.NodeList;
    (first.global.__SK_UNLOAD__ as () => void)();
    const second = createRuntime({ realm: first.global });
    expect(realm.Node).not.toBe(OldNode);
    expect(realm.NodeList).not.toBe(OldNodeList);
    const t = second.nodeOps.createText("x");
    const el = second.nodeOps.createElement("div") as unknown as DomNode;
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
  });

  test("the AudioManager pattern: instanceof HTMLAudioElement inside a MutationObserver callback", async () => {
    const { g, doc, vue } = world();
    const root = vue("div");
    const found: unknown[] = [];
    new g.MutationObserver((records) => {
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
      expect([tag, doc.createElement(tag) instanceof ctorOf(g, name)]).toEqual([tag, true]);
      expect([name, Object.getPrototypeOf(ctorOf(g, name).prototype) === g.HTMLElement.prototype]).toEqual([
        name,
        true,
      ]);
    }
    expect(doc.createElement("div") instanceof g.HTMLParagraphElement).toBe(false);
    expect(typeof g.HTMLStyleElement).toBe("function");
  });
});
