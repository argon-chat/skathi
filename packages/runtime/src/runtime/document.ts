import { mark, original, vacant } from "./realm";
import { classTest, elementsUnder, hostQuery, queryOne, viewportRect } from "./dom-members";
import type { TreeWalkerFilter } from "./dom-members";
import {
  HTMLCollectionProto,
  HTML_NS,
  MATHML_NS,
  NodeListProto,
  SVG_NS,
  branded,
  liveCollection,
  staticList,
} from "./collections";
import {
  ENGINE_TAGS,
  VALID_ELEMENT_NAME,
  domError,
  foreignNode,
  materializeOwnText,
  nodeArg,
  preInsert,
  prepareStylesheetElement,
} from "./tree-ops";
import { wrapDetached } from "./labels";
import type { Detached } from "./labels";
import { detachNode, looseCreated, nodeById } from "./nodes";
import { nodeOps } from "./node-ops";
import { AttrImpl } from "./attr";
import type { EventHandler, MirrorElement, MirrorNode } from "./types";
import { activeElement } from "./focus";
import { warnCompat } from "../compat";
import { interceptGlobalTarget, isMirrorNode } from "./listeners";
import { cssPropName, cssomName } from "./inline-style";
import { installFonts } from "../fonts";
import { installWaapi } from "../waapi";
import { ElementProto } from "./protos";
import { isMirrorEl } from "./controls";
import { frameStampers } from "./unload";
import { defineGlobal } from "./globals";
import { installSelection } from "../selection";
import { asDomRect } from "./computed-style";
import { appRoot, ensureDocumentRoot } from "../runtime";
import { hooks } from "./state";

