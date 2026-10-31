// The document text selection's JS half (src/selection.ts): `getSelection()`,
// `Selection`, `Range`, `document.createRange()`, and the `selectstart` /
// `selectionchange` events. The host's half (hit testing, painting, the text a
// selection serializes to) is tested in Rust (crates/vn-tree/src/selection.rs);
// here the host is `host.selection`, a fake with the same protocol.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type N = { __vnId: number; childNodes: N[]; parentNode: N | null; data?: string; textContent: string };

/** `<div><p>"Hello " <b>big</b> " world"</p><p>own text</p></div>` under the root. */
function world(options: Parameters<typeof createRuntime>[0] = {}) {
  const rt = createRuntime(options);
  rt.createApp({ render: () => null }).mount();
  const doc = rt.global.document;
  const body = doc.body as N;
  const el = (tag: string, parent: N) => {
    const e = rt.nodeOps.createElement(tag);
    rt.nodeOps.insert(e, parent, null);
    return e as N;
  };
  const text = (s: string, parent: N) => {
    const t = rt.nodeOps.createText(s);
    rt.nodeOps.insert(t, parent, null);
    return t as N;
  };
  const div = el("div", body);
  const p1 = el("p", div);
  const hello = text("Hello ", p1);
  const b = el("b", p1);
  const big = text("big", b);
  const world_ = text(" world", p1);
  const p2 = el("p", div);
  rt.nodeOps.setElementText(p2, "own text");
  const sel = rt.global.getSelection();
  return { rt, doc, body, div, p1, hello, b, big, world: world_, p2, sel };
}

describe("getSelection", () => {
  test("window's and document's are one Selection; an older host has none", () => {
    const { rt, doc, sel } = world();
    expect(doc.getSelection()).toBe(sel);
    expect(sel).toBeInstanceOf(rt.global.Selection);
    expect(() => new rt.global.Selection()).toThrow(TypeError);
    expect(sel.rangeCount).toBe(0);
    expect(sel.type).toBe("None");
    expect(sel.isCollapsed).toBe(true);
    expect(sel.anchorNode).toBe(null);
    expect(() => sel.getRangeAt(0)).toThrow(/IndexSizeError|not a valid index/);

    const old = createRuntime({ selection: false });
    expect(old.global.getSelection).toBeUndefined();
    expect(old.global.Range).toBeUndefined();
  });

  test("it reads the host's anchor and focus, offsets and direction", () => {
    const { rt, sel, hello, big } = world();
    rt.host.selection.sel = [big.__vnId, 2, 0, hello.__vnId, 1, 0];
    rt.host.selection.text = "ello b";
    expect(sel.anchorNode).toBe(big);
    expect(sel.anchorOffset).toBe(2);
    expect(sel.focusNode).toBe(hello);
    expect(sel.focusOffset).toBe(1);
    expect(sel.isCollapsed).toBe(false);
    expect(sel.type).toBe("Range");
    expect(sel.direction).toBe("backward");
    expect(sel.rangeCount).toBe(1);
    expect(String(sel)).toBe("ello b");
    // The range is in document order whatever the direction.
    const r = sel.getRangeAt(0);
    expect([r.startContainer, r.startOffset, r.endContainer, r.endOffset]).toEqual([hello, 1, big, 2]);
    expect(r.toString()).toBe("ello bi");
    // The same range while the selection stays put.
    expect(sel.getRangeAt(0)).toBe(r);
  });

  test("a boundary in an element's own text is handed out as a Text node", () => {
    const { rt, sel, p2 } = world();
    expect(p2.childNodes.length).toBe(0);
    rt.host.selection.sel = [p2.__vnId, 1, 1, p2.__vnId, 4, 1];
    const a = sel.anchorNode as N;
    expect(p2.childNodes.length).toBe(1);
    expect(a).toBe(p2.childNodes[0]);
    expect(a.data).toBe("own text");
    expect(sel.anchorOffset).toBe(1);
    expect(sel.focusOffset).toBe(4);
    expect(rt.host.selection.rebases).toEqual([[p2.__vnId, a.__vnId]]);
    expect(p2.textContent).toBe("own text");
  });

  test("the setters reach the host, and a node outside the document is ignored", () => {
    const { rt, sel, hello, world: w, div, doc } = world();
    sel.collapse(hello, 2);
    expect(rt.host.selection.sets.at(-1)).toEqual([hello.__vnId, 2, hello.__vnId, 2]);
    expect(sel.type).toBe("Caret");
    sel.extend(w, 3);
    expect(rt.host.selection.sets.at(-1)).toEqual([hello.__vnId, 2, w.__vnId, 3]);
    sel.setBaseAndExtent(w, 3, hello, 0);
    expect(sel.direction).toBe("backward");
    sel.selectAllChildren(div);
    expect(rt.host.selection.sets.at(-1)).toEqual([div.__vnId, 0, div.__vnId, 2]);
    sel.collapseToEnd();
    expect(rt.host.selection.sets.at(-1)).toEqual([div.__vnId, 2, div.__vnId, 2]);
    sel.removeAllRanges();
    expect(rt.host.selection.sets.at(-1)).toEqual([0, 0, 0, 0]);
    expect(sel.rangeCount).toBe(0);
    expect(() => sel.collapseToStart()).toThrow(/InvalidStateError|no selection/);
    expect(() => sel.collapse(hello, 99)).toThrow(/IndexSizeError|no child at offset/);
    const loose = doc.createTextNode("loose");
    const before = rt.host.selection.sets.length;
    sel.collapse(loose, 1);
    expect(rt.host.selection.sets.length).toBe(before);
  });

  test("addRange ties the range to the selection: changing it moves the selection", () => {
    const { rt, sel, doc, hello, world: w } = world();
    const r = doc.createRange();
    r.setStart(hello, 1);
    r.setEnd(w, 2);
    sel.addRange(r);
    expect(rt.host.selection.sets.at(-1)).toEqual([hello.__vnId, 1, w.__vnId, 2]);
    expect(sel.getRangeAt(0)).toBe(r);
    r.setEnd(w, 4);
    expect(rt.host.selection.sets.at(-1)).toEqual([hello.__vnId, 1, w.__vnId, 4]);
    // A second range is ignored, as in Chromium.
    const other = doc.createRange();
    other.selectNodeContents(w);
    sel.addRange(other);
    expect(rt.host.selection.sets.at(-1)).toEqual([hello.__vnId, 1, w.__vnId, 4]);
    sel.removeRange(r);
    expect(sel.rangeCount).toBe(0);
    // Once removed, the range is the script's own again.
    const n = rt.host.selection.sets.length;
    r.collapse(true);
    expect(rt.host.selection.sets.length).toBe(n);
  });

  test("containsNode, whole and partial", () => {
    const { rt, sel, hello, b, big } = world();
    rt.host.selection.sel = [hello.__vnId, 0, 0, big.__vnId, 1, 0];
    expect(sel.containsNode(hello)).toBe(true);
    expect(sel.containsNode(b)).toBe(false);
    expect(sel.containsNode(b, true)).toBe(true);
  });
});

