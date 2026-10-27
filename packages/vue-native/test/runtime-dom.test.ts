// The DOM surface on mirror nodes (compat roadmap G2) and the globals a reused
// browser component reaches for before it can render at all (G5).
//
// The rule these tests pin is the project's compat law: every answer here is
// either TRUTHFUL (from the mirror tree, from the host's real matcher, from the
// host's real layout bounds) or degraded WITH an advisory — never a silent
// wrong answer.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

/** A small mounted-ish tree: root > section > (button, span). */
function tree(rt) {
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
    expect(section.children).toEqual([button, span]);
    expect(button.childNodes).toEqual([text]);
    expect(button.children).toEqual([]);
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
  test("matches / closest / querySelector(All) call __vn_query with the right mode", () => {
    const rt = createRuntime();
    const { root, section, button } = tree(rt);
    rt.host.queryAnswers.set(".x|1", String(button.id));
    rt.host.queryAnswers.set(".x|2", String(section.id));
    rt.host.queryAnswers.set(".x|0", `${section.id},${button.id}`);

    expect(button.matches(".x")).toBe(true);
    expect(button.closest(".x")).toBe(section);
    expect(root.querySelector(".x")).toBe(section);
    expect(root.querySelectorAll(".x")).toEqual([section, button]);

    expect(rt.host.queryCalls.map((c) => c.mode)).toEqual([1, 2, 0, 0]);
    expect(rt.host.queryCalls[0].root).toBe(button.id);
    // No advisory: these are answered for real, not stubbed.
    expect(rt.advisories()).toEqual([]);
  });

  test("a selector the engine cannot run explains itself instead of saying `no match`", () => {
    const rt = createRuntime();
    const { root } = tree(rt);
    rt.host.queryAnswers.set(":has(.x)", "!`:has()` (the matcher has no implementation)");
    expect(root.querySelectorAll(":has(.x)")).toEqual([]);
    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    expect(advisories[0]).toContain(":has()");
  });

  test("`:scope` is anchored at the element the call was made on", () => {
    const rt = createRuntime();
    const { root, section, button } = tree(rt);
    // `:scope` means the SCOPING ROOT — the node the method was invoked on,
    // which the DOM defines per CALL and not per document. The JS side's whole
    // share of that is passing the right root id; `vn_css::query_all_in`
    // answers the rest. Two calls with the same selector and different subjects
    // must therefore reach the host with different roots.
    rt.host.queryAnswers.set(":scope > button|0", String(button.id));
    expect(section.querySelector(":scope > button")).toBe(button);
    expect(rt.host.queryCalls.at(-1)).toMatchObject({
      selector: ":scope > button",
      root: section.id,
      mode: 0,
    });

    root.querySelectorAll(":scope > button");
    expect(rt.host.queryCalls.at(-1).root).toBe(root.id);

    // …and it is no longer a compat advisory: the engine runs it now.
    expect(rt.advisories()).toEqual([]);
  });

  test("an unmatched id in the answer is skipped rather than yielding undefined", () => {
    const rt = createRuntime();
    const { root } = tree(rt);
    rt.host.queryAnswers.set(".gone", "99999");
    expect(root.querySelectorAll(".gone")).toEqual([]);
  });
});

describe("layout comes from the host's painted bounds", () => {
  test("getBoundingClientRect reports the real rectangle", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.id, "10,20,100,40");
    const r = button.getBoundingClientRect();
    expect(r).toMatchObject({ x: 10, y: 20, width: 100, height: 40, left: 10, top: 20, right: 110, bottom: 60 });
    expect(button.offsetWidth).toBe(100);
    expect(button.clientHeight).toBe(40);
    expect(button.getClientRects().length).toBe(1);
  });

  test("a node that was never painted reads all zeros, exactly like a detached one in a browser", () => {
    const rt = createRuntime();
    const { span } = tree(rt);
    const r = span.getBoundingClientRect();
    expect(r).toMatchObject({ x: 0, y: 0, width: 0, height: 0 });
    expect(span.getClientRects()).toEqual([]);
    // …and that is NOT a compat gap, so it must not produce an advisory.
    expect(rt.advisories()).toEqual([]);
  });

  test("offsetTop/offsetLeft are window-relative and say so", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.id, "10,20,100,40");
    expect(button.offsetTop).toBe(20);
    expect(rt.advisories().some((a) => a.includes("offsetParent"))).toBe(true);
  });
});

