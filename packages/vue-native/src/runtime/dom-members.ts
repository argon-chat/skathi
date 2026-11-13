import { nodeById } from "./nodes";
import { ElementProto, NODE_MEMBERS, TextProto } from "./protos";
import type { MirrorCollection, MirrorElement, MirrorNode, MirrorRect, MirrorText } from "./types";
import { warnCompat } from "../compat";
import { nodeOps, windowDocument, windowRootOf } from "./node-ops";
import {
  DOMRectListProto,
  HTMLCollectionProto,
  HTML_NS,
  MATHML_NS,
  NodeListProto,
  SVG_NS,
  liveCollection,
  staticList,
} from "./collections";
import { isMirrorNode } from "./listeners";
import { appRoot } from "../runtime";
import { asDomRect } from "./computed-style";
import { ownExpando } from "./controls";
import { hostScroll } from "./scroll";
import type { ScrollPos } from "./scroll";

// The DOM surface on mirror nodes: reused browser components call element methods unconditionally, so a missing one
// is a TypeError. Members answer truthfully (mirror tree, `__vn_query`, `__vn_bounds`, `getComputedStyle`) or
// return a degraded constant with one advisory.

/** Any function installed on a mirror prototype; the receiver is annotated per call site so `noImplicitThis` catches a member on the wrong prototype. */
type DomMember = (...args: never[]) => unknown;

export function defineDom(name: string, value: DomMember, protos: object[] = [ElementProto]): void {
  for (const proto of protos) {
    Object.defineProperty(proto, name, { value, configurable: true, writable: true });
  }
}
export function defineDomGet(
  name: string,
  get: () => unknown,
  set?: ((value: never) => void) | undefined,
  protos: object[] = [ElementProto],
): void {
  for (const proto of protos) Object.defineProperty(proto, name, { get, ...(set ? { set } : {}), configurable: true });
}
export type TreeWalkerFilter = ((node: MirrorNode) => number) | { acceptNode?: (node: MirrorNode) => number } | null;

/// `__vn_query`, decoded. The host answers `"!<advisory>"` for a selector the engine cannot run
/// (`:has()`, `::before`), so an unanswerable query explains itself instead of reporting "no match".
export function hostQuery(selector: string, rootId: VnNodeId, mode: VnQueryMode): MirrorElement[] {
  if (rootId === 0) return [];
  const raw = __vn_query(String(selector), rootId, mode);
  if (raw === "") return [];
  if (raw.charCodeAt(0) === 33 /* ! */) {
    warnCompat(
      `selector:${selector}`,
      `querySelector/closest/matches(${JSON.stringify(String(selector))}) — ${raw.slice(1)}`,
    );
    return [];
  }
  const out: MirrorElement[] = [];
  for (const part of raw.split(",")) {
    const node = nodeById(Number(part));
    if (node) out.push(node as MirrorElement);
  }
  return out;
}
export const queryOne = (selector: string, rootId: VnNodeId, mode: VnQueryMode = 0): MirrorElement | null =>
  hostQuery(selector, rootId, mode)[0] ?? null;

const ZERO_RECT = {
  x: 0,
  y: 0,
  top: 0,
  left: 0,
  right: 0,
  bottom: 0,
  width: 0,
  height: 0,
};