describe("Range", () => {
  test("boundaries: a start after the end collapses it, and the offsets are checked", () => {
    const { rt, doc, hello, world: w, p1, body } = world();
    const r = doc.createRange();
    expect(r).toBeInstanceOf(rt.global.Range);
    expect(new rt.global.Range().startContainer).toBe(body);
    r.setStart(w, 2);
    expect([r.endContainer, r.endOffset]).toEqual([w, 2]);
    r.setStart(hello, 1);
    expect(r.collapsed).toBe(false);
    expect(r.commonAncestorContainer).toBe(p1);
    expect(() => r.setStart(hello, 7)).toThrow(/IndexSizeError|no child at offset/);
    expect(() => r.setStart({}, 0)).toThrow(TypeError);
    r.setStartBefore(hello);
    expect([r.startContainer, r.startOffset]).toEqual([p1, 0]);
    r.setEndAfter(w);
    expect([r.endContainer, r.endOffset]).toEqual([p1, 3]);
    r.collapse(true);
    expect(r.collapsed).toBe(true);
  });

  test("toString is the DOM's: Text data joined, no line breaks", () => {
    const { doc, hello, world: w, div } = world();
    const r = doc.createRange();
    r.setStart(hello, 2);
    r.setEnd(w, 3);
    expect(r.toString()).toBe("llo big wo");
    r.selectNodeContents(div);
    expect(r.toString()).toBe("Hello big worldown text");
  });

  test("comparisons", () => {
    const { rt, doc, hello, big, world: w, p1, b } = world();
    const a = doc.createRange();
    a.setStart(hello, 1);
    a.setEnd(big, 2);
    const c = doc.createRange();
    c.selectNode(b);
    const R = rt.global.Range;
    expect(a.compareBoundaryPoints(R.START_TO_START, c)).toBe(-1);
    expect(a.compareBoundaryPoints(R.END_TO_END, c)).toBe(-1);
    expect(a.compareBoundaryPoints(R.START_TO_END, c)).toBe(1);
    expect(a.comparePoint(hello, 0)).toBe(-1);
    expect(a.comparePoint(big, 1)).toBe(0);
    expect(a.comparePoint(w, 0)).toBe(1);
    expect(a.isPointInRange(p1, 1)).toBe(true);
    expect(a.intersectsNode(w)).toBe(false);
    expect(a.intersectsNode(b)).toBe(true);
    const copy = a.cloneRange();
    expect([copy.startContainer, copy.startOffset, copy.endContainer, copy.endOffset]).toEqual([hello, 1, big, 2]);
    expect(copy).not.toBe(a);
  });

  test("deleteContents inside a text, and across nodes", () => {
    const { doc, hello, world: w, p1 } = world();
    const r = doc.createRange();
    r.setStart(hello, 1);
    r.setEnd(hello, 3);
    r.deleteContents();
    expect(hello.data).toBe("Hlo ");
    expect(r.collapsed).toBe(true);
    r.setStart(hello, 1);
    r.setEnd(w, 3);
    r.deleteContents();
    expect(p1.textContent).toBe("Hrld");
    expect(p1.childNodes.length).toBe(2);
    // DOM: the start is not an ancestor of the end, so the new boundary is
    // just after the start's ancestor below the common one.
    expect(r.startContainer === p1 && r.startOffset === 1 && r.collapsed).toBe(true);
  });

  test("insertNode splits a text at the boundary", () => {
    const { doc, hello, p1 } = world();
    const r = doc.createRange();
    r.setStart(hello, 2);
    const x = doc.createTextNode("X");
    r.insertNode(x);
    expect(p1.textContent).toBe("HeXllo big world");
    expect(p1.childNodes[1]).toBe(x);
  });

  test("cloneContents copies the text, and says so when the range holds elements", () => {
    const { rt, doc, hello, world: w } = world();
    const r = doc.createRange();
    r.setStart(hello, 4);
    r.setEnd(w, 2);
    const f = r.cloneContents() as N;
    expect(f.textContent).toBe("o big w");
    expect(rt.advisories().some((a: string) => a.includes("cloneContents"))).toBe(true);
  });
});

