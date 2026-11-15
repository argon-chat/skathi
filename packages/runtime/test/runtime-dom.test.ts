// The DOM surface on mirror nodes and the globals a reused browser component reaches for before it can render:
// every answer is either truthful (mirror tree, the host's matcher and layout bounds) or degraded with an advisory.

import { describe, expect, test } from "bun:test";
import { h } from "@vue/runtime-core";
import {
  createRuntime,
  type MirrorElement,
  type MirrorNode,
  type RealmObserver,
  type RealmObserverEntry,
} from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
type Constructor = new (...args: never[]) => object;

interface ResizeEntry {
  target: unknown;
  contentRect: unknown;
  borderBoxSize: unknown[];
  contentBoxSize: unknown[];
  devicePixelContentBoxSize: unknown[];
}
interface ErrorEventLike {
  type: string;
  message: string;
  cancelable: boolean;
  error: unknown;
}
interface ComputedStyleLike {
  getPropertyValue(name: string): string;
  [key: string]: unknown;
}

const sizeOf = (entry: RealmObserverEntry | undefined) => {
  const rect = entry?.contentRect as { width: number; height: number } | undefined;
  if (!rect) throw new Error("the observer reported no contentRect");
  return rect;
};
const computedStyleOf = (rt: Rt, el: MirrorElement) =>
  (rt.global as unknown as { getComputedStyle(el: MirrorElement): ComputedStyleLike }).getComputedStyle(el);
const nodeFilter = (rt: Rt) => rt.global.NodeFilter as { SHOW_ELEMENT: number; FILTER_ACCEPT: number };
const skipFlag = (node: MirrorNode) => (node as unknown as { __v_skip?: unknown }).__v_skip;

// The reflected IDL properties the runtime defines on its element mirrors (REFLECTED), beyond MirrorElement's members.
type ReflectedElement = MirrorElement & {
  disabled: boolean;
  readOnly: boolean;
  checked: boolean;
  hidden: boolean | string;
  placeholder: string;
  type: string;
  title: string;
  multiple: boolean;
  dir: string;
  tabIndex: number;
  maxLength: number;
  required: boolean | number;
  open: boolean;
};

function tree(rt: Rt) {
  const root = rt.nodeOps.createElement("root");
  const section = rt.nodeOps.createElement("section");
  const button = rt.nodeOps.createElement("button");
  const span = rt.nodeOps.createElement("span");
  const text = rt.nodeOps.createText("hi");
  rt.nodeOps.insert(section, root, null);
  rt.nodeOps.insert(button, section, null);
  rt.nodeOps.insert(span, section, null);
  rt.nodeOps.insert(text, button, null);
  return { root, section, button, span, text };
}

describe("tree queries answered from the mirror", () => {
  test("contains / isConnected / siblings / textContent are exact", () => {
    const rt = createRuntime();
    const { root, section, button, span, text } = tree(rt);
    expect(root.contains(text)).toBe(true);
    expect(button.contains(span)).toBe(false);
    expect(button.contains(button)).toBe(true);

    expect(section.firstElementChild).toBe(button);
    expect(section.lastElementChild).toBe(span);
    expect(section.childElementCount).toBe(2);
    expect(button.nextElementSibling).toBe(span);
    expect(span.previousElementSibling).toBe(button);
    expect(span.nextElementSibling).toBe(null);

    expect(root.textContent).toBe("hi");
    expect(button.nodeType).toBe(1);
    expect(text.nodeType).toBe(3);
    expect(button.nodeName).toBe("BUTTON");
  });

  test("`children` is element-only, `childNodes` is everything", () => {
    // The DOM's own split, and it is load-bearing: `aria-hidden` walks
    // `parent.children` and calls `getAttribute` on each member, so a text
    // mirror in that list is one logged failure per node.
    const rt = createRuntime();
    const { section, button, span, text } = tree(rt);
    expect([...section.children]).toEqual([button, span]);
    expect([...button.childNodes]).toEqual([text]);
    expect([...button.children]).toEqual([]);
  });

  test("parentElement is the parent element, and null at the root and when detached", () => {
    const rt = createRuntime();
    const { root, section, button, span, text } = tree(rt);
    expect(button.parentElement).toBe(section);
    expect(section.parentElement).toBe(root);
    expect(text.parentElement).toBe(button);
    expect(root.parentElement).toBe(null);
    rt.nodeOps.remove(span);
    expect(span.parentElement).toBe(null);
    expect(span.parentNode).toBe(null);
    expect(button.parentNode).toBe(button.parentElement);
  });

  test("parentElement is not a writable own field", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    expect(Object.prototype.hasOwnProperty.call(button, "parentElement")).toBe(false);
    expect(() => {
      "use strict";
      (button as { parentElement: unknown }).parentElement = null;
    }).toThrow();
  });

  test("the Node walk: firstChild / lastChild / nextSibling / previousSibling / hasChildNodes", () => {
    const rt = createRuntime();
    const { section, button, span, text } = tree(rt);
    const comment = rt.nodeOps.createComment("");
    rt.nodeOps.insert(comment, section, span);
    expect(section.firstChild).toBe(button);
    expect(section.lastChild).toBe(span);
    expect(button.nextSibling).toBe(comment);
    expect(comment.nextSibling).toBe(span);
    expect(span.previousSibling).toBe(comment);
    expect(button.previousSibling).toBe(null);
    expect(span.nextSibling).toBe(null);
    expect(text.firstChild).toBe(null);
    expect(button.hasChildNodes()).toBe(true);
    expect(span.hasChildNodes()).toBe(false);
    const walked = [];
    for (let n = section.firstChild; n; n = n.nextSibling) walked.push(n);
    expect(walked).toEqual([button, comment, span]);
  });

  test("a text or comment node has element siblings (NonDocumentTypeChildNode)", () => {
    const rt = createRuntime();
    const { section, button, span } = tree(rt);
    const comment = rt.nodeOps.createComment("");
    rt.nodeOps.insert(comment, section, span);
    expect(comment.nextElementSibling).toBe(span);
    expect(comment.previousElementSibling).toBe(button);
    expect(button.nextElementSibling).toBe(span);
  });

  test("children has item(), and getRootNode is the document once connected", () => {
    const rt = createRuntime();
    const { root, section, button, span } = tree(rt);
    expect(section.children.item(1)).toBe(span);
    expect(section.children.item(5)).toBe(null);
    expect(section.children.length).toBe(2);
    expect(button.getRootNode()).toBe(root);
    rt.nodeOps.remove(span);
    expect(span.getRootNode()).toBe(span);
  });

  test("a node under the mounted root is connected, its root node is the document", () => {
    const rt = createRuntime();
    rt.createApp({ render: () => null }).mount();
    const body = rt.global.document.body;
    const node = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(node, body, null);
    expect(node.isConnected).toBe(true);
    expect(node.parentElement).toBe(body);
    expect(node.getRootNode()).toBe(rt.global.document);
    expect(body.contains(node)).toBe(true);
  });

  test("setElementText is readable back through textContent", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.nodeOps.setElementText(button, "Save");
    expect(button.textContent).toBe("Save");
    expect(button.childNodes.length).toBe(0);
  });
});

