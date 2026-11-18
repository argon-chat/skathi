import { defineDom, defineDomGet } from "./dom-members";
import type { MirrorElement } from "./types";
import { toLong, writeAttribute } from "./patch-prop";
import { REFLECTED } from "./reflected";
import type { Reflected } from "./reflected";
import { isMirrorEl, ownExpando } from "./controls";

// `style` is not stored as text: it IS the inline declaration block, so `getAttribute("style")` reads what
// `:style` and `el.style` wrote. `attrs.style` is only a marker for `hasAttribute` / `getAttributeNames`.
const isStyleAttr = (name: unknown): boolean => String(name).toLowerCase() === "style";
defineDom("getAttribute", function (this: MirrorElement, name: unknown) {
  if (isStyleAttr(name)) return __sk_style_attr(this.__skId);
  const v = this.attrs?.[String(name)];
  return v === undefined ? null : v;
});
defineDom("hasAttribute", function (this: MirrorElement, name: unknown) {
  if (isStyleAttr(name)) return __sk_style_attr(this.__skId) !== null;
  return this.attrs ? String(name) in this.attrs : false;
});
defineDom("getAttributeNames", function (this: MirrorElement) {
  return this.attrs ? Object.keys(this.attrs) : [];
});
defineDom("setAttribute", function (this: MirrorElement, name: unknown, value: unknown) {
  const key = String(name);
  if (isStyleAttr(key)) {
    __sk_style_replace(this.__skId, String(value), true);
    return;
  }
  writeAttribute(this, key, value == null ? "" : String(value));
});
defineDom("removeAttribute", function (this: MirrorElement, name: unknown) {
  if (isStyleAttr(name)) {
    __sk_style_clear(this.__skId);
    return;
  }
  writeAttribute(this, String(name), null);
});
defineDomGet(
  "id",
  function (this: MirrorElement) {
    return isMirrorEl(this) ? (this.attrs.id ?? "") : undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "id", v);
    this.setAttribute("id", String(v));
  } as (value: never) => void,
);

// `REFLECTED` properties as accessors over their attributes. An element whose tag lacks the property
// reads `undefined` and keeps an assignment as an expando.
const TAB_INDEX_ZERO = new Set(["a", "area", "button", "frame", "iframe", "input", "object", "select", "textarea"]);
function parseHtmlInt(s: string | undefined): number | null {
  const m = s === undefined ? null : /^[\t\n\f\r ]*([-+]?\d+)/.exec(s);
  if (!m) return null;
  const n = Number(m[1]);
  return n >= -2147483648 && n <= 2147483647 ? n : null;
}
function isDetailsSummary(el: MirrorElement): boolean {
  const p = el.parentNode as MirrorElement | null;
  if (el.tag !== "summary" || !p || p.kind !== 1 || p.tag !== "details") return false;
  return p.childNodes.find((c) => c.kind === 1 && (c as MirrorElement).tag === "summary") === el;
}
function readReflected(el: MirrorElement, prop: string, r: Reflected): unknown {
  const raw = el.attrs[r.attr];
  if (r.kind === "bool") return raw !== undefined;
  if (r.kind === "string") return raw ?? "";
  const n = parseHtmlInt(raw);
  if (prop === "tabIndex") return n ?? (TAB_INDEX_ZERO.has(el.tag) || isDetailsSummary(el) ? 0 : -1);
  return n !== null && n >= 0 ? n : -1;
}
function writeReflected(el: MirrorElement, prop: string, r: Reflected, v: unknown): void {
  if (r.kind === "bool") {
    if (!v) el.removeAttribute(r.attr);
    else if (el.attrs[r.attr] === undefined) el.setAttribute(r.attr, "");
    return;
  }
  if (r.kind === "string") return el.setAttribute(r.attr, String(v));
  const n = toLong(v);
  if (n < 0 && prop !== "tabIndex") {
    throw new DOMException(
      `Failed to set the '${prop}' property: The value provided (${n}) is negative.`,
      "IndexSizeError",
    );
  }
  el.setAttribute(r.attr, String(n));
}
for (const [prop, r] of Object.entries(REFLECTED)) {
  defineDomGet(
    prop,
    function (this: MirrorElement) {
      return isMirrorEl(this) && (!r.tags || r.tags.has(this.tag)) ? readReflected(this, prop, r) : undefined;
    },
    function (this: MirrorElement, v: unknown) {
      if (!isMirrorEl(this) || (r.tags && !r.tags.has(this.tag))) return ownExpando(this, prop, v);
      writeReflected(this, prop, r, v);
    } as (value: never) => void,
  );
}
defineDomGet(
  "hidden",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    const a = this.attrs.hidden;
    return a === undefined ? false : a.toLowerCase() === "until-found" ? "until-found" : true;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "hidden", v);
    if (typeof v === "string" && v.toLowerCase() === "until-found") this.setAttribute("hidden", "until-found");
    else writeReflected(this, "hidden", REFLECTED.hidden!, v);
  } as (value: never) => void,
);
defineDomGet(
  "colorSpace",
  function (this: MirrorElement) {
    if (!isMirrorEl(this) || this.tag !== "input") return undefined;
    return this.attrs.colorspace?.toLowerCase() === "display-p3" ? "display-p3" : "limited-srgb";
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this) || this.tag !== "input") return ownExpando(this, "colorSpace", v);
    this.setAttribute("colorspace", String(v));
  } as (value: never) => void,
);
const INPUT_TYPES = new Set(
  (
    "hidden text search tel url email password date month week time datetime-local number range color " +
    "checkbox radio file submit image reset button"
  ).split(" "),
);
const TYPE_STRING_TAGS = new Set(["a", "link", "ol", "li", "source", "embed", "object", "script", "style"]);
// `type`: enumerated on <input> / <button>, read-only and derived on <select> / <textarea>, a plain string elsewhere.
defineDomGet(
  "type",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    const a = this.attrs.type;
    const low = a?.toLowerCase();
    switch (this.tag) {
      case "input":
        return low && INPUT_TYPES.has(low) ? low : "text";
      case "button":
        return low === "reset" || low === "button" ? low : "submit";
      case "select":
        return this.attrs.multiple !== undefined ? "select-multiple" : "select-one";
      case "textarea":
      case "fieldset":
      case "output":
        return this.tag;
      default:
        return TYPE_STRING_TAGS.has(this.tag) ? (a ?? "") : undefined;
    }
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "type", v);
    if (this.tag === "input" || this.tag === "button" || TYPE_STRING_TAGS.has(this.tag))
      this.setAttribute("type", String(v));
    else if (!["select", "textarea", "fieldset", "output"].includes(this.tag)) ownExpando(this, "type", v);
    // (read-only on those four: the assignment is ignored, as in sloppy-mode JS)
  } as (value: never) => void,
);
defineDomGet(
  "dir",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    const low = this.attrs.dir?.toLowerCase();
    return low === "ltr" || low === "rtl" || low === "auto" ? low : "";
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "dir", v);
    this.setAttribute("dir", String(v));
  } as (value: never) => void,
);
defineDom("toggleAttribute", function (this: MirrorElement, name: string, force?: boolean) {
  const has = this.hasAttribute(name);
  const on = force === undefined ? !has : !!force;
  if (on && !has) this.setAttribute(name, "");
  else if (!on && has) this.removeAttribute(name);
  return on;
});
