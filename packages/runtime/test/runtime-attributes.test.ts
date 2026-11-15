// `el.attributes` (a live NamedNodeMap), `Attr` nodes and the attribute methods around them.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

interface Attr {
  name: string;
  value: string;
  localName: string;
  prefix: string | null;
  namespaceURI: string | null;
  ownerElement: unknown;
  nodeType: number;
  nodeName: string;
  nodeValue: string;
  specified: boolean;
}

interface NamedNodeMap extends Iterable<Attr> {
  length: number;
  item(index: number): Attr | null;
  getNamedItem(name: string): Attr | null;
  getNamedItemNS(ns: string | null, local: string): Attr | null;
  setNamedItem(attr: Attr): Attr | null;
  removeNamedItem(name: string): Attr;
  removeNamedItemNS(ns: string | null, local: string): Attr;
  [key: number]: Attr | undefined;
  id: Attr;
  "data-state": Attr;
}

// The attribute surface the runtime installs on its element mirrors (defineDomGet), beyond MirrorElement's own members.
type AttrElement = MirrorElement & {
  attributes: NamedNodeMap;
  getAttributeNode(name: string): Attr | null;
  getAttributeNodeNS(ns: string | null, local: string): Attr | null;
  setAttributeNode(attr: unknown): Attr | null;
  removeAttributeNode(attr: Attr): Attr;
  setAttributeNS(ns: string | null, name: string, value: string): void;
  getAttributeNS(ns: string | null, local: string): string | null;
  hasAttributeNS(ns: string | null, local: string): boolean;
  removeAttributeNS(ns: string | null, local: string): void;
  hasAttributes(): boolean;
};

interface AttrGlobal {
  Attr: new () => Attr;
  NamedNodeMap: new () => NamedNodeMap;
}

function world() {
  const rt = createRuntime();
  const root = rt.nodeOps.createElement("div");
  const el = rt.nodeOps.createElement("button") as AttrElement;
  rt.nodeOps.insert(el, root, null);
  rt.patchProp(el, "id", null, "b1");
  rt.patchProp(el, "data-state", null, "open");
  rt.patchProp(el, "aria-label", null, "Close");
  return {
    rt,
    root,
    el,
    document: rt.global.document as typeof rt.global.document & { createAttribute(name: string): Attr },
    g: rt.global as typeof rt.global & AttrGlobal,
  };
}

