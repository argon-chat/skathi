// Imperative tree mutation (`appendChild`, `insertBefore`, `removeChild`, `replaceChild`, ChildNode/ParentNode, `insertAdjacent*`)
// on mirror nodes with the DOM's semantics; every case also checks `host.tree`, since mirror and host must never disagree.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

function world() {
  const rt: Any = createRuntime();
  const doc: Any = rt.global.document;
  const tree = rt.host.tree;
  const shape = (n: Any) => tree.shape(n.__vnId);
  const hostParent = (n: Any) => {
    const p = tree.parent.get(n.__vnId);
    return p ? tree.tag.get(p) : p;
  };
  const vue = (tag: string, parent?: Any) => {
    const el = rt.nodeOps.createElement(tag);
    if (parent) rt.nodeOps.insert(el, parent, null);
    return el;
  };
  return { rt, doc, tree, shape, hostParent, vue };
}

function watch(rt: Any, target: Any) {
  const seen: string[] = [];
  const tag = (n: Any) => (n.nodeType === 3 ? `#${n.data}` : n.tagName.toLowerCase());
  new rt.global.MutationObserver((records: Any[]) => {
    for (const r of records) {
      seen.push(`${tag(r.target)}:${Array.from(r.addedNodes, (n: Any) => "+" + tag(n)).join("")}${Array.from(r.removedNodes, (n: Any) => "-" + tag(n)).join("")}`);
    }
  }).observe(target, { childList: true, subtree: true });
  return seen;
}

describe("document.createElement makes tree nodes", () => {
  test("a script-made element is a mirror node, and appendChild puts it in the host tree", () => {
    const { rt, doc, shape, hostParent, vue } = world();
    const placeholder = vue("span");
    const img = doc.createElement("img");
    expect(img.tagName).toBe("IMG");
    expect(rt.nodeById(img.__vnId)).toBe(img);
    expect(img instanceof rt.global.HTMLImageElement).toBe(true);
    expect(img.parentNode).toBeNull();
    expect(hostParent(img)).toBe(0);
    expect(placeholder.appendChild(img)).toBe(img);
    expect(img.parentNode).toBe(placeholder);
    expect(placeholder.childNodes).toEqual([img]);
    expect(shape(placeholder)).toEqual(["img"]);
    expect(rt.nodesById.get(img.__vnId)).toBe(img);
    expect(rt.looseRoots.size).toBe(0);
    expect(rt.host.tree.errors).toEqual([]);
  });

  test("textContent on a detached element works (it used to throw on `childNodes`)", () => {
    const { doc } = world();
    const div = doc.createElement("div");
    div.textContent = "x";
    expect(div.textContent).toBe("x");
  });

  test("the name is checked, the engine keeps <a>, <input> and the head-only tags, and the SVG namespace keeps case", () => {
    const { rt, doc } = world();
    expect(() => doc.createElement("")).toThrow("is not a valid name");
    expect(() => doc.createElement("a b")).toThrow(expect.objectContaining({ name: "InvalidCharacterError" }));
    for (const t of ["a", "input", "style", "link", "script"]) {
      expect([t, rt.isMirrorNode?.(doc.createElement(t)) ?? "__vnId" in doc.createElement(t)]).toEqual([t, false]);
    }
    const grad = doc.createElementNS("http://www.w3.org/2000/svg", "linearGradient");
    expect(grad.tag).toBe("linearGradient");
    expect(doc.createElementNS("http://www.w3.org/1999/xhtml", "DIV").tagName).toBe("DIV");
    const c = doc.createComment("note");
    expect([c.nodeType, c.data]).toEqual([8, "note"]);
  });
});

