import type { InlineDecl, InlineStyleState, MirrorElement } from "./types";
import { warnCompat } from "../compat";
import { I } from "./event-names";
import { notifyMutation, observers } from "./mutation-observer";

// `\B` so a leading capital is only lowercased, never prefixed: `withKeys` runs this over
// `KeyboardEvent.key`, and "Enter" must become "enter", not "-enter".
export const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

/** A `:style` key or `CSSStyleDeclaration` name as CSS spells it (`backgroundColor` -> `background-color`); custom properties stay as written. */
export function cssPropName(k: string): string {
  if (k.startsWith("--")) return k;
  if (k === "cssFloat") return "float";
  if (k.includes("-")) return k.toLowerCase();
  const h = hyphenate(k);
  return /^(webkit|moz|ms)-/.test(h) ? "-" + h : h;
}
export const cssomName = (k: string): string => (k.startsWith("--") ? k : k.toLowerCase());
export const IMPORTANT_RE = /\s*!\s*important\s*$/i;
const PROP_NAME_RE = /^(?:--[^\s:;]+|-?[A-Za-z_\u0080-￿][\w\u0080-￿-]*)$/;

/** A declaration's value as stored, trimmed. An empty custom property is valid CSS but "" means remove on the host channel, so it is kept as one space. */
export const inlineValue = (v: string, custom: boolean): string => {
  const t = v.trim();
  return t === "" && custom ? " " : t;
};

export function inlineOf(el: MirrorElement): InlineStyleState {
  return (el._inline ??= { decls: new Map(), present: false, raw: null, cssVarText: "" });
}
const hostDecl = (d: InlineDecl): string => (d.important ? `${d.value} !important` : d.value);
export function serializeInline(st: InlineStyleState | undefined): string {
  let out = "";
  if (st) {
    for (const [name, d] of st.decls) {
      out += `${out ? " " : ""}${name}: ${d.value}${d.important ? " !important" : ""};`;
    }
  }
  return out;
}
/** Said once, at the first read: unlike a browser, serialization keeps values as written (no re-spelling, no shorthand folding, rejected values kept). */
export function noteInlineSerialization(): void {
  warnCompat(
    "inline-style-serialize",
    "el.style.cssText / getAttribute('style') serialize the inline declarations as they were written: " +
      "values are not re-spelled (`#f00` stays `#f00`), a shorthand is not expanded into its longhands " +
      '(after `margin: 0`, `el.style.marginTop` reads ""), and a value the engine rejects is still listed',
  );
}
export function styleAttribute(el: MirrorElement): string | null {
  const st = el._inline;
  if (!st || !st.present) return null;
  if (st.raw !== null) return st.raw;
  const text = serializeInline(st);
  if (text) noteInlineSerialization();
  return text;
}
const styleBefore = (el: MirrorElement): string | null => (observers.size ? styleAttribute(el) : null);
function styleTouched(el: MirrorElement, st: InlineStyleState, before: string | null): void {
  st.present = true;
  st.raw = null;
  if (el.attrs) el.attrs.style = "";
  notifyMutation(el, "attributes", { attributeName: "style", oldValue: before });
}

export function setInlineDecl(el: MirrorElement, name: string, value: string, important: boolean): boolean {
  if (value === "") return removeInlineDecl(el, name) !== null;
  const st = inlineOf(el);
  const prev = st.decls.get(name);
  if (prev && prev.value === value && prev.important === important) return false;
  const before = styleBefore(el);
  const d = { value, important };
  st.decls.set(name, d);
  __vn_set_style(el.__vnId, I(name), hostDecl(d));
  styleTouched(el, st, before);
  return true;
}
export function removeInlineDecl(el: MirrorElement, name: string): string | null {
  const st = el._inline;
  const prev = st?.decls.get(name);
  if (!st || !prev) return null;
  const before = styleBefore(el);
  st.decls.delete(name);
  __vn_set_style(el.__vnId, I(name), "");
  styleTouched(el, st, before);
  return prev.value;
}

/** CSS "parse a list of declarations": `;` ends one only outside strings, comments and brackets; a repeated property keeps its last, but a normal one never overrides an earlier `!important`. */
function parseInlineDecls(text: string): Map<string, InlineDecl> {
  const out = new Map<string, InlineDecl>();
  const s = String(text);
  let seg = "";
  let depth = 0;
  let quote = "";
  const flush = () => {
    const colon = seg.indexOf(":");
    if (colon > 0) {
      const raw = seg.slice(0, colon).trim();
      let value = seg.slice(colon + 1);
      const m = IMPORTANT_RE.exec(value);
      const important = !!m;
      if (m) value = value.slice(0, m.index);
      value = value.trim();
      const custom = raw.startsWith("--");
      if (PROP_NAME_RE.test(raw) && (value !== "" || custom)) {
        const name = cssomName(raw);
        value = inlineValue(value, custom);
        const prev = out.get(name);
        if (!(prev?.important && !important)) {
          out.delete(name);
          out.set(name, { value, important });
        }
      }
    }
    seg = "";
  };
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quote) {
      seg += c;
      if (c === "\\" && i + 1 < s.length) seg += s[++i];
      else if (c === quote) quote = "";
      continue;
    }
    if (c === "/" && s[i + 1] === "*") {
      const end = s.indexOf("*/", i + 2);
      i = end < 0 ? s.length : end + 1;
      seg += " ";
      continue;
    }
    if (c === "\\" && i + 1 < s.length) {
      seg += c + s[++i];
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    else if (c === "(" || c === "[" || c === "{") depth++;
    else if ((c === ")" || c === "]" || c === "}") && depth > 0) depth--;
    else if (c === ";" && depth === 0) {
      flush();
      continue;
    }
    seg += c;
  }
  flush();
  return out;
}