describe("selector matching goes to the host's real matcher", () => {
  test("matches / closest / querySelector(All) call __sk_query with the right mode", () => {
    const rt = createRuntime();
    const { root, section, button } = tree(rt);
    rt.host.queryAnswers.set(".x|1", String(button.__skId));
    rt.host.queryAnswers.set(".x|2", String(section.__skId));
    rt.host.queryAnswers.set(".x|0", `${section.__skId},${button.__skId}`);

    expect(button.matches(".x")).toBe(true);
    expect(button.closest(".x")).toBe(section);
    expect(root.querySelector(".x")).toBe(section);
    expect([...root.querySelectorAll(".x")]).toEqual([section, button]);

    expect(rt.host.queryCalls.map((c) => c.mode)).toEqual([1, 2, 0, 0]);
    expect(rt.host.queryCalls[0]?.root).toBe(button.__skId);
    expect(rt.advisories()).toEqual([]);
  });

  test("Document queries include the document element while Element queries keep excluding self", () => {
    const rt = createRuntime();
    rt.createApp({ render: () => null }).mount();
    const root = rt.global.document.documentElement;
    rt.host.queryAnswers.set("[data-wrapper]|3", String(root.__skId));
    expect(rt.global.document.querySelector("[data-wrapper]")).toBe(root);
    expect([...rt.global.document.querySelectorAll("[data-wrapper]")]).toEqual([root]);
    expect(root.querySelector("[data-wrapper]")).toBeNull();
    expect(rt.host.queryCalls.slice(-3).map((call) => call.mode)).toEqual([3, 3, 0]);
  });

  test("a selector the engine cannot run explains itself instead of saying `no match`", () => {
    const rt = createRuntime();
    const { root } = tree(rt);
    rt.host.queryAnswers.set(":has(.x)", "!`:has()` (the matcher has no implementation)");
    expect(root.querySelectorAll(":has(.x)").length).toBe(0);
    expect(rt.advisories().length).toBe(1);
  });

  test("`:scope` is anchored at the element the call was made on", () => {
    const rt = createRuntime();
    const { root, section, button } = tree(rt);
    rt.host.queryAnswers.set(":scope > button|0", String(button.__skId));
    expect(section.querySelector(":scope > button")).toBe(button);
    expect(rt.host.queryCalls.at(-1)).toMatchObject({
      selector: ":scope > button",
      root: section.__skId,
      mode: 0,
    });

    root.querySelectorAll(":scope > button");
    expect(rt.host.queryCalls.at(-1)?.root).toBe(root.__skId);

    expect(rt.advisories()).toEqual([]);
  });

  test("an unmatched id in the answer is skipped rather than yielding undefined", () => {
    const rt = createRuntime();
    const { root } = tree(rt);
    rt.host.queryAnswers.set(".gone", "99999");
    expect(root.querySelectorAll(".gone").length).toBe(0);
  });
});

describe("layout comes from the host's painted bounds", () => {
  test("getBoundingClientRect reports the real rectangle", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.__skId, "10,20,100,40");
    const r = button.getBoundingClientRect();
    expect(r).toMatchObject({ x: 10, y: 20, width: 100, height: 40, left: 10, top: 20, right: 110, bottom: 60 });
    expect(button.offsetWidth).toBe(100);
    expect(button.clientHeight).toBe(40);
    expect(button.getClientRects().length).toBe(1);
  });

  // A popper laid out at (0, 0) and moved by `transform: translate(300px, 200px)`:
  // the host appends the box it was DRAWN at. The rect methods report that; the
  // size and offset accessors keep the layout box, as a browser's do.
  test("getBoundingClientRect reports the transformed box, offsetWidth the layout one", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.__skId, "0,0,100,40,290,195,120,50");
    expect(button.getBoundingClientRect()).toMatchObject({
      x: 290,
      y: 195,
      width: 120,
      height: 50,
      right: 410,
      bottom: 245,
    });
    expect(button.getClientRects()[0]).toMatchObject({ left: 290, top: 195, width: 120 });
    expect(button.offsetWidth).toBe(100);
    expect(button.clientHeight).toBe(40);
    expect(button.offsetLeft).toBe(0);
  });

  test("a node that was never painted reads all zeros, exactly like a detached one in a browser", () => {
    const rt = createRuntime();
    const { span } = tree(rt);
    const r = span.getBoundingClientRect();
    expect(r).toMatchObject({ x: 0, y: 0, width: 0, height: 0 });
    expect(span.getClientRects().length).toBe(0);
    expect(rt.advisories()).toEqual([]);
  });

  // The host answers "stale" when a node has no rectangle yet and layout could not be forced (a gpui input
  // listener is on the stack): zero plus one advisory per call site and accessor.
  test("a read the host could not lay out for reads zero until it can", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.__skId, "stale");
    expect(button.offsetWidth).toBe(0);
    expect(button.getBoundingClientRect()).toMatchObject({ x: 0, y: 0, width: 0, height: 0 });
    expect(button.getClientRects().length).toBe(0);
    rt.host.boundsById.set(button.__skId, "0,0,120,30");
    expect(button.offsetWidth).toBe(120);
  });

  test("offsetTop/offsetLeft are window-relative", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.__skId, "10,20,100,40");
    expect(button.offsetTop).toBe(20);
  });
});