describe("Node: appendChild, insertBefore, removeChild, replaceChild", () => {
  test("appending an attached node MOVES it, and both parents' observers hear it", async () => {
    const { rt, doc, shape, vue } = world();
    const root = vue("div");
    const a = vue("section", root);
    const b = vue("aside", root);
    const p = doc.createElement("p");
    a.appendChild(p);
    const seen = watch(rt, root);
    b.appendChild(p);
    await Promise.resolve();
    expect(a.childNodes).toEqual([]);
    expect(b.childNodes).toEqual([p]);
    expect([shape(a), shape(b)]).toEqual([[], ["p"]]);
    expect(seen).toEqual(["section:-p", "aside:+p"]);
    expect(rt.host.tree.errors).toEqual([]);
  });

  test("insertBefore: before a child, null appends, the node itself as reference stays put, a stranger is NotFoundError", () => {
    const { doc, shape, vue } = world();
    const ul = vue("ul");
    const one = doc.createElement("li");
    const three = doc.createElement("li");
    one.textContent = "1";
    three.textContent = "3";
    ul.appendChild(one);
    ul.appendChild(three);
    const two = doc.createElement("b");
    expect(ul.insertBefore(two, three)).toBe(two);
    expect(ul.childNodes).toEqual([one, two, three]);
    expect(shape(ul)).toEqual(["li", "b", "li"]);
    const tail = doc.createElement("i");
    ul.insertBefore(tail, null);
    expect(ul.lastChild).toBe(tail);
    ul.insertBefore(two, two);
    expect(ul.childNodes).toEqual([one, two, three, tail]);
    expect(() => ul.insertBefore(doc.createElement("u"), doc.createElement("s"))).toThrow(
      expect.objectContaining({ name: "NotFoundError" }),
    );
    expect(() => ul.insertBefore(doc.createElement("u"))).toThrow(TypeError);
  });

  test("HierarchyRequestError: a node into its own subtree, and anything into a Text; a non-node is a TypeError", () => {
    const { doc, vue } = world();
    const outer = vue("div");
    const inner = vue("div", outer);
    expect(() => inner.appendChild(outer)).toThrow("The new child element contains the parent.");
    expect(() => outer.appendChild(outer)).toThrow(expect.objectContaining({ name: "HierarchyRequestError" }));
    const t = doc.createTextNode("t");
    expect(() => t.appendChild(doc.createElement("b"))).toThrow(expect.objectContaining({ name: "HierarchyRequestError" }));
    expect(() => outer.appendChild({})).toThrow("parameter 1 is not of type 'Node'");
    expect(() => outer.appendChild("text")).toThrow(TypeError);
  });

  test("removeChild DETACHES: the node keeps its listeners and its subtree, and can go back in", async () => {
    const { rt, doc, shape, hostParent, vue } = world();
    const host = vue("div");
    const card = doc.createElement("div");
    const label = doc.createElement("span");
    card.appendChild(label);
    host.appendChild(card);
    let clicks = 0;
    card.addEventListener("click", () => clicks++);
    expect(host.removeChild(card)).toBe(card);
    expect(card.parentNode).toBeNull();
    expect(host.childNodes).toEqual([]);
    expect(shape(host)).toEqual([]);
    expect(hostParent(card)).toBe("vn-detached");
    expect(rt.host.tree.freed).toEqual([]);
    expect(rt.isLoose ? rt.isLoose(card) : rt.looseRoots.has(card.__vnId)).toBe(true);
    card.click();
    expect(clicks).toBe(1);
    expect(() => host.removeChild(card)).toThrow(expect.objectContaining({ name: "NotFoundError" }));
    host.appendChild(card);
    expect(shape(host)).toEqual(["div"]);
    expect(shape(card)).toEqual(["span"]);
    expect(rt.host.tree.freed.length).toBe(1);
    expect(rt.looseRoots.size).toBe(0);
    expect(rt.nodesById.get(label.__vnId)).toBe(label);
    expect(rt.host.tree.errors).toEqual([]);
  });

  test("replaceChild answers the old child, which is detached, and keeps the position", () => {
    const { doc, shape, vue } = world();
    const row = vue("div");
    const [a, b, c] = ["a", "b", "c"].map((t) => doc.createElement(t === "b" ? "em" : "span"));
    row.append(a, b, c);
    const x = doc.createElement("strong");
    expect(row.replaceChild(x, b)).toBe(b);
    expect(row.childNodes).toEqual([a, x, c]);
    expect(shape(row)).toEqual(["span", "strong", "span"]);
    expect(b.parentNode).toBeNull();
    expect(() => row.replaceChild(doc.createElement("i"), b)).toThrow(expect.objectContaining({ name: "NotFoundError" }));
  });
});