/**
 * `style.cssText = text`, or `setAttribute("style", text)` (`asAttribute`, which keeps `text`
 * verbatim). Unchanged declarations cost no host call while the host's order stays the new order;
 * otherwise the block is re-sent, because order decides a shorthand against its longhand.
 */
export function replaceInline(el: MirrorElement, text: string, asAttribute: boolean): void {
  const st = inlineOf(el);
  const before = styleBefore(el);
  const next = parseInlineDecls(text);
  const old = st.decls;
  const order = [...old.keys()].filter((k) => next.has(k));
  for (const k of next.keys()) if (!old.has(k)) order.push(k);
  let i = 0;
  let inPlace = true;
  for (const k of next.keys()) {
    if (order[i++] !== k) {
      inPlace = false;
      break;
    }
  }
  for (const k of old.keys()) if (!inPlace || !next.has(k)) __vn_set_style(el.__vnId, I(k), "");
  for (const [k, d] of next) {
    const p = old.get(k);
    if (inPlace && p && p.value === d.value && p.important === d.important) continue;
    __vn_set_style(el.__vnId, I(k), hostDecl(d));
  }
  st.decls = next;
  styleTouched(el, st, before);
  if (asAttribute) st.raw = String(text);
}

export const vShowOriginalDisplay = Symbol("_vod");
export const vShowHidden = Symbol("_vsh");
const DISPLAY_RE = /(^|;)\s*display\s*:/;
const RD_IMPORTANT_RE = /\s*!important$/;

/** runtime-dom's `patchStyle`: a string is the whole block (`useCssVars` variables kept), a record is diffed per property, null clears. */
export function patchStyle(el: MirrorElement, prev: unknown, next: unknown): void {
  const isCssString = typeof next === "string";
  let hasControlledDisplay = false;
  if (next && !isCssString) {
    const n = next as Record<string, unknown>;
    if (prev) {
      if (typeof prev !== "string") {
        for (const key in prev as Record<string, unknown>) if (n[key] == null) setStyleValue(el, key, "");
      } else {
        for (const prevStyle of prev.split(";")) {
          const key = prevStyle.slice(0, prevStyle.indexOf(":")).trim();
          if (key && n[key] == null) setStyleValue(el, key, "");
        }
      }
    }
    for (const key in n) {
      if (key === "display") hasControlledDisplay = true;
      setStyleValue(el, key, n[key]);
    }
  } else if (isCssString) {
    if (prev !== next) {
      const vars = el._inline?.cssVarText;
      replaceInline(el, vars ? `${next};${vars}` : next, false);
      hasControlledDisplay = DISPLAY_RE.test(next);
    }
  } else if (prev) {
    clearInline(el);
  }
  if (vShowOriginalDisplay in el) {
    el[vShowOriginalDisplay] = hasControlledDisplay ? (el._inline?.decls.get("display")?.value ?? "") : "";
    if (el[vShowHidden]) setInlineDecl(el, "display", "none", false);
  }
}

function setStyleValue(el: MirrorElement, name: string, val: unknown): void {
  if (Array.isArray(val)) {
    setStyleFallbacks(el, name, val);
    return;
  }
  const v = val == null ? "" : String(val);
  const prop = cssPropName(name);
  if (v === "") {
    removeInlineDecl(el, prop);
  } else if (prop.startsWith("--")) {
    setInlineDecl(el, prop, inlineValue(v, true), false);
  } else if (RD_IMPORTANT_RE.test(v)) {
    const bare = v.replace(RD_IMPORTANT_RE, "");
    if (bare.trim() === "") removeInlineDecl(el, prop);
    else setInlineDecl(el, prop, inlineValue(bare, false), true);
  } else {
    setInlineDecl(el, prop, inlineValue(v, false), false);
  }
}

/** `{ display: ["-webkit-box", "flex"] }`: the last entry the engine accepts (`__vn_style_accepts`) wins; with none, the declaration is left as it was. */
function setStyleFallbacks(el: MirrorElement, name: string, list: unknown[]): void {
  const flat: unknown[] = list.flat(Infinity);
  const prop = cssPropName(name);
  for (let i = flat.length - 1; i >= 0; i--) {
    const v = flat[i] == null ? "" : String(flat[i]);
    const bare = v.replace(RD_IMPORTANT_RE, "");
    if (bare.trim() === "" || styleAccepts(prop, bare.trim())) {
      if (i < flat.length - 1) {
        warnCompat(
          `style-array:${prop}`,
          `:style ${prop}: [${flat.map((x) => JSON.stringify(x)).join(", ")}] uses ${JSON.stringify(flat[i])}: ` +
            `the engine does not apply the later ${flat.length - 1 - i === 1 ? "entry" : "entries"}. A browser keeps ` +
            `the last entry IT accepts, so one that supports a later entry would use that one`,
        );
      }
      setStyleValue(el, name, flat[i]);
      return;
    }
  }
}

function styleAccepts(prop: string, value: string): boolean {
  if (prop.startsWith("--")) return true;
  if (typeof __vn_style_accepts !== "function") {
    warnCompat(
      "style-array:host",
      "an array :style value keeps its LAST entry here: this host cannot say which values the engine " +
        "accepts (__vn_style_accepts is missing), where a browser keeps the last entry it accepts",
    );
    return true;
  }
  return __vn_style_accepts(I(prop), value);
}

export function clearInline(el: MirrorElement): void {
  const st = el._inline;
  if (!st || !st.present) return;
  const before = styleBefore(el);
  for (const k of st.decls.keys()) __vn_set_style(el.__vnId, I(k), "");
  st.decls = new Map();
  st.present = false;
  st.raw = null;
  if (el.attrs) delete el.attrs.style;
  notifyMutation(el, "attributes", { attributeName: "style", oldValue: before });
}
