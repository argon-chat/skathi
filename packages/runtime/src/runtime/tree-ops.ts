import type { MirrorElement, MirrorNode } from "./types";
import { hooks } from "./state";
import { isMirrorNode } from "./listeners";
import { warnCompat } from "../compat";
import { insertNode, nodeOps } from "./node-ops";
import { detachNode, isLoose, looseCreated } from "./nodes";
import { defineDom, siblingOf } from "./dom-members";
import { NODE_MEMBERS } from "./protos";
import { notifyMutation } from "./mutation-observer";

// Imperative tree mutation runs through `nodeOps.insert`, so the host sees the ops a Vue patch sends; a removal
// DETACHES (see `detachNode`). The engine's own detached `document.createElement(...)` objects (`ENGINE_TAGS`) may be
// passed too: `<style>`/`<link>` join the stylesheet (`foreignNode`), others are left out, advised.

export const ENGINE_TAGS = new Set(["style", "link", "script", "meta", "title", "base"]);
export function stylesheetText(node: unknown): string {
  if (node == null) return "";
  if (typeof node === "string") return node;
  const text = node as { textContent?: unknown; data?: unknown; nodeValue?: unknown; text?: unknown };
  return String(text.textContent ?? text.data ?? text.nodeValue ?? text.text ?? "");
}
/** The engine's detached style element has inert child methods: buffer its text from creation. */
export function prepareStylesheetElement(element: Record<string, unknown>): void {
  let text = stylesheetText(element);
  for (const property of ["textContent", "innerHTML", "innerText", "text"]) {
    Object.defineProperty(element, property, {
      get: () => text,
      set: (value) => {
        text = value == null ? "" : String(value);
      },
      configurable: true,
    });
  }
  const append = (child: unknown) => {
    text += stylesheetText(child);
    return child;
  };
  Object.defineProperty(element, "appendChild", { value: append, configurable: true, writable: true });
  Object.defineProperty(element, "append", {
    value: (...children: unknown[]) => {
      children.forEach(append);
    },
    configurable: true,
    writable: true,
  });
}
/** DOM's "valid element local name" (the 2025 relaxed rule): no whitespace, NUL, `/` or `>`, and a letter, `:`, `_` or non-ASCII first. */
export const VALID_ELEMENT_NAME = /^[A-Za-z:_\u0080-￿][^\t\n\f\r \/>\0]*$/;
export const fragments = new WeakSet<MirrorNode>();

export function domError(method: string, iface: string, name: string, msg: string): Error {
  return new DOMException(`Failed to execute '${method}' on '${iface}': ${msg}`, name);
}

/** A Node argument: a live mirror, or `null` for a node that is not ours (an engine element, or a freed mirror: `foreignNode`). Anything else is WebIDL's TypeError. */
export function nodeArg(x: unknown, method: string, iface: string, pos: number, type = "Node"): MirrorNode | null {
  if (isMirrorNode(x)) {
    if (type === "Element" && x.kind !== 1)
      throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter ${pos} is not of type 'Element'.`);
    return x;
  }
  if (x && typeof x === "object" && (typeof (x as { tagName?: unknown }).tagName === "string" || "__skId" in x))
    return null;
  throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter ${pos} is not of type '${type}'.`);
}

export function foreignNode(x: object, inserted: boolean, method: string): void {
  if (hooks.foreignStyle?.(x, inserted)) return;
  if ("__skId" in x) {
    warnCompat(
      "dom:freed",
      `${method}() was given a node the Vue renderer already freed (its component unmounted), so it is not ` +
        "inserted again — in a browser it would be. Keep a node you move by hand out of the renderer's own subtrees",
    );
    return;
  }
  if (inserted) {
    const tag = String((x as { tagName?: unknown }).tagName).toLowerCase();
    warnCompat(
      `dom:engine-${tag}`,
      `${method}() — a <${tag}> from document.createElement("${tag}") is a head-only element, ` +
        "not a node of the retained tree, so it is not inserted",
    );
  }
}

function preInsertCheck(parent: MirrorNode, node: MirrorNode, method: string): void {
  if (parent.kind !== 1)
    throw domError(method, "Node", "HierarchyRequestError", "This node type does not support this method.");
  for (let a: MirrorNode | null = parent; a; a = a.parentNode) {
    if (a === node)
      throw domError(method, "Node", "HierarchyRequestError", "The new child element contains the parent.");
  }
}

/** An element's own text becomes a real Text child before it gains a sibling, so the text is kept, as in a browser. */
export function materializeOwnText(el: MirrorElement): void {
  if (!el.ownText || el.childNodes.length) return;
  const t = nodeOps.createText(el.ownText);
  el.ownText = "";
  insertNode(t, el, null, false);
  if (typeof __sk_selection_rebase === "function") __sk_selection_rebase(el.__skId, t.__skId);
}