describe("DocumentFragment", () => {
  test("appending a fragment moves its children in order, empties it, and answers the fragment", async () => {
    const { rt, doc, shape, vue } = world();
    const list = vue("ul");
    const frag = doc.createDocumentFragment();
    expect(frag instanceof rt.global.DocumentFragment).toBe(true);
    expect([frag.nodeType, frag.nodeName]).toEqual([11, "#document-fragment"]);
    const items = [1, 2, 3].map((n) => {
      const li = doc.createElement("li");
      li.textContent = String(n);
      frag.appendChild(li);
      return li;
    });
    const seen = watch(rt, list);
    expect(list.appendChild(frag)).toBe(frag);
    await Promise.resolve();
    expect(frag.childNodes).toEqual([]);
    expect(list.childNodes).toEqual(items);
    expect(shape(list)).toEqual(["li", "li", "li"]);
    expect(list.textContent).toBe("123");
    expect(seen).toEqual(["ul:+li+li+li"]);
    expect(rt.host.tree.errors).toEqual([]);
  });
});

describe("ChildNode and ParentNode", () => {
  function row(w: ReturnType<typeof world>, n = 3) {
    const parent = w.vue("div");
    const kids = Array.from({ length: n }, (_, i) => {
      const el = w.doc.createElement("span");
      el.setAttribute("data-k", String.fromCharCode(65 + i));
      parent.appendChild(el);
      return el;
    });
    const names = () => parent.childNodes.map((c: Any) => (c.nodeType === 3 ? `'${c.data}'` : c.getAttribute("data-k")));
    return { parent, kids, names };
  }

  test("append / prepend take nodes and strings", () => {
    const w = world();
    const { parent, kids, names } = row(w, 2);
    parent.append("x", kids[0]);
    expect(names()).toEqual(["B", "'x'", "A"]);
    parent.prepend(kids[0], "y");
    expect(names()).toEqual(["A", "'y'", "B", "'x'"]);
    expect(w.shape(parent)).toEqual(["span", "#text:y", "span", "#text:x"]);
  });

  test("before / after skip the nodes being moved when they pick the reference", () => {
    const w = world();
    const { kids, names } = row(w);
    const [A, B, C] = kids;
    B.before(A);
    expect(names()).toEqual(["A", "B", "C"]);
    C.before(A);
    expect(names()).toEqual(["B", "A", "C"]);
    B.after(C, "t");
    expect(names()).toEqual(["B", "C", "'t'", "A"]);
    A.after(B);
    expect(names()).toEqual(["C", "'t'", "A", "B"]);
  });

  test("replaceWith, remove, replaceChildren", () => {
    const w = world();
    const { parent, kids, names } = row(w);
    const [A, B, C] = kids;
    B.replaceWith("1", "2");
    expect(names()).toEqual(["A", "'1'", "'2'", "C"]);
    expect(B.parentNode).toBeNull();
    C.replaceWith(A, C);
    expect(names()).toEqual(["'1'", "'2'", "A", "C"]);
    A.remove();
    expect(names()).toEqual(["'1'", "'2'", "C"]);
    A.remove(); // detached: nothing
    parent.replaceChildren(B, "z");
    expect(names()).toEqual(["B", "'z'"]);
    expect(w.shape(parent)).toEqual(["span", "#text:z"]);
    expect(w.rt.host.tree.errors).toEqual([]);
  });

  test("an element's own text is kept when a child joins it", () => {
    const { doc, shape, vue } = world();
    const p = vue("p");
    p.textContent = "hello ";
    p.appendChild(doc.createElement("b")).textContent = "world";
    expect(p.textContent).toBe("hello world");
    expect(shape(p)).toEqual(["#text:hello ", "b"]);
  });
});

describe("insertAdjacentElement / insertAdjacentText", () => {
  test("the four positions, null with no parent, and SyntaxError for anything else", () => {
    const { doc, shape, vue } = world();
    const parent = vue("div");
    const mid = doc.createElement("main");
    parent.appendChild(mid);
    const mk = (t: string) => doc.createElement(t);
    expect(mid.insertAdjacentElement("beforebegin", mk("header"))?.tagName).toBe("HEADER");
    mid.insertAdjacentElement("afterEnd", mk("footer"));
    mid.insertAdjacentElement("afterbegin", mk("h1"));
    mid.insertAdjacentElement("BeforeEnd", mk("p"));
    mid.insertAdjacentText("beforeend", "!");
    expect(shape(parent)).toEqual(["header", "main", "footer"]);
    expect(shape(mid)).toEqual(["h1", "p", "#text:!"]);
    expect(mk("x").insertAdjacentElement("beforebegin", mk("y"))).toBeNull();
    expect(() => mid.insertAdjacentElement("inside", mk("y"))).toThrow(expect.objectContaining({ name: "SyntaxError" }));
    expect(() => mid.insertAdjacentElement("beforeend", doc.createTextNode("t"))).toThrow("parameter 2 is not of type 'Element'");
  });
});