const doc = globalThis.document;
if (doc) {
  const defDoc = (name: string, value: unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
  };
  const defDocGet = (name: string, get: () => unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { get: mark(get), configurable: true });
  };
  // Force-installed: redjs's own `querySelector`/`getElementById` search a registry of elements it created, which is empty here,
  // so leaving them finds nothing, silently.
  const forceDoc = (name: string, value: unknown) =>
    Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  const rootId = () => (appRoot ? appRoot.__skId : 0);
  forceDoc("querySelector", (sel: string) => queryOne(sel, rootId(), 3));
  forceDoc("querySelectorAll", (sel: string) => staticList(NodeListProto, hostQuery(sel, rootId(), 3)));
  defDoc("getElementsByClassName", (classNames: unknown) => {
    const test = classTest(classNames);
    return liveCollection(HTMLCollectionProto, true, () => elementsUnder(appRoot, true, test));
  });
  defDoc("getElementsByName", (name: unknown) => {
    const want = String(name);
    return liveCollection(NodeListProto, false, () =>
      elementsUnder(appRoot, true, (el) => !el._ns && el.attrs.name === want),
    );
  });
  forceDoc("getElementById", (id: unknown) => queryOne(`[id=${JSON.stringify(String(id))}]`, rootId()));
  // A mirror element, loose until inserted. `ENGINE_TAGS` stay the engine's detached elements: `<a>`/`<input>` carry download and pick
  // idioms (see `wrapDetached`), and head-only tags are not boxes.
  // The engine's, not a previous runtime's wrapper (`original`): a reload would stack wrappers.
  const engineCreate = original(doc, "createElement", (doc as { createElement?: unknown }).createElement);
  const nativeCreate =
    typeof engineCreate === "function" ? (engineCreate as (tag: unknown, opts?: unknown) => unknown).bind(doc) : null;
  forceDoc("createElement", (tag: unknown, opts?: unknown) => {
    const name = String(tag);
    if (!VALID_ELEMENT_NAME.test(name)) {
      throw domError(
        "createElement",
        "Document",
        "InvalidCharacterError",
        `The tag name provided ('${name}') is not a valid name.`,
      );
    }
    const t = name.toLowerCase();
    if (nativeCreate && ENGINE_TAGS.has(t)) {
      const el = nativeCreate(tag, opts);
      if ((t === "a" || t === "input") && el && typeof el === "object") wrapDetached(el as Detached, t);
      if (t === "style" && el && typeof el === "object") prepareStylesheetElement(el as Record<string, unknown>);
      return el;
    }
    return looseCreated(nodeOps.createElement(t));
  });
  // The SVG namespace keeps the name's case (`linearGradient`), as the renderer's SVG elements do.
  defDoc("createElementNS", (ns: unknown, qualifiedName: unknown) => {
    const name = String(qualifiedName);
    const local = name.slice(name.indexOf(":") + 1);
    if (!VALID_ELEMENT_NAME.test(local)) {
      throw domError(
        "createElementNS",
        "Document",
        "InvalidCharacterError",
        `The qualified name provided ('${name}') contains the invalid name-start character '${local[0] ?? ""}'.`,
      );
    }
    if (ns === HTML_NS) return (doc.createElement as (t: string) => unknown)(local);
    const space = ns === SVG_NS ? "svg" : ns === MATHML_NS ? "mathml" : undefined;
    return looseCreated(nodeOps.createElement(local, space));
  });
  defDoc("createTextNode", (data: unknown) => looseCreated(nodeOps.createText(String(data))));
  defDoc("createComment", (data: unknown) => {
    const c = looseCreated(nodeOps.createComment(String(data)));
    c.text = String(data);
    return c;
  });
  defDoc("createAttribute", (name: unknown) => {
    const n = String(name);
    if (!VALID_ELEMENT_NAME.test(n)) {
      throw domError(
        "createAttribute",
        "Document",
        "InvalidCharacterError",
        `The localName provided ('${n}') contains an invalid character.`,
      );
    }
    return new AttrImpl(n, "");
  });
  defDoc(
    "createDocumentFragment",
    () => new (globalThis as unknown as { DocumentFragment: new () => MirrorElement }).DocumentFragment(),
  );
  defDoc("createTreeWalker", (root: MirrorNode | null, whatToShow = 0xffffffff, filter: TreeWalkerFilter = null) => {
    const flat: MirrorNode[] = [];
    const walk = (n: MirrorNode) => {
      for (const c of n.childNodes || []) {
        const show = c.kind === 1 ? 1 : c.kind === 2 ? 4 : 128;
        if (whatToShow & show) flat.push(c);
        walk(c);
      }
    };
    if (root) walk(root);
    let i = -1;
    const accepts = (n: MirrorNode) => {
      if (!filter) return true;
      const fn = typeof filter === "function" ? filter : filter.acceptNode;
      return !fn || fn.call(filter, n) === 1;
    };
    return branded("TreeWalker", {
      root,
      currentNode: root,
      nextNode() {
        while (++i < flat.length) {
          if (accepts(flat[i]!)) return (this.currentNode = flat[i]!);
        }
        return null;
      },
      previousNode() {
        while (--i >= 0) {
          if (accepts(flat[i]!)) return (this.currentNode = flat[i]!);
        }
        return null;
      },
    });
  });
  defDocGet("defaultView", () => globalThis);
  try {
    Object.defineProperty(doc, "activeElement", { get: activeElement, configurable: true });
  } catch (e) {
    warnCompat("activeElement", `document.activeElement could not be installed on this engine and reads null (${e})`);
  }
  interceptGlobalTarget(doc);
  interceptGlobalTarget(globalThis);
  // Viewport metrics: redjs reports 0; the window IS the app root, so use its painted box (floating positioning reads them).
  // Except with the windowing host present: then inner* (client area) and outer* (GetWindowRect) are real and deliberately
  // differ, so they are left alone.
  if (!globalThis.__SK_WINDOW__) {
    for (const [name, axis] of [
      ["innerWidth", "width"],
      ["innerHeight", "height"],
      ["outerWidth", "width"],
      ["outerHeight", "height"],
    ] as [string, "width" | "height"][]) {
      try {
        Object.defineProperty(globalThis, name, {
          get: () => viewportRect()[axis],
          configurable: true,
        });
      } catch {
        /* a non-configurable engine property — leave it alone */
      }
    }
  }
  for (const name of ["scrollX", "scrollY", "pageXOffset", "pageYOffset"]) {
    try {
      if (typeof (globalThis as Record<string, unknown>)[name] !== "number") {
        Object.defineProperty(globalThis, name, { value: 0, configurable: true, writable: true });
      }
    } catch {
      /* ditto */
    }
  }
  // `documentElement` and `body` are both the app root (the one node `:root`, `html` and `body` match), so theme code such as `classList.add("dark")` or `style.setProperty("--radius", ...)` reaches the cascade root.
  // Before `mount()` the root does not exist but module-scope theme code already writes to it; the pre-root object buffers those writes.
  {
    const pending: {
      classes: string[];
      style: Record<string, string>;
      attrs: Record<string, string>;
    } = { classes: [], style: Object.create(null), attrs: Object.create(null) };
    const preRoot = {
      nodeType: 1,
      tagName: "HTML",
      get className() {
        return pending.classes.join(" ");
      },
      set className(v: unknown) {
        pending.classes = String(v ?? "")
          .split(/\s+/)
          .filter(Boolean);
      },
      classList: {
        add(...names: string[]) {
          for (const n of names) if (n && !pending.classes.includes(n)) pending.classes.push(n);
        },
        remove(...names: string[]) {
          pending.classes = pending.classes.filter((c) => !names.includes(c));
        },
        toggle(name: string, force?: boolean) {
          const on = force === undefined ? !pending.classes.includes(name) : !!force;
          if (on) preRoot.classList.add(name);
          else preRoot.classList.remove(name);
          return on;
        },
        contains: (name: string) => pending.classes.includes(name),
        get length() {
          return pending.classes.length;
        },
        item: (i: number) => pending.classes[i] ?? null,
        toString: () => pending.classes.join(" "),
      },
      style: new Proxy(
        {
          setProperty(k: unknown, v: unknown) {
            pending.style[cssomName(String(k))] = v == null ? "" : String(v);
          },
          removeProperty(k: unknown) {
            const prev = pending.style[cssomName(String(k))] ?? "";
            delete pending.style[cssomName(String(k))];
            return prev;
          },
          getPropertyValue: (k: unknown) => pending.style[cssomName(String(k))] ?? "",
          getPropertyPriority: () => "",
        },
        {
          set(t, k, v) {
            if (typeof k === "symbol" || k in t) return Reflect.set(t, k, v);
            pending.style[cssPropName(k)] = v == null ? "" : String(v);
            return true;
          },
          get: (t, k) =>
            typeof k === "symbol" || k in t ? t[k as keyof typeof t] : (pending.style[cssPropName(k)] ?? ""),
        },
      ),
      getAttribute: (k: unknown) => pending.attrs[String(k)] ?? null,
      setAttribute(k: unknown, v: unknown) {
        pending.attrs[String(k)] = String(v);
      },
      hasAttribute: (k: unknown) => String(k) in pending.attrs,
      removeAttribute(k: unknown) {
        delete pending.attrs[String(k)];
      },
      querySelector: () => null,
      querySelectorAll: () => [],
      contains: () => false,
      appendChild: (n: unknown) => preInsert(ensureDocumentRoot(), n, null, "appendChild"),
      insertBefore: (n: unknown, anchor: unknown) => preInsert(ensureDocumentRoot(), n, anchor, "insertBefore"),
      removeChild(n: unknown) {
        const root = ensureDocumentRoot();
        const child = nodeArg(n, "removeChild", "Node", 1);
        if (!child) {
          foreignNode(n as object, false, "removeChild");
          return n;
        }
        if (child.parentNode !== root)
          throw domError("removeChild", "Node", "NotFoundError", "The node to be removed is not a child of this node.");
        detachNode(child);
        return child;
      },
      insertAdjacentElement: (where: unknown, n: unknown) =>
        ensureDocumentRoot().insertAdjacentElement(String(where), n as MirrorElement),
      addEventListener: () => {},
      removeEventListener: () => {},
      getBoundingClientRect: () => ({
        ...viewportRect(),
        toJSON() {
          return this;
        },
      }),
      children: [],
      ownerDocument: doc,
      parentNode: null,
      scrollLeft: 0,
      scrollTop: 0,
      scrollX: 0,
      scrollY: 0,
      clientTop: 0,
      clientLeft: 0,
    };
    for (const [name, axis] of [
      ["clientWidth", "width"],
      ["clientHeight", "height"],
      ["offsetWidth", "width"],
      ["offsetHeight", "height"],
      ["scrollWidth", "width"],
      ["scrollHeight", "height"],
    ] as [string, "width" | "height"][]) {
      Object.defineProperty(preRoot, name, { get: () => viewportRect()[axis], configurable: true });
    }
    hooks.flushDocumentElement = (root) => {
      for (const c of pending.classes) root.classList.add(c);
      for (const k of Object.keys(pending.style)) root.style.setProperty(k, pending.style[k]);
      for (const k of Object.keys(pending.attrs)) root.setAttribute(k, pending.attrs[k]);
    };
    for (const name of ["body", "documentElement"]) {
      try {
        Object.defineProperty(doc, name, {
          get: () => appRoot ?? preRoot,
          configurable: true,
        });
      } catch {
        /* a non-configurable engine property — leave it alone */
      }
    }
    // `<head>` is not a box (a `<style>` appended must not become a child of the root): it stays redjs's property bag, common members filled in.
    const head = doc.head as unknown as Record<string, unknown> | null | undefined;
    if (head && typeof head === "object") {
      if (!head.style || typeof head.style !== "object") head.style = {};
      const hs = head.style as Record<string, unknown>;
      if (typeof hs.setProperty !== "function") {
        hs.setProperty = (k: unknown, v: unknown) => {
          hs[String(k)] = v == null ? "" : String(v);
        };
      }
      if (typeof hs.removeProperty !== "function") {
        hs.removeProperty = (k: unknown) => {
          const prev = hs[String(k)] ?? "";
          delete hs[String(k)];
          return prev;
        };
      }
      if (typeof hs.getPropertyValue !== "function") hs.getPropertyValue = (k: unknown) => hs[String(k)] ?? "";
      if (!head.querySelector) head.querySelector = () => null;
      if (!head.querySelectorAll) head.querySelectorAll = () => [];
      if (!head.contains) head.contains = () => false;
      if (!head.getAttribute) head.getAttribute = () => null;
      if (!head.setAttribute) head.setAttribute = () => {};
      if (!head.appendChild) head.appendChild = (n: unknown) => n;
      if (!head.removeChild) head.removeChild = (n: unknown) => n;
      if (!("ownerDocument" in head)) {
        Object.defineProperty(head, "ownerDocument", { get: () => doc, configurable: true });
      }
    }
  }
}