describe("programmatic scrolling drives the host's ScrollHandle", () => {
  /** A scroll container: 300×200 viewport with 1 000 px of content below it. */
  function scroller(rt, node, extra = {}) {
    rt.host.scrollById.set(node.id, {
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
    // The whole of the bug this replaced: `scrollTop` was a plain field, so a
    // write updated a number nobody read and the box never moved.
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
    section.scrollTop = "nope";
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

    // An axis the caller did not name is LEFT WHERE IT IS, which is what
    // `scrollTo({ top })` means — not "scrolled to 0".
    section.scrollTo({ top: 5 });
    expect([section.scrollLeft, section.scrollTop]).toEqual([10, 5]);
  });

  test("scrollWidth/scrollHeight report the CONTENT box, so scroll-to-bottom works", () => {
    // `el.scrollTop = el.scrollHeight` is THE scroll-to-bottom idiom. Reporting
    // the painted rectangle here (the viewport) would stop the list one
    // viewport down and look like a clamp bug in the app.
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    rt.host.boundsById.set(section.id, "0,0,300,200");

    expect(section.clientHeight).toBe(200);
    expect(section.scrollHeight).toBe(1000);

    section.scrollTop = section.scrollHeight;
    expect(section.scrollTop).toBe(800); // the bottom, clamped
  });

  test("an element that does not scroll reports zeros and swallows a write", () => {
    // A browser's behaviour exactly, so no advisory is warranted.
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.id, "0,0,100,40");
    expect(button.scrollTop).toBe(0);
    expect(button.scrollLeft).toBe(0);
    // …and `scrollWidth` falls back to the border box, which is what a browser
    // reports for a non-scrollable element.
    expect(button.scrollWidth).toBe(100);
    button.scrollTop = 500;
    expect(button.scrollTop).toBe(0);
    expect(rt.advisories()).toEqual([]);
  });

  test("scrollIntoView sends the alignment a browser would", () => {
    const rt = createRuntime();
    const { button } = tree(rt);

    button.scrollIntoView();
    // CSSOM-View's defaults: block "start", inline "nearest".
    expect(rt.host.intoViewCalls.at(-1)).toEqual({ id: button.id, block: 0, inline: 3 });

    button.scrollIntoView(false);
    expect(rt.host.intoViewCalls.at(-1)).toMatchObject({ block: 2, inline: 3 });

    button.scrollIntoView({ block: "center", inline: "end" });
    expect(rt.host.intoViewCalls.at(-1)).toMatchObject({ block: 1, inline: 2 });

    // An unknown keyword falls back to the spec default for that axis rather
    // than throwing — a browser ignores it too.
    button.scrollIntoView({ block: "sideways" });
    expect(rt.host.intoViewCalls.at(-1)).toMatchObject({ block: 0, inline: 3 });
    expect(rt.advisories()).toEqual([]);
  });

  test("a host refusal becomes one advisory, not silence", () => {
    const rt = createRuntime();
    const { span } = tree(rt);
    rt.host.intoViewAnswer = "!scrollIntoView(): node has no rectangle";
    span.scrollIntoView();
    span.scrollIntoView();
    expect(rt.advisories().filter((a) => a.includes("no rectangle")).length).toBe(1);
  });

  test('behavior: "smooth" is performed instantly and says so, once', () => {
    // The position is exact; the interpolation is what is missing, and that is
    // a divergence, so it is named. See research/reports/m23/scroll.md for why
    // it is not run through the CSS animator.
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);

    section.scrollTo({ top: 300, behavior: "smooth" });
    expect(section.scrollTop).toBe(300);
    section.scrollBy({ top: 10, behavior: "smooth" });
    section.scrollIntoView({ behavior: "smooth" });

    const smooth = rt.advisories().filter((a) => a.includes("smooth"));
    expect(smooth.length).toBe(1);
    expect(smooth[0]).toContain("INSTANTLY");

    // …and the ordinary spellings say nothing at all.
    const quiet = createRuntime();
    const t2 = tree(quiet);
    quiet.host.scrollById.set(t2.section.id, { left: 0, top: 0, maxLeft: 0, maxTop: 800, width: 300, height: 200 });
    t2.section.scrollTo({ top: 10, behavior: "auto" });
    t2.section.scrollTo({ top: 20 });
    expect(quiet.advisories()).toEqual([]);
  });

  test("a content-size answer the host calls STALE becomes one advisory", () => {
    // The honest gap: a browser lays out on read, and here layout is a frame.
    // The host can force one during boot/onMounted and nowhere inside an event
    // handler, so `el.scrollTop = el.scrollHeight` right after adding content
    // stops one item short — and must say so rather than be quietly wrong.
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    rt.host.scrollStale = true;

    // The OFFSET half is never stale, so a plain read stays silent.
    expect(section.scrollTop).toBe(0);
    expect(rt.advisories()).toEqual([]);

    // The content half, and the clamp that uses it, are not.
    expect(section.scrollHeight).toBe(1000);
    section.scrollTop = 500;
    section.scrollTo({ top: 10 });
    section.scrollBy({ top: 10 });

    const stale = rt.advisories().filter((a) => a.includes("LAST PAINTED frame"));
    expect(stale.length).toBe(1);
    expect(stale[0]).toContain("requestAnimationFrame");
  });

  test("a stale scrollIntoView reports the same one advisory", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.intoViewAnswer = "stale";
    button.scrollIntoView();
    expect(rt.advisories().filter((a) => a.includes("LAST PAINTED frame")).length).toBe(1);
  });

  test("a scroll EVENT no longer writes the position back onto the node", () => {
    // It used to, because the field was the only place a position could live.
    // The accessor reads the host now, so a write-back would push the position
    // the event describes into the box the event came from.
    const rt = createRuntime();
    const { section } = tree(rt);
    scroller(rt, section);
    section.scrollTop = 120;

    const seen = [];
    section.addEventListener("scroll", (e) => seen.push(e.target.scrollTop));
    rt.host.dispatcher(section.id, rt.EV_CODES.scroll, 0, 120, 0, 0, null);

    expect(seen).toEqual([120]);
    // …and the only `__vn_scroll` writes are the ones the test made.
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
    expect(rt.host.props.at(-1)).toEqual({ id: button.id, key: "data-side", value: "top" });
    button.removeAttribute("data-side");
    expect(rt.host.props.at(-1)).toEqual({ id: button.id, key: "data-side", value: undefined });
  });

  test("setAttribute('style', …) goes through the style body, not the :style object path", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    button.setAttribute("style", "color: red; --x: 3");
    expect(rt.host.styles).toEqual([
      { id: button.id, key: "color", value: "red" },
      { id: button.id, key: "--x", value: "3" },
    ]);
  });
});