describe("programmatic scrolling drives the host's ScrollHandle", () => {
  function scroller(
    rt: Rt,
    node: MirrorElement,
    extra: Partial<NonNullable<ReturnType<Rt["host"]["scrollById"]["get"]>>> = {},
  ) {
    rt.host.scrollById.set(node.__skId, {
      left: 0,
      top: 0,
      maxLeft: 0,
      maxTop: 800,
      width: 300,
      height: 200,
      ...extra,
    });
  }

  test("a write reaches the host and the getter sees it on the next line", () => {
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);

    expect(section.scrollTop).toBe(0);
    section.scrollTop = 250;

    expect(rt.host.scrollCalls.some((c) => c.op === 1 && c.y === 250)).toBe(true);
    expect(section.scrollTop).toBe(250);
    expect(section.scrollLeft).toBe(0);
  });

  test("a write past the end lands at the end, like a browser's clamp", () => {
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    section.scrollTop = 1e9;
    expect(section.scrollTop).toBe(800);
    section.scrollTop = -50;
    expect(section.scrollTop).toBe(0);
  });

  test("a non-finite assignment is ignored rather than thrown", () => {
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    section.scrollTop = 120;
    section.scrollTop = NaN;
    section.scrollTop = "nope" as unknown as number;
    expect(section.scrollTop).toBe(120);
  });

  test("scrollTo takes both spellings and scrollBy is relative", () => {
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section, { maxLeft: 400 });

    section.scrollTo({ top: 100, left: 40 });
    expect([section.scrollLeft, section.scrollTop]).toEqual([40, 100]);

    section.scrollTo(10, 20);
    expect([section.scrollLeft, section.scrollTop]).toEqual([10, 20]);

    section.scrollBy({ top: 30 });
    expect([section.scrollLeft, section.scrollTop]).toEqual([10, 50]);

    section.scrollTo({ top: 5 });
    expect([section.scrollLeft, section.scrollTop]).toEqual([10, 5]);
  });

  test("scrollWidth/scrollHeight report the CONTENT box, so scroll-to-bottom works", () => {
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    rt.host.boundsById.set(section.__skId, "0,0,300,200");

    expect(section.clientHeight).toBe(200);
    expect(section.scrollHeight).toBe(1000);

    section.scrollTop = section.scrollHeight;
    expect(section.scrollTop).toBe(800); // the bottom, clamped
  });

  test("an element that does not scroll reports zeros and swallows a write", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.__skId, "0,0,100,40");
    expect(button.scrollTop).toBe(0);
    expect(button.scrollLeft).toBe(0);
    expect(button.scrollWidth).toBe(100);
    button.scrollTop = 500;
    expect(button.scrollTop).toBe(0);
    expect(rt.advisories()).toEqual([]);
  });

  test("scrollIntoView sends the alignment a browser would", () => {
    const rt = createRuntime();
    const { button } = tree(rt);

    button.scrollIntoView();
    expect(rt.host.intoViewCalls.at(-1)).toEqual({ id: button.__skId, block: 0, inline: 3 });

    button.scrollIntoView(false);
    expect(rt.host.intoViewCalls.at(-1)).toMatchObject({ block: 2, inline: 3 });

    button.scrollIntoView({ block: "center", inline: "end" });
    expect(rt.host.intoViewCalls.at(-1)).toMatchObject({ block: 1, inline: 2 });

    button.scrollIntoView({ block: "sideways" });
    expect(rt.host.intoViewCalls.at(-1)).toMatchObject({ block: 0, inline: 3 });
    expect(rt.advisories()).toEqual([]);
  });

  test('behavior: "smooth" is performed instantly', () => {
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    section.scrollTo({ top: 300, behavior: "smooth" });
    expect(section.scrollTop).toBe(300);
  });

  test("a scroll EVENT no longer writes the position back onto the node", () => {
    // The accessor reads the host, so a write-back would push the event's position into the box the event came from.
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    section.scrollTop = 120;

    const seen: number[] = [];
    section.addEventListener("scroll", (e) => seen.push((e.target as MirrorElement).scrollTop));
    rt.host.dispatcher(section.__skId, rt.EV_CODES.scroll, 0, 120, 0, 0, null);

    expect(seen).toEqual([120]);
    expect(rt.host.scrollCalls.filter((c) => c.op !== 0).length).toBe(1);
  });
});

describe("the attribute channel", () => {
  test("what patchProp writes is what getAttribute/dataset read back", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "data-state", null, "open");
    rt.patchProp(button, "aria-expanded", null, true);
    expect(button.getAttribute("data-state")).toBe("open");
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(button.hasAttribute("data-state")).toBe(true);
    expect(button.getAttribute("nope")).toBe(null);
    expect(button.dataset.state).toBe("open");
    expect(button.getAttributeNames()).toEqual(["data-state", "aria-expanded"]);

    rt.patchProp(button, "data-state", "open", null);
    expect(button.getAttribute("data-state")).toBe(null);
    expect(button.hasAttribute("data-state")).toBe(false);
  });

  test("setAttribute is the same write patchProp makes, so it reaches the host", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    button.setAttribute("data-side", "top");
    expect(button.getAttribute("data-side")).toBe("top");
    expect(rt.host.props.at(-1)).toEqual({ id: button.__skId, key: "data-side", value: "top" });
    button.removeAttribute("data-side");
    expect(rt.host.props.at(-1)).toEqual({ id: button.__skId, key: "data-side", value: undefined });
  });

  test("setAttribute('style', …) goes through the style body, not the :style object path", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    button.setAttribute("style", "color: red; --x: 3");
    expect(rt.host.styles).toEqual([
      { id: button.__skId, key: "color", value: "red" },
      { id: button.__skId, key: "--x", value: "3" },
    ]);
  });
});