// -- web fonts: `FontFace`, `FontFaceSet`, `document.fonts`. Inert without the host's loader; a reload takes over the host's listener.
installFonts(globalThis as unknown as Record<string, unknown>, doc);

// -- Web Animations: `el.animate()` on HTMLElement/SVGElement prototypes only, not Element.prototype, so motion / framer-motion keep
//    their JS animations. Inert without the host's native WAAPI core.
if (typeof __sk_waapi_core === "object" && __sk_waapi_core !== null) {
  const g = globalThis as unknown as Record<string, unknown>;
  const svg = (g.SVGElement as { prototype?: object } | undefined)?.prototype;
  installWaapi({
    host: __sk_waapi_core,
    core: __sk_waapi_core,
    protos: svg && svg !== ElementProto ? [ElementProto, svg] : [ElementProto],
    nodeOf: (t) => (isMirrorEl(t) ? t.__skId : 0),
    setStyle: (t, name, value) => (t as MirrorElement).style.setProperty(name, value),
    contains: (root, node) =>
      isMirrorNode(root) && isMirrorNode(node) && (root as MirrorElement).contains(node as MirrorNode),
    warn: warnCompat,
    now: () => (g.performance as { now(): number } | undefined)?.now() ?? Date.now(),
    frame: (cb) => {
      if (typeof requestAnimationFrame === "function") requestAnimationFrame(cb);
      else setTimeout(() => cb((g.performance as { now(): number } | undefined)?.now() ?? Date.now()), 16);
    },
    onFrame: (stamp) => void frameStampers.push(stamp),
    setTimer: (cb, ms) => setTimeout(cb, ms),
    clearTimer: (h) => clearTimeout(h as number),
    defineGlobal,
    doc: doc as unknown as Record<string, unknown> | null,
    EventTarget: g.EventTarget as (new () => object) | undefined,
    Event: g.Event as (new (type: string, init?: object) => object) | undefined,
    DOMException: g.DOMException as (new (message: string, name: string) => Error) | undefined,
  });
}