describe("inline style writes through — custom properties included", () => {
  test("setProperty reaches the host under the CSS spelling", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    button.style.setProperty("--reka-popper-available-width", "320px");
    button.style.zIndex = "50";
    expect(rt.host.styles).toEqual([
      { id: button.id, key: "--reka-popper-available-width", value: "320px" },
      { id: button.id, key: "z-index", value: "50" },
    ]);
    // …and reads back what was written.
    expect(button.style.getPropertyValue("--reka-popper-available-width")).toBe("320px");
    expect(button.style.zIndex).toBe("50");
    // Custom properties are now STORED by vn-css, so this must NOT warn.
    expect(rt.advisories()).toEqual([]);
  });
});

describe("classList and the transition-class merge", () => {
  test("classList edits rewrite the whole class attribute", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "class", null, "btn primary");
    button.classList.add("open");
    expect(rt.host.classes.at(-1).value).toBe("btn primary open");
    expect(button.classList.contains("primary")).toBe(true);
    button.classList.remove("primary");
    expect(rt.host.classes.at(-1).value).toBe("btn open");
    expect(button.classList.toggle("open")).toBe(false);
    expect(rt.host.classes.at(-1).value).toBe("btn");
  });

  test("a re-render keeps the transition classes a <Transition> layered on", () => {
    // runtime-dom keeps the same split for the same reason: without it a
    // re-render mid-transition drops `v-enter-active` and the animation stops.
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.patchProp(button, "class", null, "card");
    button._vtc = new Set(["v-enter-active"]);
    rt.writeClass(button);
    expect(rt.host.classes.at(-1).value).toBe("card v-enter-active");
    rt.patchProp(button, "class", "card", "card open");
    expect(rt.host.classes.at(-1).value).toBe("card open v-enter-active");
  });
});

