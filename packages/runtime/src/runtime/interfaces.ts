import { isMirrorEl } from "./controls";
import { mark, vacant } from "./realm";
import { ElementProto, NodeProto, TextControlProto, TextProto } from "./protos";
import type { MirrorElement, MirrorNode } from "./types";
import { defineGlobal } from "./globals";
import { BRANDS, HTMLCollectionProto, NodeListProto, isChildList } from "./collections";
import { isMirrorNode } from "./listeners";
import { NamedNodeMapImpl } from "./attributes";
import { AttrImpl } from "./attr";
import { warnCompat } from "../compat";
import { fragments } from "./tree-ops";
import { looseCreated } from "./nodes";
import { nodeOps } from "./node-ops";
import { MEDIA_CONSTANTS, TAG_CLASSES, TAG_PARENTS } from "./tags";

/** Members each tag class's PROTOTYPE carries as own accessors; each delegates to the element's own path, so behaviour is the same however it is reached. */
const TAG_ACCESSORS: Record<string, readonly string[]> = {
  INPUT: ["value", "checked", "defaultValue", "defaultChecked", "indeterminate"],
  TEXTAREA: ["value", "defaultValue"],
  SELECT: ["value", "selectedIndex"],
  OPTION: ["value", "selected"],
  IMG: ["src", "currentSrc", "complete", "naturalWidth", "naturalHeight"],
};

for (const [name, spec] of Object.entries(TAG_CLASSES)) {
  const tags: readonly string[] = typeof spec === "string" ? [spec] : spec;
  const tag = tags[0]!;
  const existing = (globalThis as Record<string, unknown>)[name];
  const ours = vacant(globalThis, name);
  const parentName = TAG_PARENTS[name];
  const parentProto =
    (parentName &&
      (globalThis as unknown as Record<string, { prototype?: object } | undefined>)[parentName]?.prototype) ||
    (tag === "INPUT" || tag === "TEXTAREA" ? TextControlProto : ElementProto);
  if (typeof existing === "function" && !ours) {
    // The ENGINE's class (redjs's HTMLImageElement, for `new Image()`) knows
    // nothing of mirror elements; extend its `instanceof` to cover them.
    try {
      const own = Function.prototype[Symbol.hasInstance];
      Object.defineProperty(existing, Symbol.hasInstance, {
        value(this: unknown, x: unknown) {
          return (isMirrorEl(x) && tags.includes(x.tagName)) || own.call(this, x);
        },
        configurable: true,
      });
      // redjs's bootstrap HTMLVideoElement is a bare class on Object.prototype:
      // it goes under HTMLMediaElement, as a browser's does.
      const p = (existing as { prototype?: object }).prototype;
      if (parentName && p && Object.getPrototypeOf(p) === Object.prototype) Object.setPrototypeOf(p, parentProto);
    } catch {
      /* a class that refuses the property keeps its own answer */
    }
    continue;
  }
  if (!ours) continue;
  // Mirror nodes are all plain HTMLElement, so `instanceof` is answered by tag
  // name; the prototype stays on HTMLElement's chain, as in a browser.
  const klass = class {};
  Object.defineProperty(klass, "name", { value: name });
  Object.defineProperty(klass, Symbol.hasInstance, {
    value: (x: unknown) => !!x && typeof x === "object" && tags.includes((x as MirrorElement).tagName),
  });
  Object.setPrototypeOf(klass.prototype, parentProto);
  if (name === "HTMLMediaElement") {
    for (const [k, v] of MEDIA_CONSTANTS) {
      Object.defineProperty(klass, k, { value: v, enumerable: true });
      Object.defineProperty(klass.prototype, k, { value: v, enumerable: true });
    }
  }
  for (const prop of TAG_ACCESSORS[tag] ?? []) {
    // The wrong receiver throws, as in a browser. So does an object that
    // inherits from this prototype: delegating would land right back here.
    const check = (self: unknown, verb: string): Record<string, unknown> => {
      if (
        !self ||
        typeof self !== "object" ||
        (self as MirrorElement).tagName !== tag ||
        klass.prototype.isPrototypeOf(self)
      ) {
        throw new TypeError(`Failed to ${verb} the '${prop}' property on '${name}': Illegal invocation`);
      }
      return self as Record<string, unknown>;
    };
    Object.defineProperty(klass.prototype, prop, {
      get(this: unknown) {
        return check(this, "read")[prop];
      },
      set(this: unknown, v: unknown) {
        check(this, "set")[prop] = v;
      },
      enumerable: true,
      configurable: true,
    });
  }
  defineGlobal(name, klass);
}