// -- text selection: `getSelection()`, `Selection`, `Range`, `document.createRange()` over the host's selection. Inert without it.
installSelection({
  host:
    typeof __sk_selection_get === "function"
      ? { get: __sk_selection_get, set: __sk_selection_set, text: __sk_selection_text }
      : null,
  nodeById: (id) => nodeById(id),
  isNode: (x): x is MirrorNode => isMirrorNode(x),
  root: () => appRoot,
  materialize: (el) => {
    materializeOwnText(el as MirrorElement);
    return el.childNodes[0] ?? null;
  },
  rect: (el) => (el as MirrorElement).getBoundingClientRect(),
  makeRect: (x, y, w, h) => asDomRect({ x, y, width: w, height: h, top: y, left: x, right: x + w, bottom: y + h }),
  createText: (data) => looseCreated(nodeOps.createText(data)),
  createFragment: () => new (globalThis as unknown as { DocumentFragment: new () => MirrorElement }).DocumentFragment(),
  warn: warnCompat,
  error: domError,
  defineGlobal,
  defineDoc: (name, value) => {
    if (doc && vacant(doc, name))
      Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
  },
});
for (const type of ["selectionchange", "selectstart"]) {
  const prop = `on${type}`;
  if (!doc || !vacant(doc, prop)) continue;
  let slot: { fn: unknown; listener: (ev: SkEvent) => void } | null = null;
  Object.defineProperty(doc, prop, {
    get: mark(() => slot?.fn ?? null),
    set: mark((v: unknown) => {
      const fn = v !== null && (typeof v === "function" || typeof v === "object") ? v : null;
      if (slot && !fn) {
        doc.removeEventListener?.(type, slot.listener as EventHandler);
        slot = null;
      } else if (slot) slot.fn = fn;
      else if (fn) {
        const entry = {
          fn,
          listener: (ev: SkEvent) => {
            if (typeof entry.fn !== "function") return;
            const r = (entry.fn as (this: unknown, ev: SkEvent) => unknown).call(doc, ev);
            if (r === false) ev.preventDefault?.();
          },
        };
        slot = entry;
        doc.addEventListener?.(type, entry.listener as EventHandler);
      }
    }),
    configurable: true,
  });
}