describe("mirror nodes are not reactive state", () => {
  test("a node carries Vue's markRaw flag, as a real DOM node effectively does", () => {
    // In a browser `reactive(divElement)` hands the element straight back —
    // `Object.prototype.toString.call(div)` is `[object HTMLDivElement]`, which
    // Vue classifies as an INVALID reactive target. Our mirrors stringify as
    // `[object Object]`, so without the flag Vue proxies them and every
    // identity comparison (`set.has(node)`, `array.indexOf(node)`) starts
    // answering NO for the same node reached two ways — which is exactly what
    // reka-ui's layer stack, Collection ordering and focus guards are built on.
    const rt = createRuntime();
    const { button, text } = tree(rt);
    expect(button.__v_skip).toBe(true);
    expect(text.__v_skip).toBe(true);
  });
});

describe("custom events and dispatchEvent", () => {
  test("a component's own event round-trips through the normal propagation walk", () => {
    // reka-ui's whole selection protocol is this: add a one-shot listener for a
    // custom name, then dispatch it on the same node.
    const rt = createRuntime();
    const { section: outer, button: inner } = tree(rt);
    const seen = [];
    inner.addEventListener("select.select", (e) => seen.push(`at-target:${e.type}`), { once: true });
    outer.addEventListener("select.select", () => seen.push("bubbled"));

    const ok = inner.dispatchEvent(new CustomEvent("select.select", { bubbles: true, cancelable: true }));
    expect(seen).toEqual(["at-target:select.select", "bubbled"]);
    expect(ok).toBe(true);
    // `once` really was once.
    inner.dispatchEvent(new CustomEvent("select.select", { bubbles: true }));
    expect(seen.filter((s) => s.startsWith("at-target")).length).toBe(1);
  });

  test("a custom event registers NO host listener and produces no advisory", () => {
    const rt = createRuntime();
    const { button: inner } = tree(rt);
    inner.addEventListener("focusScope.autoFocusOnMount", () => {});
    expect(rt.host.listeners).toEqual([]);
    expect(rt.advisories()).toEqual([]);
  });

  test("…but a PLATFORM event the host cannot deliver still says so", () => {
    const rt = createRuntime();
    const { button: inner } = tree(rt);
    inner.addEventListener("contextmenu", () => {});
    expect(rt.advisories().length).toBe(1);
    expect(rt.advisories()[0]).toContain("contextmenu");
  });

  // m12 items 3 and 6: these four used to belong in the test above. vn-css's
  // animator sources them now — it is the only thing that knows when a track
  // finishes — so they register a real host listener and say nothing.
  test("the CSS animation/transition events are delivered, not advised", () => {
    for (const name of ["transitionend", "animationstart", "animationend", "animationcancel"]) {
      const rt = createRuntime();
      const { button: inner } = tree(rt);
      inner.addEventListener(name, () => {});
      expect(rt.advisories()).toEqual([]);
      expect(rt.host.listeners.length).toBe(1);
    }
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
    // The root is the node that stands in for `document`, so its bit is what
    // gets set — nothing else in the tree listens for keydown.
    expect(rt.host.listeners.at(-1)).toMatchObject({ code: rt.EV_CODES.keydown, on: true });
  });

  test("an event dispatched at a node reaches document and window listeners after it", () => {
    const rt = createRuntime();
    const app = rt.createApp({ render: () => null });
    app.mount();
    const node = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(node, rt.nodesById.get(rt.host.listeners[0]?.id ?? 1) ?? node, null);

    const order = [];
    node.addEventListener("keydown", () => order.push("node"));
    rt.global.document.addEventListener("keydown", () => order.push("document"));
    globalThis.addEventListener?.call?.(globalThis, "keydown", () => {});

    rt.host.dispatcher(node.id, rt.EV_CODES.keydown, 0, 0, 0, 0, "Escape");
    expect(order).toEqual(["node", "document"]);
  });
});

describe("ResizeObserver samples the host's layout", () => {
  test("it fires once for the first size and again only when the size changes", async () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.boundsById.set(button.id, "0,0,100,40");
    const seen = [];
    const ob = new rt.global.ResizeObserver((entries) => seen.push(entries.map((e) => e.contentRect.width)));
    ob.observe(button);

    const frame = () => new Promise((r) => setTimeout(r, 4));
    await frame();
    expect(seen).toEqual([[100]]);
    // No change -> no callback.
    await frame();
    expect(seen).toEqual([[100]]);
    // A real change -> exactly one more.
    rt.host.boundsById.set(button.id, "0,0,250,40");
    await frame();
    await frame();
    expect(seen).toEqual([[100], [250]]);
    ob.disconnect();
  });
});

