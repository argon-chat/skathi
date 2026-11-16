import type { MirrorElement } from "./types";
import { nodeById } from "./nodes";
import { notifyMutation, observers } from "./mutation-observer";
import { noteWrite } from "./perf-advice";

// Inline style lives in the host: `el.style` is its CSSStyleDeclaration (`__sk_style_object`), and the
// `style` attribute reads back its declarations (`__sk_style_attr`). Here: runtime-dom's `:style` diff, and
// the attribute marker and MutationObserver records the host asks for after a change.

// `\B` so a leading capital is only lowercased, never prefixed: `withKeys` runs this over
// `KeyboardEvent.key`, and "Enter" must become "enter", not "-enter".
export const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

export const cssomName = (k: string): string => (k.startsWith("--") ? k : k.toLowerCase());

__sk_style_hook((id, oldValue, present) => {
  const el = nodeById(id) as MirrorElement | undefined;
  if (!el) return;
  if (el.attrs) {
    if (present) el.attrs.style = "";
    else delete el.attrs.style;
  }
  if (observers.size) notifyMutation(el, "attributes", { attributeName: "style", oldValue });
});

export function inlineDisplay(el: MirrorElement): string {
  return __sk_style_get(el.__skId, "display");
}
export function setInlineDisplay(el: MirrorElement, value: string): void {
  __sk_style_assign(el.__skId, "display", value);
}

export const vShowOriginalDisplay = Symbol("_vod");
export const vShowHidden = Symbol("_vsh");
const DISPLAY_RE = /(^|;)\s*display\s*:/;

/** runtime-dom's `patchStyle`: a string is the whole block (`useCssVars` variables kept), a record is diffed per property, null clears. */
export function patchStyle(el: MirrorElement, prev: unknown, next: unknown): void {
  const isCssString = typeof next === "string";
  let hasControlledDisplay = false;
  if (next && !isCssString) {
    const n = next as Record<string, unknown>;
    // A re-render with the same values skips the host; like React, an `el.style` write to one of these keys is
    // re-asserted only when its `:style` value changes.
    if (prev && typeof prev === "object" && sameStyle(prev as Record<string, unknown>, n)) return;
    // Every entry goes to the host in one `__sk_style_patch` string, which writes only what differs.
    let entries = "";
    if (prev) {
      if (typeof prev !== "string") {
        for (const key in prev as Record<string, unknown>) if (n[key] == null) entries += `${key}\0\0`;
      } else {
        for (const prevStyle of prev.split(";")) {
          const key = prevStyle.slice(0, prevStyle.indexOf(":")).trim();
          if (key && n[key] == null) entries += `${key}\0\0`;
        }
      }
    }
    for (const key in n) {
      if (key === "display") hasControlledDisplay = true;
      entries += `${key}\0${styleText(n[key])}\0`;
    }
    __sk_style_patch(el.__skId, entries);
    if (__DEV__) noteWrite(el);
  } else if (isCssString) {
    if (prev !== next) {
      const vars = el._cssVarText;
      __sk_style_replace(el.__skId, vars ? `${next};${vars}` : next, false);
      if (__DEV__) noteWrite(el);
      hasControlledDisplay = DISPLAY_RE.test(next);
    }
  } else if (prev) {
    __sk_style_clear(el.__skId);
    if (__DEV__) noteWrite(el);
  }
  if (vShowOriginalDisplay in el) {
    el[vShowOriginalDisplay] = hasControlledDisplay ? inlineDisplay(el) : "";
    if (el[vShowHidden]) setInlineDisplay(el, "none");
  }
}

function sameStyle(prev: Record<string, unknown>, next: Record<string, unknown>): boolean {
  let count = 0;
  for (const key in next) {
    const a = prev[key];
    const b = next[key];
    if (a !== b && !(Array.isArray(a) && Array.isArray(b) && String(a) === String(b))) return false;
    count++;
  }
  for (const _ in prev) count--;
  return count === 0;
}

/** A `:style` value as `__sk_style_patch` takes it; an array (`{ display: ["-webkit-box", "flex"] }`) is `\x02` then `\x01` before each entry. */
function styleText(v: unknown): string {
  if (v == null) return "";
  if (!Array.isArray(v)) return String(v);
  let out = "\x02";
  for (const x of v.flat(Infinity)) out += x == null ? "\x01" : `\x01${String(x)}`;
  return out;
}