describe("props that are DOM properties, as runtime-dom sets them", () => {
  const hostProp = (rt: Rt, el: MirrorElement, key: string) =>
    rt.host.props.filter((p) => p.id === el.__skId && p.key === key).at(-1)?.value;
  const make = (rt: Rt, tag: string) => rt.nodeOps.createElement(tag) as ReflectedElement;

  test("a boolean property is present as '' or absent, never the string 'false'", () => {
    const rt = createRuntime();
    const panel = make(rt, "div");
    rt.patchProp(panel, "hidden", null, false);
    expect(panel.hasAttribute("hidden")).toBe(false);
    expect(hostProp(rt, panel, "hidden")).toBeUndefined();
    rt.patchProp(panel, "hidden", false, true);
    expect(panel.getAttribute("hidden")).toBe("");
    expect(hostProp(rt, panel, "hidden")).toBe("");
    rt.patchProp(panel, "hidden", true, "");
    expect(panel.getAttribute("hidden")).toBe("");
    rt.patchProp(panel, "hidden", "", null);
    expect(panel.hasAttribute("hidden")).toBe(false);

    const button = make(rt, "button");
    rt.patchProp(button, "disabled", null, false);
    expect(button.hasAttribute("disabled")).toBe(false);
    rt.patchProp(button, "disabled", false, "disabled");
    expect(button.getAttribute("disabled")).toBe("");
    expect(button.disabled).toBe(true);
  });

  test("where the element has no such property, the value is the attribute, verbatim", () => {
    const rt = createRuntime();
    const div = make(rt, "div");
    rt.patchProp(div, "disabled", null, false);
    expect(div.getAttribute("disabled")).toBe("false");
    rt.patchProp(div, "aria-hidden", null, false);
    expect(div.getAttribute("aria-hidden")).toBe("false");
    rt.patchProp(div, "data-open", null, false);
    expect(div.getAttribute("data-open")).toBe("false");
    const g = make(rt, "g");
    rt.patchProp(g, "hidden", null, false, "svg");
    expect(g.getAttribute("hidden")).toBe("false");
  });

  test("attribute-only booleans, camelCase properties and integers name the right attribute", () => {
    const rt = createRuntime();
    const input = make(rt, "input");
    rt.patchProp(input, "readonly", null, false); // isSpecialBooleanAttr
    expect(input.hasAttribute("readonly")).toBe(false);
    rt.patchProp(input, "readOnly", null, true);
    expect(input.getAttribute("readonly")).toBe("");
    expect(input.readOnly).toBe(true);
    rt.patchProp(input, "tabIndex", null, "2");
    expect(input.getAttribute("tabindex")).toBe("2");
    rt.patchProp(input, "maxLength", null, 8.7);
    expect(input.getAttribute("maxlength")).toBe("8");
    rt.patchProp(input, "tabIndex", "2", null);
    expect(input.hasAttribute("tabindex")).toBe(false);
    const label = make(rt, "label");
    rt.patchProp(label, "htmlFor", null, "email");
    expect(label.getAttribute("for")).toBe("email");
  });

  test(":checked sets the property and the boolean attribute; setAttribute keeps its text", () => {
    const rt = createRuntime();
    const box = make(rt, "input");
    rt.patchProp(box, "type", null, "checkbox");
    rt.patchProp(box, "checked", null, false);
    expect(box.checked).toBe(false);
    expect(box.hasAttribute("checked")).toBe(false);
    rt.patchProp(box, "checked", false, true);
    expect(box.checked).toBe(true);
    expect(box.getAttribute("checked")).toBe("");
    rt.patchProp(box, "true-value", null, "yes");
    expect(box._trueValue).toBe("yes");

    const div = make(rt, "div");
    div.setAttribute("hidden", "false");
    expect(div.getAttribute("hidden")).toBe("false");
    expect(hostProp(rt, div, "hidden")).toBe("false");
    expect(div.hidden).toBe(true);
    div.removeAttribute("hidden");
    expect(div.hidden).toBe(false);
  });

  test("v-text's textContent is the property", () => {
    const rt = createRuntime();
    const p = make(rt, "p");
    rt.patchProp(p, "textContent", null, "hello");
    expect(p.textContent).toBe("hello");
    expect(p.hasAttribute("textContent")).toBe(false);
  });

  test("a component's :hidden=false leaves no attribute (reka TabsContent)", () => {
    const rt = createRuntime();
    rt.createApp({ render: () => h("section", { id: "panel", hidden: false, "data-state": "active" }) }).mount();
    const panel = [...rt.nodesById.values()].find((n) => n.tag === "section") as MirrorElement;
    expect(panel.hasAttribute("hidden")).toBe(false);
    expect(panel.getAttribute("data-state")).toBe("active");
    expect(hostProp(rt, panel, "hidden")).toBeUndefined();
  });
});

describe("reflected IDL attributes", () => {
  const make = (rt: Rt, tag: string) => rt.nodeOps.createElement(tag) as ReflectedElement;

  test("input.placeholder, input.type, el.title and el.hidden read and write their attributes", () => {
    const rt = createRuntime();
    const input = make(rt, "input");
    expect(input.placeholder).toBe("");
    expect(input.type).toBe("text");
    rt.patchProp(input, "placeholder", null, "Email");
    rt.patchProp(input, "type", null, "EMAIL");
    expect(input.placeholder).toBe("Email");
    expect(input.type).toBe("email");
    input.type = "bogus";
    expect(input.getAttribute("type")).toBe("bogus");
    expect(input.type).toBe("text");
    input.placeholder = "Name";
    expect(input.getAttribute("placeholder")).toBe("Name");

    const div = make(rt, "div");
    expect(div.title).toBe("");
    div.title = "Tip";
    expect(div.getAttribute("title")).toBe("Tip");
    expect(div.hidden).toBe(false);
    div.hidden = true;
    expect(div.getAttribute("hidden")).toBe("");
    div.hidden = "until-found";
    expect(div.hidden).toBe("until-found");
    div.hidden = "";
    expect(div.hasAttribute("hidden")).toBe(false);
    expect(div.placeholder).toBeUndefined();
    div.placeholder = "x";
    expect(div.placeholder).toBe("x");
    expect(div.hasAttribute("placeholder")).toBe(false);
  });

  test("button.type, select.type and dir are enumerated", () => {
    const rt = createRuntime();
    const button = make(rt, "button");
    expect(button.type).toBe("submit");
    button.setAttribute("type", "Reset");
    expect(button.type).toBe("reset");
    const select = make(rt, "select");
    expect(select.type).toBe("select-one");
    select.multiple = true;
    expect(select.type).toBe("select-multiple");
    const div = make(rt, "div");
    expect(div.dir).toBe("");
    div.setAttribute("dir", "RTL");
    expect(div.dir).toBe("rtl");
    div.dir = "sideways";
    expect(div.dir).toBe("");
  });

  test("tabIndex: 0 on what is focusable by default, -1 elsewhere, the parsed attribute when set", () => {
    const rt = createRuntime();
    expect(make(rt, "div").tabIndex).toBe(-1);
    expect(make(rt, "button").tabIndex).toBe(0);
    expect(make(rt, "a").tabIndex).toBe(0);
    expect(make(rt, "input").tabIndex).toBe(0);
    const details = make(rt, "details");
    const summary = make(rt, "summary");
    rt.nodeOps.insert(summary, details, null);
    expect(summary.tabIndex).toBe(0);
    const div = make(rt, "div");
    div.setAttribute("tabindex", " 3");
    expect(div.tabIndex).toBe(3);
    div.setAttribute("tabindex", "x");
    expect(div.tabIndex).toBe(-1);
    div.tabIndex = 0;
    expect(div.getAttribute("tabindex")).toBe("0");
    const input = make(rt, "input");
    expect(input.maxLength).toBe(-1);
    expect(() => (input.maxLength = -2)).toThrow();
    input.maxLength = 4;
    expect(input.getAttribute("maxlength")).toBe("4");
  });

  test("disabled, required, open and readOnly are booleans over their attributes", () => {
    const rt = createRuntime();
    const field = make(rt, "textarea");
    field.disabled = true;
    field.required = 1;
    field.readOnly = true;
    expect(field.getAttributeNames().sort()).toEqual(["disabled", "readonly", "required"]);
    field.disabled = false;
    expect(field.disabled).toBe(false);
    expect(field.hasAttribute("disabled")).toBe(false);
    const details = make(rt, "details");
    details.setAttribute("open", "");
    expect(details.open).toBe(true);
    expect(make(rt, "div").disabled).toBeUndefined();
  });
});