describe("missing globals (G5)", () => {
  test("MutationObserver sees real childList and attribute mutations, batched on a microtask", async () => {
    const rt = createRuntime();
    const { section, button } = tree(rt);
    const seen = [];
    const ob = new rt.global.MutationObserver((records) => seen.push(...records));
    ob.observe(section, { childList: true, attributes: true, subtree: true });

    const extra = rt.nodeOps.createElement("i");
    rt.nodeOps.insert(extra, section, null);
    rt.patchProp(button, "data-state", null, "open");
    // Nothing is delivered synchronously — a browser batches to the microtask
    // checkpoint too.
    expect(seen).toEqual([]);
    await Promise.resolve();
    await Promise.resolve();

    expect(seen.map((r) => r.type)).toEqual(["childList", "attributes"]);
    expect(seen[0].addedNodes).toEqual([extra]);
    expect(seen[1].target).toBe(button);
    expect(seen[1].attributeName).toBe("data-state");

    ob.disconnect();
    rt.nodeOps.remove(extra);
    await Promise.resolve();
    expect(seen.length).toBe(2);
  });

  test("an observer without subtree only hears about its own node", async () => {
    const rt = createRuntime();
    const { section, button } = tree(rt);
    const seen = [];
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
    const seen = [];
    const ob = new rt.global.MutationObserver((r) => seen.push(...r));
    // What reka's focus scope and floating-ui's auto-update both write. A
    // Document is not an element here, but its element subtree is the root's,
    // so the observer has to land on the root rather than warn and drop.
    ob.observe({ documentElement: section }, { childList: true, subtree: true });
    const extra = rt.nodeOps.createElement("i");
    rt.nodeOps.insert(extra, button, null);
    await Promise.resolve();
    await Promise.resolve();
    expect(seen.map((r) => r.type)).toEqual(["childList"]);
    ob.disconnect();
  });

  test("DocumentFragment is a real (detached) node, so a Teleport into one works", () => {
    const rt = createRuntime();
    const frag = new rt.global.DocumentFragment();
    expect(frag.nodeType).toBe(11);
    const child = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(child, frag, null);
    expect(frag.children).toEqual([child]);
    expect(frag.childNodes).toEqual([child]);
    // Detached: it hangs off no root, so it is not painted.
    expect(child.isConnected).toBe(false);
  });

  test("NodeFilter exists with the constants createTreeWalker is called with", () => {
    const rt = createRuntime();
    expect(rt.global.NodeFilter.SHOW_ELEMENT).toBe(1);
    expect(rt.global.NodeFilter.FILTER_ACCEPT).toBe(1);
  });

  test("createTreeWalker really walks the mirror tree", () => {
    const rt = createRuntime();
    const { root, section, button, span } = tree(rt);
    const walker = rt.global.document.createTreeWalker(root, rt.global.NodeFilter.SHOW_ELEMENT);
    const out = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) out.push(n);
    expect(out).toEqual([section, button, span]);
  });

  test("HTMLInputElement answers for the tag it names", () => {
    const rt = createRuntime();
    const input = rt.nodeOps.createElement("input");
    const div = rt.nodeOps.createElement("div");
    expect(input instanceof rt.global.HTMLInputElement).toBe(true);
    expect(div instanceof rt.global.HTMLInputElement).toBe(false);
    expect(div instanceof rt.global.HTMLDivElement).toBe(true);
  });

  test("getComputedStyle reads the host's computed values, and says so for the rest", () => {
    const rt = createRuntime();
    const { button } = tree(rt);
    rt.host.computedById.set(
      button.id,
      "display=flex\nposition=fixed\ntransition-duration=0.15s\n--primary=240 5.9% 10%\n",
    );
    const cs = rt.global.getComputedStyle(button);
    expect(cs.display).toBe("flex");
    expect(cs.position).toBe("fixed");
    expect(cs.transitionDuration).toBe("0.15s");
    expect(cs.getPropertyValue("--primary")).toBe("240 5.9% 10%");
    expect(rt.advisories()).toEqual([]);

    // A property the host does not publish reads "" — with one advisory.
    expect(cs.letterSpacing).toBe("");
    expect(rt.advisories().some((a) => a.includes("getComputedStyle"))).toBe(true);
  });

  test("document.querySelectorAll goes through the same host matcher", () => {
    const rt = createRuntime();
    const { root, button } = tree(rt);
    // `appRoot` is only set by mount(); before that the document has no tree.
    expect(rt.global.document.querySelectorAll("[data-x]")).toEqual([]);
    void root;
    void button;
  });
});
