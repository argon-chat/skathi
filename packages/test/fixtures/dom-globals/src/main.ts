import { createApp, h } from "@skathi-native/runtime";

const dom = document;

type Class<Args extends unknown[] = never[], T extends object = object> = {
  new (...args: Args): T;
  readonly prototype: T;
  readonly name: string;
};
interface Globals {
  Node: Class & {
    readonly ELEMENT_NODE: number;
    readonly TEXT_NODE: number;
    readonly COMMENT_NODE: number;
    readonly DOCUMENT_FRAGMENT_NODE: number;
  };
  Element: Class;
  HTMLElement: Class;
  SVGElement: Class;
  SVGSVGElement: Class;
  SVGGeometryElement: Class;
  SVGGraphicsElement: Class;
  CharacterData: Class;
  Text: Class;
  NodeList: Class;
  HTMLCollection: Class;
  HTMLMediaElement: Class;
  HTMLVideoElement: Class;
  HTMLAudioElement: Class;
  DOMTokenList: Class;
  DOMStringMap: Class;
  MediaQueryList: Class;
  MutationRecord: Class;
  TreeWalker: Class;
  EventTarget: Class;
  Event: Class;
  TransitionEvent: Class<[string, { propertyName: string }], { propertyName: string }>;
  AnimationEvent: Class<[string], Event>;
  TouchEvent: Class<[string, { touches: object[] }], { touches: { length: number } }>;
  Touch: Class<[{ identifier: number; target: unknown }]>;
  TouchList: Class;
}
const g = globalThis as unknown as Globals & { __SK_TEST_DOM__: unknown };

const App = {
  render: () =>
    h("main", { id: "page" }, [
      h("ul", { id: "list" }, [h("li", { id: "a", class: "x" }, "a"), " ", h("li", { name: "b", class: "x" }, "b")]),
      h("svg", { id: "icon", width: 20, height: 20, viewBox: "0 0 20 20" }, [
        h("path", { d: "M2 2L18 18", stroke: "#888" }),
      ]),
      h("input", { id: "q", name: "q" }),
    ]),
};
createApp(App).mount("#app");

const proto = Object.getPrototypeOf;

g.__SK_TEST_DOM__ = {
  probe(): string {
    const list = dom.getElementById("list")!;
    const icon = dom.getElementById("icon")!;
    const path = icon.firstChild as HTMLElement;
    const text = list.childNodes[1]!;
    const kids = list.children;
    const byTag = dom.getElementsByTagName("li");
    const before = { kids: kids.length, byTag: byTag.length };
    const extra = dom.createElement("li");
    list.appendChild(extra);
    const after = { kids: kids.length, byTag: byTag.length, nodes: list.childNodes.length };
    list.removeChild(extra);
    const qsa = dom.querySelectorAll("li");
    const resolveSubjects = (subject: unknown): unknown[] =>
      subject instanceof g.NodeList
        ? Array.from(subject as Iterable<unknown>)
        : Array.isArray(subject)
          ? subject
          : [subject];
    const video = dom.createElement("video");
    const audio = dom.createElement("audio");
    return JSON.stringify({
      nodeIsOwn: g.Node !== g.Element && g.Node.name === "Node",
      constants: [g.Node.ELEMENT_NODE, g.Node.TEXT_NODE, g.Node.COMMENT_NODE, g.Node.DOCUMENT_FRAGMENT_NODE],
      chain: [
        proto(g.HTMLElement.prototype) === g.Element.prototype,
        proto(g.Element.prototype) === g.Node.prototype,
        proto(g.Node.prototype) === g.EventTarget.prototype,
        proto(g.SVGElement.prototype) === g.Element.prototype,
      ],
      textIsNode: [
        text instanceof g.Node,
        text instanceof g.CharacterData,
        text instanceof g.Text,
        text.nodeType === g.Node.TEXT_NODE,
      ],
      elementIsNode: [
        list instanceof g.Node,
        list instanceof g.Element,
        list instanceof g.HTMLElement,
        document instanceof g.Node,
      ],
      childNodes: [
        list.childNodes instanceof g.NodeList,
        Object.prototype.toString.call(list.childNodes),
        list.childNodes.item(1) === text,
        list.childNodes === list.childNodes,
      ],
      children: [
        kids instanceof g.HTMLCollection,
        kids === list.children,
        kids.namedItem("a") === list.firstChild,
        (kids as HTMLCollection & Record<string, unknown>).b === list.lastChild,
        Object.keys(kids).join(),
      ],
      live: [before, after],
      qsa: [
        qsa instanceof g.NodeList,
        Array.isArray(qsa),
        qsa.length,
        resolveSubjects(qsa).length,
        resolveSubjects(list.childNodes).length,
      ],
      byClass: [dom.getElementsByClassName("x").length, list.getElementsByClassName("x").length],
      byName: [dom.getElementsByName("q") instanceof g.NodeList, dom.getElementsByName("q").length],
      svg: [
        icon.tagName,
        path.tagName,
        icon.namespaceURI,
        icon instanceof g.SVGElement,
        icon instanceof g.HTMLElement,
        icon instanceof g.SVGSVGElement,
        path instanceof g.SVGGeometryElement,
        path instanceof g.SVGGraphicsElement,
      ],
      svgPainted: icon.getBoundingClientRect().width,
      svgNS: dom.createElementNS("http://www.w3.org/2000/svg", "svg").tagName,
      media: [
        video instanceof g.HTMLVideoElement,
        video instanceof g.HTMLMediaElement,
        audio instanceof g.HTMLAudioElement,
        audio instanceof g.HTMLVideoElement,
        proto(g.HTMLVideoElement.prototype) === g.HTMLMediaElement.prototype,
      ],
      brands: [
        list.classList instanceof g.DOMTokenList,
        list.dataset instanceof g.DOMStringMap,
        typeof g.MutationRecord,
        typeof g.TreeWalker,
        matchMedia("(min-width: 1px)") instanceof g.MediaQueryList,
      ],
      events: [
        new g.TransitionEvent("transitionend", { propertyName: "opacity" }).propertyName,
        new g.AnimationEvent("animationend") instanceof g.Event,
      ],
      touch: [
        "ontouchstart" in window,
        "ontouchstart" in dom.documentElement,
        "ontouchstart" in list,
        Number.isInteger(navigator.maxTouchPoints) && navigator.maxTouchPoints >= 0,
        typeof g.TouchEvent,
        typeof g.Touch,
        typeof g.TouchList,
        new g.TouchEvent("touchstart", { touches: [new g.Touch({ identifier: 1, target: list })] }).touches.length,
      ],
    });
  },
};
