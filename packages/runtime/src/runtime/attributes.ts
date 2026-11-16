import type { MirrorClassList, MirrorElement, MirrorNode } from "./types";
import { defineDom, defineDomGet } from "./dom-members";
import { NODE_MEMBERS } from "./protos";
import { hyphenate } from "./inline-style";
import { branded } from "./collections";
import { setClassText } from "./patch-prop";
import { canonicalEvent } from "./event-names";
import { isMirrorEl, ownExpando } from "./controls";
import { propagate } from "./events";
import { domError } from "./tree-ops";
import { activate, activateWindowControl, isActivationEvent } from "./links";
import { AttrImpl, attrParts } from "./attr";

function attrNameNS(el: MirrorElement, ns: unknown, local: unknown): string | null {
  const want = ns == null || ns === "" ? null : String(ns);
  const l = String(local);
  for (const name of Object.keys(el.attrs ?? {})) {
    const p = attrParts(name);
    if (p.ns === want && p.local === l) return name;
  }
  return null;
}

const attrNodes = new WeakMap<MirrorElement, Map<string, AttrImpl>>();
export let noAttrNodes = true;

function attrNode(el: MirrorElement, name: string): AttrImpl | null {
  if (!el.attrs || !(name in el.attrs)) return null;
  let map = attrNodes.get(el);
  if (!map) attrNodes.set(el, (map = new Map()));
  let a = map.get(name);
  if (!a) {
    noAttrNodes = false;
    a = new AttrImpl(name, "");
    a._el = el;
    map.set(name, a);
  }
  return a;
}

export function attrChanged(el: MirrorElement, name: string, oldValue: string | null): void {
  if (noAttrNodes || (el.attrs && name in el.attrs)) return;
  const map = attrNodes.get(el);
  const a = map?.get(name);
  if (!a) return;
  if (oldValue !== null) a._value = oldValue;
  a._el = null;
  map!.delete(name);
}

function setAttrNode(el: MirrorElement, attr: unknown, method: string): AttrImpl | null {
  if (!(attr instanceof AttrImpl)) {
    throw new TypeError(`Failed to execute '${method}' on 'Element': parameter 1 is not of type 'Attr'.`);
  }
  if (attr._el && attr._el !== el) {
    throw domError(
      method,
      "Element",
      "InUseAttributeError",
      "The node provided is an attribute node that is already an attribute of another Element; attribute nodes must be explicitly cloned.",
    );
  }
  if (attr._el === el) return attr;
  const old = attrNode(el, attr._name);
  if (old) old._value = old.value;
  el.setAttribute(attr._name, attr._value);
  if (old) old._el = null;
  noAttrNodes = false;
  let map = attrNodes.get(el);
  if (!map) attrNodes.set(el, (map = new Map()));
  attr._el = el;
  map.set(attr._name, attr);
  return old;
}

function removeAttrNode(el: MirrorElement, attr: unknown, method: string, iface: string): AttrImpl {
  if (!(attr instanceof AttrImpl) || attr._el !== el) {
    throw domError(method, iface, "NotFoundError", "The node provided is owned by another element.");
  }
  attr._value = attr.value;
  el.removeAttribute(attr._name);
  return attr;
}

const namedNodeMaps = new WeakMap<MirrorElement, object>();