describe("el.attributes", () => {
  test("length, item(), [i], [name] and iteration, in the attribute list's order", () => {
    const { el } = world();
    const map = el.attributes;
    expect(map.length).toBe(3);
    expect(map.item(0)!.name).toBe("id");
    expect(map[1]!.name).toBe("data-state");
    expect(map[2]!.value).toBe("Close");
    expect(map[3]).toBe(undefined);
    expect(map.item(3)).toBe(null);
    expect(map.id.value).toBe("b1");
    expect(map["data-state"].value).toBe("open");
    expect([...map].map((a: Attr) => `${a.name}=${a.value}`)).toEqual(["id=b1", "data-state=open", "aria-label=Close"]);
    expect(Array.from(map, (a: Attr) => a.name)).toEqual(["id", "data-state", "aria-label"]);
    expect(Array.prototype.slice.call(map).length).toBe(3);
    expect(Object.keys(map)).toEqual(["0", "1", "2"]);
    expect(0 in map).toBe(true);
    expect(3 in map).toBe(false);
  });

  test("it is live, and the same object on every read", () => {
    const { rt, el } = world();
    const map = el.attributes;
    expect(el.attributes).toBe(map);
    el.setAttribute("title", "x");
    expect(map.length).toBe(4);
    expect(map[3]!.name).toBe("title");
    rt.patchProp(el, "data-state", "open", null);
    expect(map.length).toBe(3);
    expect([...map].map((a: Attr) => a.name)).toEqual(["id", "aria-label", "title"]);
  });

  test("one Attr per attribute: the map, getNamedItem and getAttributeNode agree", () => {
    const { el } = world();
    const a = el.attributes[0]!;
    expect(el.attributes.getNamedItem("id")).toBe(a);
    expect(el.getAttributeNode("id")).toBe(a);
    expect(el.getAttributeNode("missing")).toBe(null);
    expect(el.attributes.getNamedItem("missing")).toBe(null);
  });

  test("an Attr's fields, and its value reads and writes through the element", () => {
    const { el } = world();
    const a = el.getAttributeNode("data-state")!;
    expect(a.name).toBe("data-state");
    expect(a.localName).toBe("data-state");
    expect(a.prefix).toBe(null);
    expect(a.namespaceURI).toBe(null);
    expect(a.ownerElement).toBe(el);
    expect(a.nodeType).toBe(2);
    expect(a.nodeName).toBe("data-state");
    expect(a.specified).toBe(true);
    el.setAttribute("data-state", "closed");
    expect(a.value).toBe("closed");
    a.value = "delayed-open";
    expect(el.getAttribute("data-state")).toBe("delayed-open");
    a.nodeValue = "instant-open";
    expect(el.getAttribute("data-state")).toBe("instant-open");
  });

  test("class and style keep their own rules through an Attr", () => {
    const { el } = world();
    el.setAttribute("class", "a b");
    el.setAttribute("style", "color: red");
    const cls = el.getAttributeNode("class")!;
    const style = el.getAttributeNode("style")!;
    expect(cls.value).toBe("a b");
    expect(style.value).toBe("color: red");
    cls.value = "c";
    expect(el.className).toBe("c");
    expect(el.classList.contains("c")).toBe(true);
    style.value = "color: blue";
    expect(el.style.color).toBe("blue");
  });

  test("removing the attribute detaches its Attr, which keeps the value it had", () => {
    const { el } = world();
    const a = el.getAttributeNode("aria-label")!;
    el.removeAttribute("aria-label");
    expect(a.ownerElement).toBe(null);
    expect(a.value).toBe("Close");
    // A detached Attr's value is its own.
    a.value = "Open";
    expect(el.hasAttribute("aria-label")).toBe(false);
    // Added again, it is a new Attr.
    el.setAttribute("aria-label", "Again");
    expect(el.getAttributeNode("aria-label")).not.toBe(a);
  });

  test("removeNamedItem returns the removed Attr, and throws NotFoundError for a missing one", () => {
    const { el } = world();
    const a = el.attributes.removeNamedItem("id");
    expect(a.name).toBe("id");
    expect(a.value).toBe("b1");
    expect(a.ownerElement).toBe(null);
    expect(el.hasAttribute("id")).toBe(false);
    expect(() => el.attributes.removeNamedItem("id")).toThrow();
    try {
      el.attributes.removeNamedItem("id");
    } catch (e) {
      expect((e as { name: string }).name).toBe("NotFoundError");
    }
  });

  test("setNamedItem / setAttributeNode attach an Attr, replace the old one, and refuse another element's", () => {
    const { rt, el, document, root } = world();
    const fresh = document.createAttribute("title");
    expect(fresh.ownerElement).toBe(null);
    fresh.value = "hello";
    expect(el.setAttributeNode(fresh)).toBe(null);
    expect(el.getAttribute("title")).toBe("hello");
    expect(el.getAttributeNode("title")).toBe(fresh);
    expect(fresh.ownerElement).toBe(el);

    const second = document.createAttribute("title");
    second.value = "bye";
    const old = el.attributes.setNamedItem(second);
    expect(old).toBe(fresh);
    expect(fresh.ownerElement).toBe(null);
    expect(fresh.value).toBe("hello");
    expect(el.getAttribute("title")).toBe("bye");

    const other = rt.nodeOps.createElement("span") as AttrElement;
    rt.nodeOps.insert(other, root, null);
    expect(() => other.setAttributeNode(second)).toThrow();
    expect(() => el.setAttributeNode({ name: "x", value: "y" })).toThrow(TypeError);
    // Setting it on its own element again is a no-op that returns it.
    expect(el.setAttributeNode(second)).toBe(second);
  });

  test("removeAttributeNode removes the element's own Attr and refuses another", () => {
    const { el, document } = world();
    const a = el.getAttributeNode("id")!;
    expect(el.removeAttributeNode(a)).toBe(a);
    expect(el.hasAttribute("id")).toBe(false);
    expect(() => el.removeAttributeNode(a)).toThrow();
    expect(() => el.removeAttributeNode(document.createAttribute("x"))).toThrow();
  });

  test("namespaced attributes: the prefix names the namespace, as Vue's renderer sets them on SVG", () => {
    const { el } = world();
    const XLINK = "http://www.w3.org/1999/xlink";
    el.setAttributeNS(XLINK, "xlink:href", "#icon");
    expect(el.getAttribute("xlink:href")).toBe("#icon");
    expect(el.getAttributeNS(XLINK, "href")).toBe("#icon");
    expect(el.hasAttributeNS(XLINK, "href")).toBe(true);
    expect(el.hasAttributeNS(null, "href")).toBe(false);
    const a = el.attributes.getNamedItemNS(XLINK, "href")!;
    expect(a.prefix).toBe("xlink");
    expect(a.localName).toBe("href");
    expect(a.namespaceURI).toBe(XLINK);
    expect(el.getAttributeNodeNS(XLINK, "href")).toBe(a);
    // Set again under the same namespace and local name: the same attribute.
    el.setAttributeNS(XLINK, "xlink:href", "#other");
    expect(el.getAttribute("xlink:href")).toBe("#other");
    // A null namespace is the plain attribute.
    el.setAttributeNS(null, "fill", "red");
    expect(el.getAttribute("fill")).toBe("red");
    expect(el.getAttributeNS(null, "fill")).toBe("red");
    expect(el.getAttributeNS("", "fill")).toBe("red");
    el.removeAttributeNS(XLINK, "href");
    expect(el.hasAttribute("xlink:href")).toBe(false);
    expect(() => el.attributes.removeNamedItemNS(XLINK, "href")).toThrow();
  });

  test("hasAttributes, getAttributeNames", () => {
    const { rt, el } = world();
    expect(el.hasAttributes()).toBe(true);
    expect(el.getAttributeNames()).toEqual(["id", "data-state", "aria-label"]);
    const bare = rt.nodeOps.createElement("span") as AttrElement;
    expect(bare.hasAttributes()).toBe(false);
    expect(bare.getAttributeNames()).toEqual([]);
    expect(bare.attributes.length).toBe(0);
    expect([...bare.attributes]).toEqual([]);
  });

  test("toggleAttribute: force true keeps an existing value, force false removes, and it answers presence", () => {
    const { el } = world();
    expect(el.toggleAttribute("data-state", true)).toBe(true);
    expect(el.getAttribute("data-state")).toBe("open");
    expect(el.toggleAttribute("data-state")).toBe(false);
    expect(el.hasAttribute("data-state")).toBe(false);
    expect(el.toggleAttribute("data-state", false)).toBe(false);
    expect(el.hasAttribute("data-state")).toBe(false);
    expect(el.toggleAttribute("hidden")).toBe(true);
    expect(el.getAttribute("hidden")).toBe("");
  });

  test("Attr and NamedNodeMap are interfaces, and instanceof answers for the runtime's objects", () => {
    const { el, g, document } = world();
    expect(typeof g.Attr).toBe("function");
    expect(typeof g.NamedNodeMap).toBe("function");
    expect(el.attributes instanceof g.NamedNodeMap).toBe(true);
    expect(el.attributes[0] instanceof g.Attr).toBe(true);
    expect(document.createAttribute("x") instanceof g.Attr).toBe(true);
    expect(() => new g.Attr()).toThrow();
    expect(() => document.createAttribute("1x")).toThrow();
  });
});
