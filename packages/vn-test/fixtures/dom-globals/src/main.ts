import { createApp, h } from "vue-native";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any
const g = globalThis as Any;

const App = {
  render: () =>
    h("main", { id: "page" }, [
      h("ul", { id: "list" }, [h("li", { id: "a", class: "x" }, "a"), " ", h("li", { name: "b", class: "x" }, "b")]),
      h("svg", { id: "icon", width: 20, height: 20, viewBox: "0 0 20 20" }, [h("path", { d: "M2 2L18 18", stroke: "#888" })]),
      h("input", { id: "q", name: "q" }),
    ]),
};
createApp(App).mount("#app");

const proto = Object.getPrototypeOf;

g.__VN_TEST_DOM__ = {
  probe(): string {
    const list = document.getElementById("list") as Any;
    const icon = document.getElementById("icon") as Any;
    const path = icon.firstChild as Any;
    const text = list.childNodes[1] as Any;
    const kids = list.children;
    const byTag = document.getElementsByTagName("li");
    const before = { kids: kids.length, byTag: byTag.length };
    const extra = document.createElement("li");
    list.appendChild(extra);
    const after = { kids: kids.length, byTag: byTag.length, nodes: list.childNodes.length };
    list.removeChild(extra);
    const qsa = document.querySelectorAll("li");
    const resolveSubjects = (subject: Any) =>
      subject instanceof g.NodeList ? Array.from(subject) : Array.isArray(subject) ? subject : [subject];
    const video = document.createElement("video");
    const audio = document.createElement("audio");
    return JSON.stringify({
      nodeIsOwn: g.Node !== g.Element && g.Node.name === "Node",
      constants: [g.Node.ELEMENT_NODE, g.Node.TEXT_NODE, g.Node.COMMENT_NODE, g.Node.DOCUMENT_FRAGMENT_NODE],
      chain: [
        proto(g.HTMLElement.prototype) === g.Element.prototype,
        proto(g.Element.prototype) === g.Node.prototype,
        proto(g.Node.prototype) === g.EventTarget.prototype,
        proto(g.SVGElement.prototype) === g.Element.prototype,
      ],
      textIsNode: [text instanceof g.Node, text instanceof g.CharacterData, text instanceof g.Text, text.nodeType === g.Node.TEXT_NODE],
      elementIsNode: [list instanceof g.Node, list instanceof g.Element, list instanceof g.HTMLElement, document instanceof g.Node],
      childNodes: [list.childNodes instanceof g.NodeList, Object.prototype.toString.call(list.childNodes), list.childNodes.item(1) === text, list.childNodes === list.childNodes],
      children: [kids instanceof g.HTMLCollection, kids === list.children, kids.namedItem("a") === list.firstChild, kids.b === list.lastChild, Object.keys(kids).join()],
      live: [before, after],
      qsa: [qsa instanceof g.NodeList, Array.isArray(qsa), qsa.length, resolveSubjects(qsa).length, resolveSubjects(list.childNodes).length],
      byClass: [document.getElementsByClassName("x").length, list.getElementsByClassName("x").length],
      byName: [document.getElementsByName("q") instanceof g.NodeList, document.getElementsByName("q").length],
      svg: [icon.tagName, path.tagName, icon.namespaceURI, icon instanceof g.SVGElement, icon instanceof g.HTMLElement, icon instanceof g.SVGSVGElement, path instanceof g.SVGGeometryElement, path instanceof g.SVGGraphicsElement],
      svgPainted: icon.getBoundingClientRect().width,
      svgNS: document.createElementNS("http://www.w3.org/2000/svg", "svg").tagName,
      media: [video instanceof g.HTMLVideoElement, video instanceof g.HTMLMediaElement, audio instanceof g.HTMLAudioElement, audio instanceof g.HTMLVideoElement, proto(g.HTMLVideoElement.prototype) === g.HTMLMediaElement.prototype],
      brands: [list.classList instanceof g.DOMTokenList, list.dataset instanceof g.DOMStringMap, typeof g.MutationRecord, typeof g.TreeWalker, matchMedia("(min-width: 1px)") instanceof g.MediaQueryList],
      events: [new g.TransitionEvent("transitionend", { propertyName: "opacity" }).propertyName, new g.AnimationEvent("animationend") instanceof g.Event],
      touch: [
        "ontouchstart" in window,
        "ontouchstart" in document.documentElement,
        "ontouchstart" in list,
        navigator.maxTouchPoints,
        typeof g.TouchEvent,
        typeof g.Touch,
        typeof g.TouchList,
        new g.TouchEvent("touchstart", { touches: [new g.Touch({ identifier: 1, target: list })] }).touches.length,
      ],
    });
  },
};