export class NamedNodeMapImpl {
  declare _el: MirrorElement;
  get length(): number {
    return Object.keys(this._el.attrs ?? {}).length;
  }
  item(i: unknown): AttrImpl | null {
    const n = Number(i) >>> 0;
    const name = Object.keys(this._el.attrs ?? {})[n];
    return name === undefined ? null : attrNode(this._el, name);
  }
  getNamedItem(name: unknown): AttrImpl | null {
    return attrNode(this._el, String(name));
  }
  getNamedItemNS(ns: unknown, local: unknown): AttrImpl | null {
    const name = attrNameNS(this._el, ns, local);
    return name === null ? null : attrNode(this._el, name);
  }
  setNamedItem(attr: unknown): AttrImpl | null {
    return setAttrNode(this._el, attr, "setNamedItem");
  }
  setNamedItemNS(attr: unknown): AttrImpl | null {
    return setAttrNode(this._el, attr, "setNamedItemNS");
  }
  removeNamedItem(name: unknown): AttrImpl {
    const a = attrNode(this._el, String(name));
    if (!a)
      throw domError(
        "removeNamedItem",
        "NamedNodeMap",
        "NotFoundError",
        `No item with name '${String(name)}' was found.`,
      );
    return removeAttrNode(this._el, a, "removeNamedItem", "NamedNodeMap");
  }
  removeNamedItemNS(ns: unknown, local: unknown): AttrImpl {
    const name = attrNameNS(this._el, ns, local);
    if (name === null) {
      throw domError(
        "removeNamedItemNS",
        "NamedNodeMap",
        "NotFoundError",
        `No item with name '${String(ns)}::${String(local)}' was found.`,
      );
    }
    return removeAttrNode(this._el, attrNode(this._el, name), "removeNamedItemNS", "NamedNodeMap");
  }
  *[Symbol.iterator](): IterableIterator<AttrImpl> {
    for (const name of Object.keys(this._el.attrs ?? {})) {
      const a = attrNode(this._el, name);
      if (a) yield a;
    }
  }
}
Object.defineProperty(NamedNodeMapImpl, "name", { value: "NamedNodeMap" });

function namedNodeMapOf(el: MirrorElement): object {
  let m = namedNodeMaps.get(el);
  if (m) return m;
  const target = Object.create(NamedNodeMapImpl.prototype) as NamedNodeMapImpl;
  Object.defineProperty(target, "_el", { value: el });
  const index = (k: PropertyKey): number => (typeof k === "string" && /^(0|[1-9]\d*)$/.test(k) ? Number(k) : -1);
  const named = (k: PropertyKey): boolean => typeof k === "string" && !(k in target) && !!el.attrs && k in el.attrs;
  m = new Proxy(target, {
    get(t, k, receiver) {
      const i = index(k);
      if (i >= 0) return t.item(i) ?? undefined;
      if (named(k)) return attrNode(el, k as string);
      return Reflect.get(t, k, receiver);
    },
    has(t, k) {
      const i = index(k);
      if (i >= 0) return i < t.length;
      return named(k) || Reflect.has(t, k);
    },
    ownKeys(t) {
      return [...Object.keys(el.attrs ?? {}).map((_, i) => String(i)), ...Reflect.ownKeys(t)];
    },
    getOwnPropertyDescriptor(t, k) {
      const i = index(k);
      if (i >= 0)
        return i < t.length ? { value: t.item(i), enumerable: true, configurable: true, writable: false } : undefined;
      return Reflect.getOwnPropertyDescriptor(t, k);
    },
    set(t, k, v, receiver) {
      if (index(k) >= 0 || named(k)) return true;
      return Reflect.set(t, k, v, receiver);
    },
  });
  namedNodeMaps.set(el, m);
  return m;
}