// WebIDL interface objects the realm lacks: code that merely names one (tsyringe's `design:paramtypes` with `Window`) must not throw a ReferenceError.

type Iface = { prototype: object };

function defineInterface(
  name: string,
  is: (x: unknown) => boolean,
  parent: Iface | null,
  construct?: (...args: unknown[]) => object,
): Iface | undefined {
  if (!vacant(globalThis, name)) return (globalThis as Record<string, unknown>)[name] as Iface;
  const iface = function (this: unknown, ...args: unknown[]): object {
    if (construct && new.target) return construct(...args);
    throw new TypeError(
      construct
        ? `Failed to construct '${name}': Please use the 'new' operator, this DOM object constructor cannot be called as a function.`
        : "Illegal constructor",
    );
  };
  Object.defineProperty(iface, "name", { value: name });
  const proto = Object.create(parent?.prototype ?? Object.prototype, {
    constructor: { value: iface, writable: true, configurable: true },
  });
  Object.defineProperty(iface, "prototype", { value: proto });
  Object.defineProperty(iface, Symbol.hasInstance, {
    value: (x: unknown) => is(x) || (!!x && typeof x === "object" && proto.isPrototypeOf(x)),
  });
  defineGlobal(name, iface);
  return iface as unknown as Iface;
}

export const theGlobal = (k: string) => (globalThis as Record<string, unknown>)[k];
const ET: Iface | null = typeof EventTarget === "function" ? (EventTarget as unknown as Iface) : null;

function adoptInterface(
  name: string,
  proto: object,
  parent: Iface | null,
  also?: (x: object) => boolean,
): Iface | undefined {
  const engine = theGlobal(name);
  const iface = vacant(globalThis, name)
    ? function (): never {
        throw new TypeError("Illegal constructor");
      }
    : null;
  if (!iface) {
    const engineProto = typeof engine === "function" ? (engine as unknown as Iface).prototype : undefined;
    if (engineProto && engineProto !== proto) {
      try {
        Object.setPrototypeOf(proto, engineProto);
      } catch {
        /* keeps its own chain */
      }
    }
  } else {
    Object.defineProperty(iface, "name", { value: name });
    const parentProto = parent?.prototype ?? Object.prototype;
    if (Object.getPrototypeOf(proto) !== parentProto) Object.setPrototypeOf(proto, parentProto);
    Object.defineProperty(proto, "constructor", { value: iface, writable: true, configurable: true });
    Object.defineProperty(iface, "prototype", { value: proto, writable: false });
  }
  const target = iface ?? engine;
  if (also && typeof target === "function") {
    const own = Function.prototype[Symbol.hasInstance];
    try {
      Object.defineProperty(target, Symbol.hasInstance, {
        value(this: unknown, x: unknown) {
          return (!!x && typeof x === "object" && also(x)) || own.call(this, x);
        },
        configurable: true,
      });
    } catch {
      /* an interface that refuses the property keeps its own answer */
    }
  }
  if (iface) defineGlobal(name, iface);
  return (target ?? undefined) as Iface | undefined;
}
adoptInterface("NodeList", NodeListProto, null, isChildList);
adoptInterface("HTMLCollection", HTMLCollectionProto, null);