describe("inline style writes through — custom properties included", () => {
  test("setProperty reaches the host under the CSS spelling", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    button.style.setProperty("--reka-popper-available-width", "320px");
    button.style.zIndex = "50";
    expect(rt.host.styles).toEqual([
      { id: button.__skId, key: "--reka-popper-available-width", value: "320px" },
      { id: button.__skId, key: "z-index", value: "50" },
    ]);
    expect(button.style.getPropertyValue("--reka-popper-available-width")).toBe("320px");
    expect(button.style.zIndex).toBe("50");
    expect(rt.advisories()).toEqual([]);
  });
});

describe("classList and the transition-class merge", () => {
  test("classList edits rewrite the whole class attribute", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "class", null, "btn primary");
    button.classList.add("open");
    expect(rt.host.classes.at(-1)?.value).toBe("btn primary open");
    expect(button.classList.contains("primary")).toBe(true);
    button.classList.remove("primary");
    expect(rt.host.classes.at(-1)?.value).toBe("btn open");
    expect(button.classList.toggle("open")).toBe(false);
    expect(rt.host.classes.at(-1)?.value).toBe("btn");
  });

  test("a write that leaves the class as it was does not reach the host", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "class", null, "btn primary");
    const writes = rt.host.classes.length;
    button.classList.add("btn");
    button.classList.remove("open");
    button.className = "btn primary";
    expect(rt.host.classes.length).toBe(writes);
    button.className = "btn";
    expect(rt.host.classes.length).toBe(writes + 1);
  });

  test("a re-render keeps the transition classes a <Transition> layered on", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "class", null, "card");
    button._vtc = new Set(["v-enter-active"]);
    rt.writeClass(button);
    expect(rt.host.classes.at(-1)?.value).toBe("card v-enter-active");
    rt.patchProp(button, "class", "card", "card open");
    expect(rt.host.classes.at(-1)?.value).toBe("card open v-enter-active");
  });
});

describe("className, classList and the class attribute are one thing", () => {
  test("className and setAttribute write through; getAttribute and className read the attribute", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    expect(button.getAttribute("class")).toBe(null);
    expect(button.className).toBe("");
    button.className = "row group/msg";
    expect(rt.host.classes.at(-1)).toEqual({ id: button.__skId, value: "row group/msg" });
    expect(button.getAttribute("class")).toBe("row group/msg");
    expect(button.hasAttribute("class")).toBe(true);
    expect(button.classList.contains("group/msg")).toBe(true);
    button.setAttribute("class", "a b");
    expect(button.className).toBe("a b");
    expect(rt.host.classes.at(-1)?.value).toBe("a b");
    button.removeAttribute("class");
    expect(button.getAttribute("class")).toBe(null);
    expect(rt.host.classes.at(-1)?.value).toBe("");
    rt.patchProp(button, "class", null, "");
    expect(button.getAttribute("class")).toBe("");
    rt.patchProp(button, "class", "", null);
    expect(button.hasAttribute("class")).toBe(false);
  });

  test("a transition class is part of the attribute, and a read-modify-write does not keep it for good", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "class", null, "card");
    button._vtc = new Set(["v-enter-active"]);
    rt.writeClass(button);
    expect(button.className).toBe("card v-enter-active");
    expect(button.classList.contains("v-enter-active")).toBe(true);
    button.className += " lit";
    expect(rt.host.classes.at(-1)?.value).toBe("card lit v-enter-active");
    button._vtc = undefined;
    rt.writeClass(button);
    expect(button.className).toBe("card lit");
    button._vtc = new Set(["v-leave-active"]);
    button.classList.remove("v-leave-active");
    expect(button._vtc).toBeUndefined();
    expect(button.className).toBe("card lit");
  });

  test("a class change is an attributes mutation", async () => {
    const rt = createRuntime();
    const { section, button } = tree(rt);
    const seen: RealmObserverEntry[] = [];
    new rt.global.MutationObserver((records) => seen.push(...records)).observe(section, {
      attributes: true,
      attributeFilter: ["class"],
      attributeOldValue: true,
      subtree: true,
    });
    rt.patchProp(button, "class", null, "x");
    button.classList.add("y");
    button.classList.add("y"); // no change, no record
    await Promise.resolve();
    expect(seen.map((r) => [r.attributeName, r.oldValue])).toEqual([
      ["class", null],
      ["class", "x"],
    ]);
  });
});

describe("mirror nodes are not reactive state", () => {
  test("a node carries Vue's markRaw flag, as a real DOM node effectively does", () => {
    // In a browser `reactive(divElement)` returns the element (an invalid reactive target). Our mirrors stringify as
    // `[object Object]`, so without the flag Vue proxies them and identity comparisons (`set.has(node)`) fail, which
    // reka-ui's layer stack and focus guards rely on.
    const rt = createRuntime();
    const { button, text } = tree(rt);
    expect(skipFlag(button)).toBe(true);
    expect(skipFlag(text)).toBe(true);
  });
});

