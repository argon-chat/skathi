import { mark, original, vacant } from "./realm";
import { classTest, elementsUnder, hostQuery, queryOne } from "./dom-members";
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
  cloneMirror,
  domError,
  materializeOwnText,
  nodeArg,
  prepareStylesheetElement,
} from "./tree-ops";
import { looseCreated, nodeById } from "./nodes";
import { nodeOps } from "./node-ops";
import { AttrImpl } from "./attr";
import type { EventHandler, MirrorElement, MirrorNode } from "./types";
import { activeElement } from "./focus";
import { warnCompat } from "../compat";
import { interceptGlobalTarget, isMirrorNode } from "./listeners";
import { installWaapi } from "../waapi";
import { ElementProto } from "./protos";
import { isMirrorEl } from "./controls";
import { frameStampers } from "./unload";
import { defineGlobal } from "./globals";
import { installSelection } from "../selection";
import { asDomRect } from "./computed-style";
import { appRoot, ensureDocumentRoot } from "./root";

const doc = globalThis.document;
if (doc) {
  const defDoc = (name: string, value: unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
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
  // A mirror element, loose until inserted. Head-only `ENGINE_TAGS` stay the engine's: they are not boxes.
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
    if (ns === HTML_NS) return (doc as unknown as { createElement(t: string): unknown }).createElement(local);
    const space = ns === SVG_NS ? "svg" : ns === MATHML_NS ? "mathml" : undefined;
    return looseCreated(nodeOps.createElement(local, space));
  });
  defDoc("createTextNode", (data: unknown) => looseCreated(nodeOps.createText(String(data))));
  defDoc("createComment", (data: unknown) => {
    const c = looseCreated(nodeOps.createComment(String(data)));
    c.text = String(data);
    return c;
  });
  defDoc("importNode", (node: unknown, deep?: unknown) => {
    const n = nodeArg(node, "importNode", "Document", 1);
    if (!n) throw domError("importNode", "Document", "NotSupportedError", "The node provided is not a mirror node.");
    return cloneMirror(n, !!deep);
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
  try {
    Object.defineProperty(doc, "activeElement", { get: activeElement, configurable: true });
  } catch (e) {
    warnCompat("activeElement", `document.activeElement could not be installed on this engine and reads null (${e})`);
  }
  interceptGlobalTarget(doc);
  interceptGlobalTarget(globalThis);
  // `documentElement` and `body` are both the app root (the one node `:root`, `html` and `body` match), so theme code
  // such as `classList.add("dark")` or `style.setProperty("--radius", ...)` reaches the cascade root. After a teardown
  // the next read makes the next app's root.
  for (const name of ["body", "documentElement"]) {
    try {
      Object.defineProperty(doc, name, { get: ensureDocumentRoot, configurable: true });
    } catch {
      /* a non-configurable engine property — leave it alone */
    }
  }
}

// -- View Transitions: `document.startViewTransition`, `activeViewTransition`, `ViewTransition`. Inert without the host.
const attachViewTransitions = (globalThis as { __sk_view_transitions_attach?: (document: object) => void })
  .__sk_view_transitions_attach;
if (doc && typeof attachViewTransitions === "function") attachViewTransitions(doc);

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
  let slot: { fn: unknown; listener: (ev: Event) => void } | null = null;
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
          listener: (ev: Event) => {
            if (typeof entry.fn !== "function") return;
            const r = (entry.fn as (this: unknown, ev: Event) => unknown).call(doc, ev);
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