// document.elementFromPoint / elementsFromPoint: the host's hit test (skips `pointer-events: none` and `visibility:
// hidden`), viewport coordinates, null / [] outside the viewport; a non-finite coordinate throws (WebIDL `double`).
if (doc && typeof __sk_hit_at === "function") {
  const hitAt = (name: string, x: unknown, y: unknown, all: boolean): MirrorElement[] => {
    const px = Number(x);
    const py = Number(y);
    if (!Number.isFinite(px) || !Number.isFinite(py)) {
      throw new TypeError(`Failed to execute '${name}' on 'Document': The provided double value is non-finite.`);
    }
    const ids = __sk_hit_at(px, py, all ? 1 : 0);
    if (!ids) return [];
    const out: MirrorElement[] = [];
    for (const part of ids.split(",")) {
      const n = nodeById(Number(part) as SkNodeId);
      if (n && n.kind === 1) out.push(n as MirrorElement);
    }
    return out;
  };
  for (const [name, fn] of [
    ["elementFromPoint", (x: unknown, y: unknown) => hitAt("elementFromPoint", x, y, false)[0] ?? null],
    ["elementsFromPoint", (x: unknown, y: unknown) => hitAt("elementsFromPoint", x, y, true)],
  ] as const) {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(fn), configurable: true, writable: true });
  }
}