// redjs aliases `Node` to `Element`; this gives Node its own interface (constants, text nodes `instanceof`) and re-parents Element, text mirrors and Attr under it.
const NODE_CONSTANTS: readonly (readonly [string, number])[] = [
  ["ELEMENT_NODE", 1],
  ["ATTRIBUTE_NODE", 2],
  ["TEXT_NODE", 3],
  ["CDATA_SECTION_NODE", 4],
  ["ENTITY_REFERENCE_NODE", 5],
  ["ENTITY_NODE", 6],
  ["PROCESSING_INSTRUCTION_NODE", 7],
  ["COMMENT_NODE", 8],
  ["DOCUMENT_NODE", 9],
  ["DOCUMENT_TYPE_NODE", 10],
  ["DOCUMENT_FRAGMENT_NODE", 11],
  ["NOTATION_NODE", 12],
  ["DOCUMENT_POSITION_DISCONNECTED", 1],
  ["DOCUMENT_POSITION_PRECEDING", 2],
  ["DOCUMENT_POSITION_FOLLOWING", 4],
  ["DOCUMENT_POSITION_CONTAINS", 8],
  ["DOCUMENT_POSITION_CONTAINED_BY", 16],
  ["DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32],
];
const isDocument = (x: unknown) => x != null && x === theGlobal("document");
const NodeIface: Iface | null = (() => {
  const engineNode = theGlobal("Node");
  const engineElement = theGlobal("Element");
  if (typeof engineNode === "function" && engineNode !== engineElement && !vacant(globalThis, "Node")) {
    return engineNode as unknown as Iface;
  }
  const iface = function (): never {
    throw new TypeError("Illegal constructor");
  };
  Object.defineProperty(iface, "name", { value: "Node" });
  const proto = NodeProto;
  Object.setPrototypeOf(proto, ET?.prototype ?? Object.prototype);
  Object.defineProperty(proto, "constructor", { value: iface, writable: true, configurable: true });
  Object.defineProperty(iface, "prototype", { value: proto, writable: false });
  for (const [k, v] of NODE_CONSTANTS) {
    Object.defineProperty(iface, k, { value: v, enumerable: true });
    Object.defineProperty(proto, k, { value: v, enumerable: true });
  }
  Object.defineProperty(iface, Symbol.hasInstance, {
    value: (x: unknown) =>
      !!x &&
      typeof x === "object" &&
      (proto.isPrototypeOf(x) || isMirrorNode(x) || isDocument(x) || x instanceof AttrImpl),
  });
  try {
    Object.defineProperty(globalThis, "Node", { value: mark(iface), configurable: true, writable: true });
  } catch {
    warnCompat(
      "global:Node",
      "Node could not be installed over the engine's own: Node.TEXT_NODE and the other constants are missing",
    );
    return null;
  }
  if (typeof engineElement === "function" && !vacant(globalThis, "Element")) {
    try {
      const elementProto = (engineElement as unknown as Iface).prototype;
      if (Object.getPrototypeOf(elementProto) !== proto) Object.setPrototypeOf(elementProto, proto);
    } catch {
      /* `instanceof Node` still holds for mirrors, by kind */
    }
  }
  return iface as unknown as Iface;
})();
const ElementIface: Iface | null = (() => {
  if (!vacant(globalThis, "Element")) return (theGlobal("Element") as Iface | undefined) ?? null;
  const iface = defineInterface("Element", () => false, NodeIface) ?? null;
  if (iface) adoptInterface("HTMLElement", ElementProto, iface);
  return iface;
})();

function wrapElementHasInstance(iface: unknown, mirrorIs: (el: MirrorElement) => boolean): void {
  if (typeof iface !== "function") return;
  const own = Function.prototype[Symbol.hasInstance];
  try {
    Object.defineProperty(iface, Symbol.hasInstance, {
      value(this: unknown, x: unknown) {
        return isMirrorEl(x) ? mirrorIs(x) : own.call(this, x);
      },
      configurable: true,
    });
  } catch {
    /* an interface that refuses the property keeps its own answer */
  }
}
wrapElementHasInstance(theGlobal("HTMLElement"), (el) => !el._ns && !fragments.has(el));

const WindowIface = defineInterface("Window", (x) => x != null && (x === globalThis || x === theGlobal("window")), ET);
if (WindowIface) {
  try {
    // `window.constructor === Window`, which is how some code spots a window.
    if (
      !Object.prototype.hasOwnProperty.call(globalThis, "constructor") ||
      theGlobal("constructor") === Object ||
      vacant(globalThis, "constructor")
    )
      Object.defineProperty(globalThis, "constructor", { value: WindowIface, writable: true, configurable: true });
  } catch {
    /* a global that refuses the property keeps Object's */
  }
}
const DocumentIface = defineInterface("Document", isDocument, NodeIface ?? ET);
defineInterface("HTMLDocument", isDocument, DocumentIface ?? ET);
defineInterface("Location", (x) => x != null && x === theGlobal("location"), null);
defineInterface("Screen", (x) => x != null && x === theGlobal("screen"), ET);

const mirrorKind = (x: unknown) =>
  !!x && typeof x === "object" && Object.getPrototypeOf(x) === TextProto ? (x as MirrorNode).kind : 0;
const CharacterDataIface = defineInterface(
  "CharacterData",
  (x) => mirrorKind(x) === 2 || mirrorKind(x) === 3,
  NodeIface,
);
if (CharacterDataIface && Object.getPrototypeOf(TextProto) !== CharacterDataIface.prototype) {
  try {
    Object.setPrototypeOf(TextProto, CharacterDataIface.prototype);
  } catch {
    /* `instanceof` still holds, by kind */
  }
}
defineInterface(
  "Text",
  (x) => mirrorKind(x) === 2,
  CharacterDataIface ?? null,
  (data) => looseCreated(nodeOps.createText(data === undefined ? "" : String(data))),
);
defineInterface(
  "Comment",
  (x) => mirrorKind(x) === 3,
  CharacterDataIface ?? null,
  (data) => {
    const c = looseCreated(nodeOps.createComment(data === undefined ? "" : String(data)));
    c.text = data === undefined ? "" : String(data);
    return c;
  },
);
defineInterface("ShadowRoot", () => false, theGlobal("DocumentFragment") as Iface | null);
defineInterface("Attr", (x) => x instanceof AttrImpl, NodeIface);
if (NodeIface && Object.getPrototypeOf(AttrImpl.prototype) !== NodeIface.prototype) {
  Object.setPrototypeOf(AttrImpl.prototype, NodeIface.prototype);
}
defineInterface("NamedNodeMap", (x) => x instanceof NamedNodeMapImpl, null);

// -- SVG and MathML elements: `instanceof SVGElement` / `MathMLElement` / the tag's interface, never HTMLElement (Vue's Teleport and motion's scroll offsets test these).
const SVG_INTERFACES: readonly (readonly [name: string, parent: string, tags: readonly string[]])[] = [
  ["SVGGraphicsElement", "SVGElement", []],
  ["SVGGeometryElement", "SVGGraphicsElement", []],
  ["SVGSVGElement", "SVGGraphicsElement", ["svg"]],
  ["SVGGElement", "SVGGraphicsElement", ["g"]],
  ["SVGDefsElement", "SVGGraphicsElement", ["defs"]],
  ["SVGUseElement", "SVGGraphicsElement", ["use"]],
  ["SVGImageElement", "SVGGraphicsElement", ["image"]],
  ["SVGForeignObjectElement", "SVGGraphicsElement", ["foreignObject"]],
  ["SVGAElement", "SVGGraphicsElement", ["a"]],
  ["SVGSwitchElement", "SVGGraphicsElement", ["switch"]],
  ["SVGPathElement", "SVGGeometryElement", ["path"]],
  ["SVGRectElement", "SVGGeometryElement", ["rect"]],
  ["SVGCircleElement", "SVGGeometryElement", ["circle"]],
  ["SVGEllipseElement", "SVGGeometryElement", ["ellipse"]],
  ["SVGLineElement", "SVGGeometryElement", ["line"]],
  ["SVGPolylineElement", "SVGGeometryElement", ["polyline"]],
  ["SVGPolygonElement", "SVGGeometryElement", ["polygon"]],
  ["SVGTextContentElement", "SVGGraphicsElement", []],
  ["SVGTextPositioningElement", "SVGTextContentElement", []],
  ["SVGTextElement", "SVGTextPositioningElement", ["text"]],
  ["SVGTSpanElement", "SVGTextPositioningElement", ["tspan"]],
  ["SVGTextPathElement", "SVGTextContentElement", ["textPath"]],
  ["SVGGradientElement", "SVGElement", []],
  ["SVGLinearGradientElement", "SVGGradientElement", ["linearGradient"]],
  ["SVGRadialGradientElement", "SVGGradientElement", ["radialGradient"]],
  ["SVGStopElement", "SVGElement", ["stop"]],
  ["SVGClipPathElement", "SVGElement", ["clipPath"]],
  ["SVGMaskElement", "SVGElement", ["mask"]],
  ["SVGPatternElement", "SVGElement", ["pattern"]],
  ["SVGMarkerElement", "SVGElement", ["marker"]],
  ["SVGSymbolElement", "SVGElement", ["symbol"]],
  ["SVGFilterElement", "SVGElement", ["filter"]],
  ["SVGTitleElement", "SVGElement", ["title"]],
  ["SVGDescElement", "SVGElement", ["desc"]],
  ["SVGMetadataElement", "SVGElement", ["metadata"]],
  ["SVGStyleElement", "SVGElement", ["style"]],
  ["SVGScriptElement", "SVGElement", ["script"]],
  ["SVGViewElement", "SVGElement", ["view"]],
];
{
  const isSvg = (x: unknown): x is MirrorElement => isMirrorEl(x) && x._ns === "svg";
  const SVGElementIface = defineInterface("SVGElement", isSvg, ElementIface);
  wrapElementHasInstance(SVGElementIface, (el) => el._ns === "svg");
  defineInterface("MathMLElement", (x) => isMirrorEl(x) && x._ns === "mathml", ElementIface);
  // Each interface's tags plus those under it; the table lists parents first, so walking backwards completes a child before its parent.
  const tagsOf = new Map<string, Set<string>>(SVG_INTERFACES.map(([name, , tags]) => [name, new Set(tags)]));
  for (let i = SVG_INTERFACES.length - 1; i >= 0; i--) {
    const [name, parent] = SVG_INTERFACES[i]!;
    const into = tagsOf.get(parent);
    if (into) for (const t of tagsOf.get(name)!) into.add(t);
  }
  for (const [name, parent] of SVG_INTERFACES) {
    const tags = tagsOf.get(name)!;
    defineInterface(
      name,
      (x) => isSvg(x) && tags.has(x.tag),
      (theGlobal(parent) as Iface | undefined) ?? SVGElementIface ?? null,
    );
  }
}

for (const [name, set] of Object.entries(BRANDS)) {
  defineInterface(name, (x) => !!x && typeof x === "object" && set.has(x), null);
}
defineInterface(
  "MediaQueryList",
  (x) =>
    !!ET &&
    x instanceof (ET as unknown as typeof EventTarget) &&
    typeof (x as { media?: unknown }).media === "string" &&
    "matches" in x,
  ET,
);
defineInterface("Performance", (x) => x != null && x === theGlobal("performance"), ET);
defineInterface("Crypto", (x) => x != null && x === theGlobal("crypto"), null);

// InputEvent's `dataTransfer` and `getTargetRanges()`: only for contenteditable; in a text field's `beforeinput` /
// `input` Chromium has no DataTransfer (the text is `data`) and no target ranges.
{
  const IE = theGlobal("InputEvent");
  const proto = typeof IE === "function" ? (IE as { prototype: object }).prototype : null;
  if (proto && vacant(proto, "dataTransfer"))
    Object.defineProperty(proto, "dataTransfer", {
      get: mark(function dataTransfer() {
        return null;
      }),
      enumerable: true,
      configurable: true,
    });
  if (proto && vacant(proto, "getTargetRanges"))
    Object.defineProperty(proto, "getTargetRanges", {
      value: mark(function getTargetRanges(): unknown[] {
        return [];
      }),
      writable: true,
      enumerable: true,
      configurable: true,
    });
}