/// The viewport as `document.documentElement` reports it: from the client-area origin (the space
/// all rects live in) to the far corner of the app root, which is inset by host chrome. The root's
/// own size would make @floating-ui clip against an offset space and shift every popover left.
export function viewportRect(): MirrorRect {
  if (!appRoot) return { ...ZERO_RECT };
  const r = rectOf(appRoot);
  const width = Math.round(r.x + r.width);
  const height = Math.round(r.y + r.height);
  return { x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height };
}
/** `__vn_bounds`, with `"stale"` (the host cannot force layout inside an input handler) folded into `""`. A script-facing read (`kind`) leaves one advisory per call site and kind. */
export function boundsOf(node: MirrorNode, kind?: string): VnRect {
  if (!node.__vnId) return "";
  const raw = __vn_bounds(node.__vnId);
  if (raw !== "stale") return raw;
  if (kind) {
    // Frames only (V8 and bun put an `Error` line first, JSC does not): [0] this
    // function, [1] rectOf, [2] the accessor, [3] its caller.
    const frames = (new Error().stack ?? "").split("\n").filter((l) => /^\s*at |@/.test(l));
    const site = frames[3]?.trim() ?? "";
    warnCompat(
      `layout-stale:${kind}:${site}`,
      `${kind} answered 0 for an element that has not been laid out yet` +
        (site ? ` (${site})` : "") +
        ": vue-native cannot lay out while the platform is dispatching mouse or keyboard input, " +
        "so a node mounted in this same event has no size; a browser would lay out and answer it. " +
        "Measure from requestAnimationFrame() or a ResizeObserver callback",
    );
  }
  return "";
}
/** The node's LAYOUT box, before any CSS `transform`: what `offsetWidth`, `clientWidth` and ResizeObserver report. */
export function rectOf(node: MirrorNode, kind?: string): MirrorRect {
  return rectFrom(boundsOf(node, kind), 0);
}
/**
 * Where the node was DRAWN: its border box under its own and its ancestors' `transform`, as `getBoundingClientRect()`
 * reports. Poppers sit at (0, 0) and move by `translate()`, so layout boxes would anchor to the window corner. The
 * host appends these four numbers to `__vn_bounds` only when a transform moved the box.
 */
export function paintedRectOf(node: MirrorNode, kind?: string): MirrorRect {
  return rectFrom(boundsOf(node, kind), 4);
}
function rectFrom(raw: VnRect, at: 0 | 4): MirrorRect {
  if (!raw)
    return {
      ...ZERO_RECT,
      toJSON() {
        return this;
      },
    };
  const n = raw.split(",").map(Number);
  const [x = NaN, y = NaN, w = NaN, h = NaN] = n.length >= at + 4 ? n.slice(at, at + 4) : n;
  return {
    x,
    y,
    width: w,
    height: h,
    top: y,
    left: x,
    right: x + w,
    bottom: y + h,
    toJSON() {
      return this;
    },
  };
}