function insertAll(parent: MirrorElement, items: (MirrorNode | object)[], ref: MirrorNode | null): void {
  const added: MirrorNode[] = [];
  for (const it of items) {
    if (!isMirrorNode(it)) {
      foreignNode(it, true, "insert");
      continue;
    }
    for (const n of fragments.has(it) ? it.childNodes.slice() : [it]) {
      if (!added.length) materializeOwnText(parent);
      insertNode(n, parent, ref, false);
      added.push(n);
    }
  }
  if (added.length) notifyMutation(parent, "childList", { addedNodes: added, removedNodes: [] });
}

export function preInsert(parent: MirrorNode, node: unknown, child: unknown, method: string): unknown {
  const n = nodeArg(node, method, "Node", 1);
  const c = child == null ? null : nodeArg(child, method, "Node", 2);
  if (!n) {
    foreignNode(node as object, true, method);
    return node;
  }
  preInsertCheck(parent, n, method);
  if (child != null && (!c || c.parentNode !== parent)) {
    throw domError(
      method,
      "Node",
      "NotFoundError",
      "The node before which the new node is to be inserted is not a child of this node.",
    );
  }
  insertAll(parent as MirrorElement, [n], c === n ? n.nextSibling : c);
  return n;
}

function convertNodes(parent: MirrorNode, args: unknown[], method: string, iface: string): (MirrorNode | object)[] {
  const items = args.map((a) => {
    if (isMirrorNode(a)) return a;
    if (a && typeof a === "object" && (typeof (a as { tagName?: unknown }).tagName === "string" || "__skId" in a))
      return a;
    return looseCreated(nodeOps.createText(String(a)));
  });
  for (const it of items) if (isMirrorNode(it)) preInsertCheckAs(parent, it, method, iface);
  return items;
}
function preInsertCheckAs(parent: MirrorNode, node: MirrorNode, method: string, iface: string): void {
  try {
    preInsertCheck(parent, node, method);
  } catch (e) {
    throw domError(method, iface, (e as { name: string }).name, String((e as Error).message).replace(/^.*?': /, ""));
  }
}

function viable(from: MirrorNode | null, items: unknown[], step: 1 | -1): MirrorNode | null {
  let n = from;
  while (n && items.includes(n)) n = siblingOf(n, step);
  return n;
}

defineDom(
  "appendChild",
  function (this: MirrorNode, node: unknown) {
    if (!isMirrorNode(this)) return node;
    return preInsert(this, node, null, "appendChild");
  },
  NODE_MEMBERS,
);
defineDom(
  "insertBefore",
  function (this: MirrorNode, node: unknown, child: unknown) {
    if (!isMirrorNode(this)) return node;
    if (arguments.length < 2) {
      throw new TypeError("Failed to execute 'insertBefore' on 'Node': 2 arguments required, but only 1 present.");
    }
    return preInsert(this, node, child, "insertBefore");
  },
  NODE_MEMBERS,
);
defineDom(
  "removeChild",
  function (this: MirrorNode, child: unknown) {
    const c = nodeArg(child, "removeChild", "Node", 1);
    if (!c || !isMirrorNode(this)) {
      foreignNode(child as object, false, "removeChild");
      return child;
    }
    if (c.parentNode !== this) {
      throw domError("removeChild", "Node", "NotFoundError", "The node to be removed is not a child of this node.");
    }
    detachNode(c);
    return c;
  },
  NODE_MEMBERS,
);
defineDom(
  "replaceChild",
  function (this: MirrorNode, node: unknown, child: unknown) {
    const n = nodeArg(node, "replaceChild", "Node", 1);
    const c = nodeArg(child, "replaceChild", "Node", 2);
    if (!isMirrorNode(this) || !n || !c) {
      if (!n) foreignNode(node as object, true, "replaceChild");
      if (!c) foreignNode(child as object, false, "replaceChild");
      return child;
    }
    preInsertCheck(this, n, "replaceChild");
    if (c.parentNode !== this) {
      throw domError("replaceChild", "Node", "NotFoundError", "The node to be replaced is not a child of this node.");
    }
    if (n === c) return c;
    let ref = c.nextSibling;
    if (ref === n) ref = n.nextSibling;
    detachNode(c);
    insertAll(this as MirrorElement, [n], ref);
    return c;
  },
  NODE_MEMBERS,
);

defineDom(
  "remove",
  function (this: MirrorNode) {
    if (!isMirrorNode(this)) return void foreignNode(this, false, "remove");
    if (this.parentNode) detachNode(this);
  },
  NODE_MEMBERS,
);
defineDom(
  "before",
  function (this: MirrorNode, ...nodes: unknown[]) {
    const parent = this.parentNode;
    if (!isMirrorNode(this) || !parent) return;
    const prev = viable(siblingOf(this, -1), nodes, -1);
    const items = convertNodes(parent, nodes, "before", "Element");
    insertAll(parent, items, viable(prev ? siblingOf(prev, 1) : (parent.childNodes[0] ?? null), nodes, 1));
  },
  NODE_MEMBERS,
);
defineDom(
  "after",
  function (this: MirrorNode, ...nodes: unknown[]) {
    const parent = this.parentNode;
    if (!isMirrorNode(this) || !parent) return;
    const next = viable(siblingOf(this, 1), nodes, 1);
    insertAll(parent, convertNodes(parent, nodes, "after", "Element"), next);
  },
  NODE_MEMBERS,
);
defineDom(
  "replaceWith",
  function (this: MirrorNode, ...nodes: unknown[]) {
    const parent = this.parentNode;
    if (!isMirrorNode(this) || !parent) return;
    const next = viable(siblingOf(this, 1), nodes, 1);
    const items = convertNodes(parent, nodes, "replaceWith", "Element");
    if (items.includes(this)) return insertAll(parent, items, next);
    insertAll(parent, items, this);
    detachNode(this);
  },
  NODE_MEMBERS,
);

defineDom("append", function (this: MirrorElement, ...nodes: unknown[]) {
  if (!isMirrorNode(this)) return;
  insertAll(this, convertNodes(this, nodes, "append", "Element"), null);
});
defineDom("prepend", function (this: MirrorElement, ...nodes: unknown[]) {
  if (!isMirrorNode(this)) return;
  const items = convertNodes(this, nodes, "prepend", "Element");
  insertAll(this, items, viable(this.childNodes[0] ?? null, nodes, 1));
});
defineDom("replaceChildren", function (this: MirrorElement, ...nodes: unknown[]) {
  if (!isMirrorNode(this)) return;
  const items = convertNodes(this, nodes, "replaceChildren", "Element");
  const gone = this.childNodes.filter((c) => !items.includes(c));
  for (const c of gone) detachNode(c, false);
  if (this.ownText) nodeOps.setElementText(this, "");
  if (gone.length) notifyMutation(this, "childList", { addedNodes: [], removedNodes: gone });
  insertAll(this, items, null);
});

const ADJACENT = ["beforebegin", "afterbegin", "beforeend", "afterend"] as const;
function adjacentSlot(el: MirrorElement, where: unknown, method: string): [MirrorElement, MirrorNode | null] | null {
  const w = String(where).toLowerCase();
  if (!(ADJACENT as readonly string[]).includes(w)) {
    throw domError(
      method,
      "Element",
      "SyntaxError",
      `The value provided ('${String(where)}') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or 'afterEnd'.`,
    );
  }
  if (w === "afterbegin") return [el, el.childNodes[0] ?? null];
  if (w === "beforeend") return [el, null];
  const parent = el.parentNode;
  if (!parent) return null;
  return [parent, w === "beforebegin" ? el : siblingOf(el, 1)];
}
defineDom("insertAdjacentElement", function (this: MirrorElement, where: unknown, element: unknown) {
  const n = nodeArg(element, "insertAdjacentElement", "Element", 2, "Element");
  if (!isMirrorNode(this)) return element;
  const slot = adjacentSlot(this, where, "insertAdjacentElement");
  if (!slot) return null;
  if (!n) {
    foreignNode(element as object, true, "insertAdjacentElement");
    return element;
  }
  preInsertCheckAs(slot[0], n, "insertAdjacentElement", "Element");
  insertAll(slot[0], [n], slot[1] === n ? n.nextSibling : slot[1]);
  return n;
});
defineDom("insertAdjacentText", function (this: MirrorElement, where: unknown, data: unknown) {
  if (!isMirrorNode(this)) return;
  const slot = adjacentSlot(this, where, "insertAdjacentText");
  if (slot) insertAll(slot[0], [looseCreated(nodeOps.createText(String(data)))], slot[1]);
});

/** A loose copy of `node`: tag, namespace, attributes (class and inline style among them) and, when `deep`, its subtree. Listeners and scope ids are not copied, as in the DOM. */
export function cloneMirror(node: MirrorNode, deep: boolean): MirrorNode {
  const copy = copyOf(node, deep);
  return isLoose(copy) ? copy : looseCreated(copy);
}
function copyOf(node: MirrorNode, deep: boolean): MirrorNode {
  if (node.kind !== 1) {
    const text = node.text ?? "";
    if (node.kind === 2) return nodeOps.createText(text);
    const comment = nodeOps.createComment(text);
    comment.text = text;
    return comment;
  }
  const copy = fragments.has(node)
    ? (new (globalThis as unknown as { DocumentFragment: new () => MirrorElement }).DocumentFragment() as MirrorElement)
    : nodeOps.createElement(node.tag, node._ns);
  for (const name of node.getAttributeNames()) {
    const value = node.getAttribute(name);
    if (value !== null) copy.setAttribute(name, value);
  }
  if (!deep) return copy;
  if (node.ownText) nodeOps.setElementText(copy, node.ownText);
  for (const child of node.childNodes) insertNode(copyOf(child, true), copy, null, false);
  return copy;
}

defineDom(
  "cloneNode",
  function (this: MirrorNode, deep?: unknown) {
    if (isMirrorNode(this)) return cloneMirror(this, !!deep);
    warnCompat(
      "cloneNode",
      "cloneNode() on an element the engine created detached (a <style>, <link>, <a> or <input> from " +
        "document.createElement) is not implemented; it answers null",
    );
    return null;
  },
  NODE_MEMBERS,
);
