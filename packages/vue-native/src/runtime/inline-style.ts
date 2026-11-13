import type { MirrorElement } from "./types";
import { warnCompat } from "../compat";
import { I } from "./event-names";
import { nodeById } from "./nodes";
import { notifyMutation, observers } from "./mutation-observer";

// Inline style lives in the host: `el.style` is its CSSStyleDeclaration (`__vn_style_object`), and the
// `style` attribute reads back its declarations (`__vn_style_attr`). Here: runtime-dom's `:style` diff, and
// the attribute marker and MutationObserver records the host asks for after a change.

// `\B` so a leading capital is only lowercased, never prefixed: `withKeys` runs this over
// `KeyboardEvent.key`, and "Enter" must become "enter", not "-enter".
export const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

/** A `:style` key as CSS spells it (`backgroundColor` -> `background-color`); custom properties stay as written. */
export function cssPropName(k: string): string {
  if (k.startsWith("--")) return k;
  if (k === "cssFloat") return "float";
  if (k.includes("-")) return k.toLowerCase();
  const h = hyphenate(k);
  return /^(webkit|moz|ms)-/.test(h) ? "-" + h : h;
}

export const cssomName = (k: string): string => (k.startsWith("--") ? k : k.toLowerCase());

__vn_style_hook((id, oldValue, present) => {
  const el = nodeById(id) as MirrorElement | undefined;
  if (!el) return;
  if (el.attrs) {
    if (present) el.attrs.style = "";
    else delete el.attrs.style;
  }
  if (observers.size) notifyMutation(el, "attributes", { attributeName: "style", oldValue });
});

export function inlineDisplay(el: MirrorElement): string {
  return __vn_style_get(el.__vnId, "display");
}
export function setInlineDisplay(el: MirrorElement, value: string): void {
  __vn_style_assign(el.__vnId, "display", value);
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
      const vars = el._cssVarText;
      __vn_style_replace(el.__vnId, vars ? `${next};${vars}` : next, false);
      hasControlledDisplay = DISPLAY_RE.test(next);
    }
  } else if (prev) {
    __vn_style_clear(el.__vnId);
  }
  if (vShowOriginalDisplay in el) {
    el[vShowOriginalDisplay] = hasControlledDisplay ? inlineDisplay(el) : "";
    if (el[vShowHidden]) setInlineDisplay(el, "none");
  }
}

function setStyleValue(el: MirrorElement, name: string, val: unknown): void {
  if (Array.isArray(val)) setStyleFallbacks(el, name, val);
  else __vn_style_assign(el.__vnId, name, val == null ? "" : String(val));
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