describe("custom events and dispatchEvent", () => {
  test("a component's own event round-trips through the normal propagation walk", () => {
    const rt = createRuntime();
    const { section: outer, button: inner } = tree(rt);
    const seen: string[] = [];
    inner.addEventListener("select.select", (e) => seen.push(`at-target:${e.type}`), { once: true });
    outer.addEventListener("select.select", () => seen.push("bubbled"));

    const ok = inner.dispatchEvent(new CustomEvent("select.select", { bubbles: true, cancelable: true }));
    expect(seen).toEqual(["at-target:select.select", "bubbled"]);
    expect(ok).toBe(true);
    inner.dispatchEvent(new CustomEvent("select.select", { bubbles: true }));
    expect(seen.filter((s) => s.startsWith("at-target")).length).toBe(1);
  });

  test("a custom event registers NO host listener and produces no advisory", () => {
    const rt = createRuntime();
    const { button: inner } = tree(rt);
    inner.addEventListener("focusScope.autoFocusOnMount", () => {});
    expect(rt.host.listeners).toEqual([{ id: inner.__skId, code: rt.EV_CODES.click, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("the CSS animation/transition events are delivered, not advised", () => {
    for (const name of ["transitionend", "animationstart", "animationend", "animationcancel"] as const) {
      const rt = createRuntime();
      const { button: inner } = tree(rt);
      inner.addEventListener(name, () => {});
      expect(rt.advisories()).toEqual([]);
      expect(rt.flagsFor(rt.EV_CODES[name])).toEqual([{ id: inner.__skId, code: rt.EV_CODES[name], on: true }]);
    }
  });

  test("eventPhase, currentTarget and composedPath() read as a browser's, during the walk and after it", () => {
    const rt = createRuntime();
    const { section: outer, button: inner } = tree(rt);
    const seen: string[] = [];
    const who = (t: unknown) => (t === inner ? "inner" : t === outer ? "outer" : String(t));
    const note = (where: string) => (e: Event) => {
      const path: unknown[] = e.composedPath();
      seen.push(`${where}:${e.eventPhase}:${who(e.currentTarget)}:${path[0] === inner}:${path[1] === outer}`);
    };
    outer.addEventListener("x.probe", note("capture"), { capture: true });
    inner.addEventListener("x.probe", note("target"));
    outer.addEventListener("x.probe", note("bubble"));

    const ev = new CustomEvent("x.probe", { bubbles: true });
    inner.dispatchEvent(ev);
    expect(seen).toEqual([
      "capture:1:outer:true:true", // CAPTURING_PHASE
      "target:2:inner:true:true", //  AT_TARGET
      "bubble:3:outer:true:true", //  BUBBLING_PHASE
    ]);
    expect(ev.currentTarget).toBe(null);
    expect(ev.eventPhase).toBe(0);
    expect(ev.composedPath()).toEqual([]);
    expect(ev.target).toBe(inner);
  });

  test("…and a listener that throws is reported, not the end of the dispatch", () => {
    const reported: (() => void)[] = [];
    const rt = createRuntime({ globals: { queueMicrotask: (f: () => void) => reported.push(f) } });
    const { button: inner } = tree(rt);
    const ran: string[] = [];
    inner.addEventListener("x.boom", () => {
      throw new Error("listener");
    });
    inner.addEventListener("x.boom", () => ran.push("same node"));
    inner.parentNode?.addEventListener("x.boom", () => ran.push("ancestor"));
    const ev = new CustomEvent("x.boom", { bubbles: true });
    expect(inner.dispatchEvent(ev)).toBe(true);
    expect(ran).toEqual(["same node", "ancestor"]);
    expect(reported.length).toBe(1);
    expect(() => reported[0]?.()).toThrow("listener");
    expect(ev.currentTarget).toBe(null);
    expect(ev.eventPhase).toBe(0);
  });

  test("preventDefault on a dispatched event is reported to the dispatcher", () => {
    const rt = createRuntime();
    const { button: inner } = tree(rt);
    inner.addEventListener("menu.itemSelect", (e) => e.preventDefault());
    const ok = inner.dispatchEvent(new CustomEvent("menu.itemSelect", { bubbles: true, cancelable: true }));
    expect(ok).toBe(false);
  });
});

describe("document- and window-level listeners", () => {
  test("a document listener rides the app root's host listener bit", () => {
    const rt = createRuntime();
    const app = rt.createApp({ render: () => null });
    app.mount();
    rt.host.listeners.length = 0;

    rt.global.document.addEventListener("keydown", () => {});
    expect(rt.host.listeners.at(-1)).toMatchObject({ code: rt.EV_CODES.keydown, on: true });
  });

  test("an event dispatched at a node reaches document and window listeners after it", () => {
    const rt = createRuntime();
    const app = rt.createApp({ render: () => null });
    app.mount();
    const node = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(node, (rt.nodesById.get(rt.host.listeners[0]?.id ?? 1) ?? node) as MirrorElement, null);

    const order: string[] = [];
    node.addEventListener("keydown", () => order.push("node"));
    rt.global.document.addEventListener("keydown", () => order.push("document"));
    globalThis.addEventListener?.call?.(globalThis, "keydown", () => {});

    rt.host.dispatcher(node.__skId, rt.EV_CODES.keydown, 0, 0, 0, 0, "Escape");
    expect(order).toEqual(["node", "document"]);
  });
});

describe("ResizeObserver, after layout", () => {
  function world(globals: Record<string, unknown> = {}) {
    const rt = createRuntime({ globals });
    const nodes = tree(rt);
    rt.host.boundsById.set(nodes.section.__skId, "0,0,300,100");
    rt.host.boundsById.set(nodes.button.__skId, "0,0,100,40");
    rt.host.boundsById.set(nodes.span.__skId, "0,0,30,10");
    return { rt, ...nodes };
  }

  test("a target's first size is reported at the next step, and again only when it changes", () => {
    const { rt, button } = world();
    const seen: number[][] = [];
    const ob = new rt.global.ResizeObserver((entries) => seen.push(entries.map((e) => sizeOf(e).width)));
    expect(rt.host.layout.step()).toBe(-1); // nothing observed: the host calls nothing
    ob.observe(button);
    expect(rt.host.layout.pending).toBe(true);
    expect(rt.host.layout.step()).toBe(1);
    expect(seen).toEqual([[100]]);
    expect(rt.host.layout.step()).toBe(0);
    expect(seen).toEqual([[100]]);
    rt.host.boundsById.set(button.__skId, "0,0,250,40");
    rt.host.layout.step();
    expect(seen).toEqual([[100], [250]]);
    ob.disconnect();
    rt.host.layout.step();
    expect(rt.host.layout.observing).toBe(false);
  });

  test("an unpainted target reports 0x0 at once, then its size", () => {
    const { rt, span } = world();
    rt.host.boundsById.delete(span.__skId);
    const seen: number[][] = [];
    new rt.global.ResizeObserver((entries) => seen.push(entries.map((e) => sizeOf(e).height))).observe(span);
    rt.host.layout.step();
    expect(seen).toEqual([[0]]);
    rt.host.boundsById.set(span.__skId, "0,0,30,10");
    rt.host.layout.step();
    expect(seen).toEqual([[0], [10]]);
  });

  test("the entry: the layout box, contentRect at (0, 0), device pixels, and the observer as `this`", () => {
    const { rt, button } = world({ devicePixelRatio: 1.5 });
    rt.host.boundsById.set(button.__skId, "40,60,101,40");
    let got = null as { entry: ResizeEntry; o: unknown } | null;
    let self: unknown = null;
    const ob = new rt.global.ResizeObserver(function (this: unknown, entries, o) {
      self = this;
      got = { entry: entries[0] as unknown as ResizeEntry, o };
    });
    ob.observe(button);
    rt.host.layout.step();
    expect(self).toBe(ob);
    expect(got!.o).toBe(ob);
    expect(got!.entry.target).toBe(button);
    expect(got!.entry.contentRect).toMatchObject({ x: 0, y: 0, top: 0, left: 0, width: 101, height: 40 });
    expect(got!.entry.borderBoxSize[0]).toEqual({ inlineSize: 101, blockSize: 40 });
    expect(got!.entry.contentBoxSize[0]).toEqual({ inlineSize: 101, blockSize: 40 });
    expect(got!.entry.devicePixelContentBoxSize[0]).toEqual({ inlineSize: 152, blockSize: 60 });
  });

  test("what a callback changes is laid out and reported in the same step, deeper targets only", () => {
    const { rt, section, button } = world();
    const order: string[] = [];
    let grow = false;
    new rt.global.ResizeObserver(() => {
      order.push("section");
      grow = true;
    }).observe(section);
    new rt.global.ResizeObserver((e) => order.push(`button ${sizeOf(e[0]).width}`)).observe(button);
    expect(rt.host.layout.step()).toBe(1); // both first sizes, one round
    expect(order).toEqual(["section", "button 100"]);
    order.length = 0;
    grow = false;

    rt.host.boundsById.set(section.__skId, "0,0,320,100");
    const rounds = rt.host.layout.step(() => {
      if (grow) rt.host.boundsById.set(button.__skId, "0,0,180,40");
      grow = false;
    });
    expect(rounds).toBe(2);
    expect(order).toEqual(["section", "button 180"]);
    expect(rt.host.layout.loopErrors).toBe(0);
  });

  test("a change at or above the depth just reported is skipped: the loop error, and the next step reports it", () => {
    const events: ErrorEventLike[] = [];
    const onerror: unknown[][] = [];
    const { rt, section, button } = world({
      dispatchEvent: (ev: ErrorEventLike) => (events.push(ev), true),
    });
    rt.global.onerror = (...args: unknown[]) => void onerror.push(args);
    const sizes: string[] = [];
    let shrinkSection = false;
    new rt.global.ResizeObserver((e) => sizes.push(`section ${sizeOf(e[0]).width}`)).observe(section);
    new rt.global.ResizeObserver((e) => {
      sizes.push(`button ${sizeOf(e[0]).width}`);
      shrinkSection = true;
    }).observe(button);
    rt.host.layout.step(() => (shrinkSection = false));
    expect(sizes).toEqual(["section 300", "button 100"]);
    sizes.length = 0;

    rt.host.boundsById.set(button.__skId, "0,0,120,40");
    rt.host.layout.step(() => {
      if (shrinkSection) rt.host.boundsById.set(section.__skId, "0,0,280,100");
      shrinkSection = false;
    });
    expect(sizes).toEqual(["button 120"]);
    expect(rt.host.layout.loopErrors).toBe(1);
    const ev = events.find((e) => e.type === "error")!;
    expect(ev).toBeInstanceOf(rt.global.ErrorEvent);
    expect(ev.message).toBe("ResizeObserver loop completed with undelivered notifications.");
    expect(ev.cancelable).toBe(true);
    expect(ev.error).toBe(null);
    expect(onerror).toEqual([["ResizeObserver loop completed with undelivered notifications.", "", 0, 0, null]]);
    expect(rt.host.layout.pending).toBe(true);
    rt.host.layout.step();
    expect(sizes).toEqual(["button 120", "section 280"]);
    expect(rt.host.layout.loopErrors).toBe(1);
  });

  test("an observer disconnected by an earlier callback in the round is not called", () => {
    const { rt, section, button } = world();
    const calls: string[] = [];
    let second: RealmObserver | null = null;
    new rt.global.ResizeObserver(() => {
      calls.push("first");
      second?.disconnect();
    }).observe(section);
    second = new rt.global.ResizeObserver(() => calls.push("second"));
    second.observe(button);
    rt.host.layout.step();
    expect(calls).toEqual(["first"]);
  });

  test("a callback that throws is reported after it, and the next observer is still called", () => {
    const later: (() => void)[] = [];
    const { rt, section, button } = world({ queueMicrotask: (fn: () => void) => later.push(fn) });
    const calls: string[] = [];
    new rt.global.ResizeObserver(() => {
      calls.push("throws");
      throw new Error("boom");
    }).observe(section);
    new rt.global.ResizeObserver(() => calls.push("next")).observe(button);
    rt.host.layout.step();
    expect(calls).toEqual(["throws", "next"]);
    expect(later.length).toBe(1);
    expect(() => later[0]!()).toThrow("boom");
  });

  test("a host without the step: polled from animation frames, then a slow timer while nothing changes", () => {
    const queue: (() => void)[] = [];
    const timers = new Map<number, { cb: () => void; ms: number }>();
    let nextTimer = 1;
    const rt = createRuntime({
      globals: {
        requestAnimationFrame: (cb: () => void) => queue.push(cb),
        setTimeout: (cb: () => void, ms: number) => (timers.set(nextTimer, { cb, ms }), nextTimer++),
        clearTimeout: (id: number) => timers.delete(id),
      },
      layoutWatch: false,
    });
    const tick = (n = 1) => {
      for (let i = 0; i < n; i++) queue.splice(0).forEach((cb) => cb());
    };
    const { button } = tree(rt);
    rt.host.boundsById.set(button.__skId, "0,0,100,40");
    const seen: number[][] = [];
    new rt.global.ResizeObserver((entries) => seen.push(entries.map((e) => sizeOf(e).width))).observe(button);
    tick(10);
    expect(seen).toEqual([[100]]);
    expect(queue.length).toBe(0);
    const slow = [...timers].filter(([, t]) => t.ms === 250);
    expect(slow.length).toBe(1);
    rt.host.boundsById.set(button.__skId, "0,0,250,40");
    const [id, t] = slow[0]!;
    timers.delete(id);
    t.cb();
    tick();
    expect(seen).toEqual([[100], [250]]);
  });
});

describe("missing globals (G5)", () => {
  test("MutationObserver sees real childList and attribute mutations, batched on a microtask", async () => {
    const rt = createRuntime();
    const { section, button } = tree(rt);
    const seen: RealmObserverEntry[] = [];
    const ob = new rt.global.MutationObserver((records) => seen.push(...records));
    ob.observe(section, { childList: true, attributes: true, subtree: true });

    const extra = rt.nodeOps.createElement("i");
    rt.nodeOps.insert(extra, section, null);
    rt.patchProp(button, "data-state", null, "open");
    expect(seen).toEqual([]);
    await Promise.resolve();
    await Promise.resolve();

    expect(seen.map((r) => r.type)).toEqual(["childList", "attributes"]);
    expect([...(seen[0]!.addedNodes as Iterable<unknown>)]).toEqual([extra]);
    expect(seen[1]!.target).toBe(button);
    expect(seen[1]!.attributeName).toBe("data-state");

    ob.disconnect();
    rt.nodeOps.remove(extra);
    await Promise.resolve();
    expect(seen.length).toBe(2);
  });

  test("an observer without subtree only hears about its own node", async () => {
    const rt = createRuntime();
    const { section, button } = tree(rt);
    const seen: RealmObserverEntry[] = [];
    const ob = new rt.global.MutationObserver((r) => seen.push(...r));
    ob.observe(section, { childList: true });
    rt.patchProp(button, "data-x", null, "1");
    await Promise.resolve();
    await Promise.resolve();
    expect(seen).toEqual([]);
  });

  test("observe(document) means the root element — 'watch everything' is not a no-op", async () => {
    const rt = createRuntime();
    const { section, button } = tree(rt);
    const seen: RealmObserverEntry[] = [];
    const ob = new rt.global.MutationObserver((r) => seen.push(...r));
    ob.observe({ documentElement: section } as unknown as MirrorNode, { childList: true, subtree: true });
    const extra = rt.nodeOps.createElement("i");
    rt.nodeOps.insert(extra, button, null);
    await Promise.resolve();
    await Promise.resolve();
    expect(seen.map((r) => r.type)).toEqual(["childList"]);
    ob.disconnect();
  });

  test("DocumentFragment is a real (detached) node, so a Teleport into one works", () => {
    const rt = createRuntime();
    const frag = new (rt.global.DocumentFragment as new () => MirrorElement)();
    expect(frag.nodeType).toBe(11);
    const child = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(child, frag, null);
    expect([...frag.children]).toEqual([child]);
    expect([...frag.childNodes]).toEqual([child]);
    expect(child.parentNode).toBe(frag);
    expect(child.parentElement).toBe(null);
    expect(child.isConnected).toBe(false);
  });

  test("createTreeWalker really walks the mirror tree", () => {
    const rt = createRuntime();
    const { root, section, button, span } = tree(rt);
    const doc = rt.global.document as unknown as {
      createTreeWalker(root: MirrorNode, what: number): { nextNode(): MirrorNode | null };
    };
    const walker = doc.createTreeWalker(root, nodeFilter(rt).SHOW_ELEMENT);
    const out: MirrorNode[] = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) out.push(n);
    expect(out).toEqual([section, button, span]);
  });

  test("HTMLInputElement answers for the tag it names", () => {
    const rt = createRuntime();
    const input = rt.nodeOps.createElement("input");
    const div = rt.nodeOps.createElement("div");
    expect(input instanceof (rt.global.HTMLInputElement as Constructor)).toBe(true);
    expect(div instanceof (rt.global.HTMLInputElement as Constructor)).toBe(false);
    expect(div instanceof (rt.global.HTMLDivElement as Constructor)).toBe(true);
  });

  test("getComputedStyle reads the host's computed values, and empty for the rest", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.computedById.set(
      button.__skId,
      "display=flex\nposition=fixed\ntransition-duration=0.15s\n--primary=240 5.9% 10%\n",
    );
    const cs = computedStyleOf(rt, button);
    expect(cs.display).toBe("flex");
    expect(cs.position).toBe("fixed");
    expect(cs.transitionDuration).toBe("0.15s");
    expect(cs.getPropertyValue("--primary")).toBe("240 5.9% 10%");
    expect(rt.advisories()).toEqual([]);

    expect(cs.letterSpacing).toBe("");
  });

  test('getComputedStyle: a node the host has no style for reads "" for everything, without an advisory', () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    const cs = computedStyleOf(rt, button);
    for (const k of [
      "width",
      "height",
      "position",
      "direction",
      "animationName",
      "transitionDelay",
      "transitionDuration",
    ]) {
      expect(cs[k]).toBe("");
    }
    expect(cs.getPropertyValue("--chat-width")).toBe("");
    expect(rt.advisories()).toEqual([]);

    rt.host.computedById.set(button.__skId, "display=block\n");
    expect(cs.getPropertyValue("--undeclared")).toBe("");
    expect(rt.advisories()).toEqual([]);
  });
});