defineDomGet("attributes", function (this: MirrorElement) {
  return isMirrorEl(this) ? namedNodeMapOf(this) : undefined;
});
defineDom("hasAttributes", function (this: MirrorElement) {
  return !!this.attrs && Object.keys(this.attrs).length > 0;
});
defineDom("getAttributeNode", function (this: MirrorElement, name: unknown) {
  return attrNode(this, String(name));
});
defineDom("getAttributeNodeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  const name = attrNameNS(this, ns, local);
  return name === null ? null : attrNode(this, name);
});
defineDom("setAttributeNode", function (this: MirrorElement, attr: unknown) {
  return setAttrNode(this, attr, "setAttributeNode");
});
defineDom("setAttributeNodeNS", function (this: MirrorElement, attr: unknown) {
  return setAttrNode(this, attr, "setAttributeNodeNS");
});
defineDom("removeAttributeNode", function (this: MirrorElement, attr: unknown) {
  return removeAttrNode(this, attr, "removeAttributeNode", "Element");
});
defineDom("getAttributeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  const name = attrNameNS(this, ns, local);
  return name === null ? null : this.getAttribute(name);
});
defineDom("hasAttributeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  return attrNameNS(this, ns, local) !== null;
});
// The qualified name is the key, so `setAttributeNS(XLINK_NS, "xlink:href", v)` and `getAttribute("xlink:href")` meet.
defineDom("setAttributeNS", function (this: MirrorElement, ns: unknown, qualifiedName: unknown, value: unknown) {
  const q = String(qualifiedName);
  const existing = attrNameNS(this, ns, q.slice(q.indexOf(":") + 1));
  this.setAttribute(existing ?? q, value);
});
defineDom("removeAttributeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  const name = attrNameNS(this, ns, local);
  if (name !== null) this.removeAttribute(name);
});
const camelToData = (k: string): string => "data-" + hyphenate(k);
defineDomGet("dataset", function (this: MirrorElement) {
  const node = this;
  const proxy = new Proxy(Object.create(null) as Record<string, string | undefined>, {
    get: (_t, k) => (typeof k === "string" ? node.attrs?.[camelToData(k)] : undefined),
    has: (_t, k) => typeof k === "string" && camelToData(k) in (node.attrs ?? {}),
    set: (_t, k, v) => {
      node.setAttribute(camelToData(k as string), v);
      return true;
    },
    deleteProperty: (_t, k) => {
      node.removeAttribute(camelToData(k as string));
      return true;
    },
    ownKeys: () =>
      Object.keys(node.attrs ?? {})
        .filter((k) => k.startsWith("data-"))
        .map((k) => k.slice(5).replace(/-([a-z])/g, (_, c: string) => c.toUpperCase())),
    getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
  });
  Object.defineProperty(this, "dataset", { value: branded("DOMStringMap", proxy), configurable: true });
  return proxy;
});

defineDomGet(
  "className",
  function (this: MirrorElement) {
    return isMirrorEl(this) ? (this.attrs.class ?? "") : undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "className", v);
    setClassText(this, String(v));
  } as (value: never) => void,
);
defineDomGet("classList", function (this: MirrorElement) {
  const node = this;
  const parts = () => (node.attrs.class ?? "").split(/\s+/).filter(Boolean);
  const commit = (list: string[]) => setClassText(node, list.join(" "));
  const list: MirrorClassList = {
    add(...names: string[]) {
      const cur = parts();
      for (const n of names) if (n && !cur.includes(n)) cur.push(n);
      commit(cur);
    },
    remove(...names: string[]) {
      commit(parts().filter((c) => !names.includes(c)));
    },
    toggle(name: string, force?: boolean) {
      const on = force === undefined ? !parts().includes(name) : !!force;
      if (on) list.add(name);
      else list.remove(name);
      return on;
    },
    contains: (name: string) => parts().includes(name),
    get length() {
      return parts().length;
    },
    item: (i: number) => parts()[i] ?? null,
    toString: () => node.className,
    [Symbol.iterator]: () => parts()[Symbol.iterator](),
  };
  Object.defineProperty(this, "classList", { value: branded("DOMTokenList", list), configurable: true });
  return list;
});

defineDomGet("style", function (this: MirrorElement) {
  const style = __sk_style_object(this);
  Object.defineProperty(this, "style", { value: style, configurable: true });
  return style;
});

// dispatchEvent runs a component's own events through the capture/bubble walk of host events (reka-ui's `select.select`, `menu.itemSelect`).
defineDom(
  "dispatchEvent",
  function (this: MirrorNode, ev: Event) {
    if (!ev || typeof ev.type !== "string") return true;
    const name = canonicalEvent(ev.type);
    const proceed = propagate(this, name, ev);
    // DOM activation step: an uncancelled dispatched click acts like a real one (FileSaver.js; `el.click()` is built on it).
    if (proceed && name === "click" && isActivationEvent(ev)) {
      activateWindowControl(this);
      activate(this, !!ev.bubbles);
    }
    return proceed;
  },
  NODE_MEMBERS,
);