describe("what the renderer and script share", () => {
  test("Vue's setElementText DETACHES children it did not make (a script's, a leaving transition's); Vue's later remove frees once", () => {
    const { rt, doc, vue } = world();
    const box = vue("div");
    const leaving = vue("span", box);
    const leavingId = leaving.__vnId;
    const mine = doc.createElement("canvas");
    box.appendChild(mine);
    rt.nodeOps.setElementText(box, "empty");
    expect(box.childNodes).toEqual([]);
    expect(rt.host.tree.freed).toEqual([]);
    const other = vue("section");
    other.appendChild(mine);
    expect(rt.host.tree.shape(other.__vnId)).toEqual(["canvas"]);
    rt.nodeOps.remove(leaving);
    expect(leaving.__vnId).toBe(0);
    expect(rt.host.tree.live.has(leavingId)).toBe(false);
    expect([...rt.host.tree.live].some((id: number) => rt.host.tree.tag.get(id) === "vn-detached")).toBe(false);
    expect(rt.host.tree.errors).toEqual([]);
  });

  test("a node Vue freed has id 0, and inserting it again is an advisory, never a host op on a recycled id", () => {
    const { rt, vue } = world();
    const parent = vue("div");
    const gone = vue("span", parent);
    rt.nodeOps.remove(gone);
    expect(gone.__vnId).toBe(0);
    expect(parent.appendChild(gone)).toBe(gone);
    expect(parent.childNodes).toEqual([]);
    expect(rt.advisories().join("\n")).toContain("already freed");
    expect(rt.host.tree.errors).toEqual([]);
  });

  test("an engine <a> appended to the tree is an advisory, and an engine <style> anywhere joins the stylesheet", async () => {
    const rt: Any = createRuntime({ globals: { __VN_CSS__: [".bundle{}"] } });
    rt.createApp({ render: () => null }).mount();
    const doc = rt.global.document;
    const a = doc.createElement("a");
    expect(doc.body.appendChild(a)).toBe(a);
    expect(rt.advisories().join("\n")).toContain("engine's detached element");
    const s = doc.createElement("style");
    s.textContent = ".late{}";
    doc.body.appendChild(s);
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}\n.late{}");
    doc.body.removeChild(s);
    await Promise.resolve();
    expect(rt.host.sheets.at(-1)).toBe(".bundle{}");
  });

  test("el.id is the id attribute; the host node id is __vnId", () => {
    const { rt, doc } = world();
    const el = doc.createElement("div");
    expect(el.id).toBe("");
    const hostId = el.__vnId;
    el.id = "panel";
    expect(el.getAttribute("id")).toBe("panel");
    expect(el.__vnId).toBe(hostId);
    expect(rt.host.props.at(-1)).toEqual({ id: hostId, key: "id", value: "panel" });
  });
});

describe("loose nodes are freed once the collector has them", () => {
  test("a script-made subtree nothing holds is freed by the sweep, its box included", async () => {
    const { rt, doc, vue } = world();
    const holder = vue("div");
    const made: number[] = [];
    // In a function of its own, so no stack slot of this one keeps them.
    (function build() {
      for (let i = 0; i < 64; i++) {
        const card = doc.createElement("div");
        card.appendChild(doc.createElement("img"));
        made.push(card.__vnId);
        if (i % 2) {
          holder.appendChild(card);
          holder.removeChild(card); // parked in a box
        }
      }
    })();
    let freed = 0;
    for (let round = 0; round < 20 && freed < 32; round++) {
      await new Promise((r) => setTimeout(r, 0));
      Bun.gc(true);
      rt.sweepLoose();
      freed = made.filter((id) => !rt.host.tree.live.has(id)).length;
    }
    // A conservative collector may keep a few; most must go.
    expect(freed).toBeGreaterThanOrEqual(32);
    expect(rt.host.tree.errors).toEqual([]);
    const boxes = [...rt.host.tree.live].filter((id: number) => rt.host.tree.tag.get(id) === "vn-detached");
    expect(boxes.length).toBeLessThanOrEqual(64 - freed);
  });
});