defineDom(
  "contains",
  function (this: MirrorNode, other: MirrorNode | null | undefined) {
    for (let n: MirrorNode | null | undefined = other; n; n = n.parentNode) if (n === this) return true;
    return false;
  },
  NODE_MEMBERS,
);
defineDomGet(
  "ownerDocument",
  function (this: MirrorNode) {
    const root = windowRootOf(this);
    return root ? windowDocument(root) : globalThis.document;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "nodeType",
  function (this: MirrorNode) {
    return this.kind === 2 ? 3 : this.kind === 3 ? 8 : 1;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "nodeName",
  function (this: MirrorNode) {
    return (this as Partial<MirrorElement>).tagName ?? (this.kind === 2 ? "#text" : "#comment");
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "isConnected",
  function (this: MirrorNode) {
    for (let n: MirrorNode | null = this; n; n = n.parentNode) if (n === appRoot) return true;
    return false;
  },
  undefined,
  NODE_MEMBERS,
);
for (const alias of ["data", "nodeValue"]) {
  defineDomGet(
    alias,
    function (this: MirrorText) {
      return this.text ?? "";
    },
    function (this: MirrorText, value: unknown) {
      nodeOps.setText(this, value == null ? "" : String(value));
    },
    [TextProto],
  );
}
defineDomGet(
  "textContent",
  function (this: MirrorText) {
    return this.text ?? "";
  },
  function (this: MirrorText, value: unknown) {
    nodeOps.setText(this, value == null ? "" : String(value));
  },
  [TextProto],
);
// `parentElement`: the parent when it is an Element. The one non-mirror parent is `document.head`
// (an element in a browser), so the test is "an element node or an object shaped like one".
defineDomGet(
  "parentElement",
  function (this: MirrorNode) {
    const p = this.parentNode as unknown as { nodeType?: unknown; tagName?: unknown } | null;
    if (!p) return null;
    return p.nodeType === 1 || (p.nodeType === undefined && typeof p.tagName === "string") ? p : null;
  },
  undefined,
  NODE_MEMBERS,
);
export function siblingOf(node: MirrorNode, step: 1 | -1): MirrorNode | null {
  const p = node.parentNode;
  if (!p || !p.childNodes) return null;
  // `document.head`'s list is a NodeList with no `indexOf`; a mirror's is an Array.
  const i = Array.prototype.indexOf.call(p.childNodes, node);
  return i >= 0 ? (p.childNodes[i + step] ?? null) : null;
}
defineDomGet(
  "nextSibling",
  function (this: MirrorNode) {
    return siblingOf(this, 1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "previousSibling",
  function (this: MirrorNode) {
    return siblingOf(this, -1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "firstChild",
  function (this: MirrorNode) {
    return this.childNodes?.[0] ?? null;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "lastChild",
  function (this: MirrorNode) {
    const kids = this.childNodes;
    return kids && kids.length ? kids[kids.length - 1] : null;
  },
  undefined,
  NODE_MEMBERS,
);
defineDom(
  "hasChildNodes",
  function (this: MirrorNode) {
    return !!this.childNodes?.length;
  },
  NODE_MEMBERS,
);
defineDom(
  "getRootNode",
  function (this: MirrorNode) {
    let n: MirrorNode = this;
    while (n.parentNode) n = n.parentNode;
    return n === appRoot ? (globalThis.document ?? n) : n;
  },
  NODE_MEMBERS,
);
// `children` is the element-only view of `childNodes`; `aria-hidden` (reka's Dialog) walks it and
// calls `getAttribute` on each, so text and comment mirrors must not appear.
const elementChildren = (n: MirrorNode): MirrorElement[] =>
  (n.childNodes || []).filter((c): c is MirrorElement => c.kind === 1);
const childrenOf = new WeakMap<object, MirrorCollection<MirrorElement>>();
defineDomGet("children", function (this: MirrorElement) {
  let kids = childrenOf.get(this);
  if (!kids) childrenOf.set(this, (kids = liveCollection(HTMLCollectionProto, true, () => elementChildren(this))));
  return kids;
});
defineDomGet("firstElementChild", function (this: MirrorElement) {
  return elementChildren(this)[0] ?? null;
});
defineDomGet("lastElementChild", function (this: MirrorElement) {
  const kids = elementChildren(this);
  return kids[kids.length - 1] ?? null;
});
defineDomGet("childElementCount", function (this: MirrorElement) {
  return elementChildren(this).length;
});
function elementSibling(node: MirrorNode, step: number): MirrorElement | null {
  const p = node.parentNode;
  if (!p || !p.childNodes) return null;
  for (let i = Array.prototype.indexOf.call(p.childNodes, node) + step; i >= 0 && i < p.childNodes.length; i += step) {
    if (p.childNodes[i]!.kind === 1) return p.childNodes[i] as MirrorElement;
  }
  return null;
}
defineDomGet(
  "nextElementSibling",
  function (this: MirrorNode) {
    return elementSibling(this, 1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "previousElementSibling",
  function (this: MirrorNode) {
    return elementSibling(this, -1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "textContent",
  function (this: MirrorElement) {
    let out = "";
    const walk = (n: MirrorNode) => {
      if (n.kind === 2) out += n.text ?? "";
      else if (n.kind === 1 && n.ownText) out += n.ownText;
      for (const c of n.childNodes || []) walk(c);
    };
    walk(this);
    return out;
  },
  function (this: MirrorElement, value: unknown) {
    // The engine's detached elements share this prototype and have no tree:
    // the text is theirs to keep (a `<style>` from it is read on insertion).
    if (!isMirrorNode(this)) return ownExpando(this, "textContent", value == null ? "" : String(value));
    nodeOps.setElementText(this, value == null ? "" : String(value));
  } as (value: never) => void,
);

defineDom("matches", function (this: MirrorElement, selector: string) {
  return hostQuery(selector, this.__vnId, 1).length > 0;
});
defineDom("closest", function (this: MirrorElement, selector: string) {
  return queryOne(selector, this.__vnId, 2);
});
defineDom("querySelector", function (this: MirrorElement, selector: string) {
  return queryOne(selector, this.__vnId, 0);
});
defineDom("querySelectorAll", function (this: MirrorElement, selector: string) {
  return staticList(NodeListProto, hostQuery(selector, this.__vnId, 0));
});

// Live collections walk the mirror tree, not the selector matcher: `foo:bar` is a tag name here and an invalid selector there.
export function elementsUnder(
  root: MirrorNode | null,
  inclusive: boolean,
  test: (el: MirrorElement) => boolean,
): MirrorElement[] {
  const out: MirrorElement[] = [];
  if (!root) return out;
  const walk = (n: MirrorNode) => {
    const kids = n.childNodes;
    for (let i = 0; i < kids.length; i++) {
      const c = kids[i]!;
      if (c.kind !== 1) continue;
      if (test(c)) out.push(c);
      walk(c);
    }
  };
  if (inclusive && root.kind === 1 && test(root)) out.push(root);
  walk(root);
  return out;
}
export function tagTest(qualifiedName: unknown): (el: MirrorElement) => boolean {
  const q = String(qualifiedName);
  if (q === "*") return () => true;
  const lower = q.toLowerCase();
  return (el) => (el._ns ? el.tag === q : el.tag.toLowerCase() === lower);
}
export function classTest(classNames: unknown): (el: MirrorElement) => boolean {
  const want = String(classNames)
    .split(/[ \t\n\f\r]+/)
    .filter(Boolean);
  if (!want.length) return () => false;
  return (el) => {
    const have = el.attrs.class;
    if (!have) return false;
    const tokens = have.split(/[ \t\n\f\r]+/);
    return want.every((w) => tokens.includes(w));
  };
}
defineDom("getElementsByTagName", function (this: MirrorElement, qualifiedName: unknown) {
  const test = tagTest(qualifiedName);
  return liveCollection(HTMLCollectionProto, true, () => elementsUnder(this, false, test));
});
defineDom("getElementsByClassName", function (this: MirrorElement, classNames: unknown) {
  const test = classTest(classNames);
  return liveCollection(HTMLCollectionProto, true, () => elementsUnder(this, false, test));
});

// A mirror's namespace is `_ns`; any other element on this prototype (the engine's detached ones) is HTML.
defineDomGet("namespaceURI", function (this: MirrorElement) {
  return this._ns === "svg" ? SVG_NS : this._ns === "mathml" ? MATHML_NS : HTML_NS;
});
defineDomGet("localName", function (this: MirrorElement) {
  const name = typeof this.tag === "string" ? this.tag : String(this.tagName ?? "").toLowerCase();
  return this._ns ? name.slice(name.indexOf(":") + 1) : name.toLowerCase();
});
defineDomGet("prefix", function (this: MirrorElement) {
  return null;
});

// Bounds come from the last painted frame. The rect methods answer where the box was DRAWN (transforms applied); the size accessors answer the layout box.
defineDom("getBoundingClientRect", function (this: MirrorElement) {
  return asDomRect(paintedRectOf(this, "getBoundingClientRect()"));
});
defineDom("getClientRects", function (this: MirrorElement) {
  const r = paintedRectOf(this, "getClientRects()");
  return staticList(DOMRectListProto, r.width || r.height ? [asDomRect(r)] : []);
});
for (const [name, pick] of [
  ["offsetWidth", (r: MirrorRect) => r.width],
  ["offsetHeight", (r: MirrorRect) => r.height],
  ["clientWidth", (r: MirrorRect) => r.width],
  ["clientHeight", (r: MirrorRect) => r.height],
] as [string, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    return Math.round(pick(rectOf(this, name)));
  });
}
// `scrollWidth` / `scrollHeight` are the CONTENT box: `el.scrollTop = el.scrollHeight` is wrong by the whole overflow if this answers the viewport.
for (const [name, pick, fallback] of [
  ["scrollWidth", (s: ScrollPos) => s.scrollWidth, (r: MirrorRect) => r.width],
  ["scrollHeight", (s: ScrollPos) => s.scrollHeight, (r: MirrorRect) => r.height],
] as [string, (s: ScrollPos) => number, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    const s = hostScroll(this, 0, NaN, NaN, true);
    return Math.round(s ? pick(s) : fallback(rectOf(this, name)));
  });
}
// `offset*` is measured from the offsetParent in the DOM; with no positioned-ancestor chain these are window-relative.
for (const [name, pick] of [
  ["offsetTop", (r: MirrorRect) => r.top],
  ["offsetLeft", (r: MirrorRect) => r.left],
] as [string, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    warnCompat(
      "offsetParent",
      "offsetTop/offsetLeft are measured from the WINDOW, not from an offsetParent — " +
        "vue-native has no positioned-ancestor chain; use getBoundingClientRect(), which is exact",
    );
    return Math.round(pick(rectOf(this, name)));
  });
}
defineDomGet("offsetParent", () => null);
// Border widths are not published (the painted rect is the border box); 0, because @floating-ui adds these to every clipping rect and `undefined` makes it NaN.
defineDomGet("clientTop", () => 0);
defineDomGet("clientLeft", () => 0);