describe("selection events", () => {
  test("the codes are the host's, and a document listener arms the root's bit", () => {
    const { rt, doc, body } = world();
    expect([rt.EV_CODES.selectstart, rt.EV_CODES.selectionchange]).toEqual([45, 46]);
    rt.host.listeners.length = 0;
    doc.addEventListener("selectionchange", () => {});
    expect(rt.flagsFor(46)).toEqual([{ id: body.__vnId, code: 46, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("selectionchange arrives at the document: not bubbling, not cancelable", () => {
    const { rt, doc, body } = world();
    const seen: string[] = [];
    let ev: { target: unknown; bubbles: boolean; cancelable: boolean; eventPhase: number } | null = null;
    rt.global.addEventListener("selectionchange", () => seen.push("window:capture"), true);
    rt.global.addEventListener("selectionchange", () => seen.push("window:bubble"));
    doc.addEventListener("selectionchange", (e: typeof ev) => {
      seen.push("document");
      ev = { target: e!.target, bubbles: e!.bubbles, cancelable: e!.cancelable, eventPhase: e!.eventPhase };
    });
    rt.patchProp(body, "onClick", null, () => {});
    expect(rt.host.dispatcher(body.__vnId, 46, 0, 0, 0, 0, undefined)).toBe(true);
    expect(seen).toEqual(["window:capture", "document"]);
    expect(ev).toEqual({ target: doc, bubbles: false, cancelable: false, eventPhase: 2 });
  });

  test("document.onselectionchange is the IDL handler", () => {
    const { rt, doc, body } = world();
    let n = 0;
    const fn = () => n++;
    doc.onselectionchange = fn;
    expect(doc.onselectionchange).toBe(fn);
    rt.host.dispatcher(body.__vnId, 46, 0, 0, 0, 0, undefined);
    doc.onselectionchange = null;
    rt.host.dispatcher(body.__vnId, 46, 0, 0, 0, 0, undefined);
    expect(n).toBe(1);
    expect(doc.onselectionchange).toBe(null);
  });

  test("selectstart bubbles to the document and is cancelable; an element selectionchange advises", () => {
    const { rt, doc, big, div } = world();
    const order: string[] = [];
    div.addEventListener("selectstart", () => order.push("div"));
    doc.addEventListener("selectstart", (e: { preventDefault(): void }) => {
      order.push("document");
      e.preventDefault();
    });
    expect(rt.host.dispatcher(big.__vnId, 45, 0, 0, 0, 0, undefined)).toBe(false);
    expect(order).toEqual(["div", "document"]);
    const p = div as unknown as { onselectstart: unknown };
    expect(p.onselectstart).toBe(null);
    div.addEventListener("selectionchange", () => {});
    expect(rt.advisories().some((a: string) => a.includes("selectionchange at document only"))).toBe(true);
  });
});
