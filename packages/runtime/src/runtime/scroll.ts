import type { MirrorElement } from "./types";
import { warnCompat } from "../compat";
import { defineDom, defineDomGet } from "./dom-members";

// Programmatic scrolling drives the host's real `ScrollHandle`. A write is CLAMPED and the
// getter answers the clamped value synchronously; the `scroll` EVENT is asynchronous, as in CSSOM-View.

export interface ScrollPos {
  left: number;
  top: number;
  scrollWidth: number;
  scrollHeight: number;
  stale: boolean;
}

/** One host call for every scroll accessor; `null` when not a scroll container. `sizeMatters` enables the `noteStale` advisory. */
export function hostScroll(
  node: MirrorElement,
  op: 0 | 1 | 2,
  x = NaN,
  y = NaN,
  sizeMatters = false,
): ScrollPos | null {
  if (!node.__skId) return null;
  const raw = __sk_scroll(node.__skId, op, x, y);
  if (!raw) return null;
  const [left = NaN, top = NaN, scrollWidth = NaN, scrollHeight = NaN, stale] = raw.split(",").map(Number);
  if (stale && sizeMatters) noteStale();
  return { left, top, scrollWidth, scrollHeight, stale: !!stale };
}

/**
 * The host forces layout on read wherever a draw is legal, but not inside handlers gpui calls directly (keydown/keyup,
 * mousedown, wheel, clipboard, focus/blur: `sk_tree::in_input_dispatch`). There a content-size read in the same JS
 * wave as a DOM change answers the last painted frame.
 */
function noteStale(): void {
  warnCompat(
    "scroll-layout",
    "scrollWidth/scrollHeight (and the clamp on a scroll) answered from the LAST PAINTED frame: " +
      "the DOM changed in this same wave, and a layout cannot be forced inside a keydown/keyup " +
      "(or the click Enter/Space makes), mousedown, wheel, clipboard or focus/blur handler here " +
      "(it can everywhere else: timers, rAF, fetch callbacks, mouseup/click, input). " +
      "`el.scrollTop = el.scrollHeight` right after adding content from such a handler therefore " +
      "stops one item short — do it inside requestAnimationFrame(), which runs after the frame " +
      "that laid the new content out",
  );
}

/** `behavior: "smooth"` is performed instantly, with an advisory: a running CSS animation switches the retained-render cache off document-wide. */
function noteBehavior(behavior: unknown): void {
  if (behavior !== "smooth") return;
  warnCompat(
    "scroll-behavior",
    'scroll behavior: "smooth" is performed INSTANTLY — the final position is exact and readable ' +
      "on the next line, but there is no interpolation and no `scrollend` event; " +
      'use behavior: "auto" (or "instant") to say so explicitly',
  );
}

function scrollArgs(a: unknown, b: unknown): { left: number; top: number } {
  if (a !== null && typeof a === "object") {
    const o = a as { left?: unknown; top?: unknown; behavior?: unknown };
    noteBehavior(o.behavior);
    return { left: o.left === undefined ? NaN : Number(o.left), top: o.top === undefined ? NaN : Number(o.top) };
  }
  return { left: a === undefined ? NaN : Number(a), top: b === undefined ? NaN : Number(b) };
}

for (const [name, axis] of [
  ["scrollLeft", "left"],
  ["scrollTop", "top"],
] as [string, "left" | "top"][]) {
  defineDomGet(
    name,
    function (this: MirrorElement) {
      const s = hostScroll(this, 0);
      return s ? s[axis] : 0;
    },
    function (this: MirrorElement, value: never) {
      const v = Number(value);
      if (!Number.isFinite(v)) return;
      // The write is clamped against the content size, so a stale size lands stale.
      hostScroll(this, 1, axis === "left" ? v : NaN, axis === "top" ? v : NaN, true);
    },
  );
}

const scrollToImpl = function (this: MirrorElement, a: unknown, b: unknown) {
  const { left, top } = scrollArgs(a, b);
  hostScroll(this, 1, left, top, true);
};
defineDom("scrollTo", scrollToImpl);
defineDom("scroll", scrollToImpl);
defineDom("scrollBy", function (this: MirrorElement, a: unknown, b: unknown) {
  const { left, top } = scrollArgs(a, b);
  hostScroll(this, 2, Number.isFinite(left) ? left : 0, Number.isFinite(top) ? top : 0, true);
});

const SCROLL_ALIGN: Record<string, SkScrollAlign> = {
  start: 0,
  center: 1,
  end: 2,
  nearest: 3,
};

defineDom("scrollIntoView", function (this: MirrorElement, arg: unknown) {
  if (!this.__skId) return;
  let block: SkScrollAlign = 0;
  let inline: SkScrollAlign = 3;
  if (arg === false) block = 2;
  else if (arg !== null && typeof arg === "object") {
    const o = arg as { block?: unknown; inline?: unknown; behavior?: unknown };
    noteBehavior(o.behavior);
    if (o.block !== undefined) block = SCROLL_ALIGN[String(o.block)] ?? 0;
    if (o.inline !== undefined) inline = SCROLL_ALIGN[String(o.inline)] ?? 3;
  }
  // Only horizontal-tb ltr exists: block is vertical, inline horizontal (`direction: rtl` would not flip it).
  const out = __sk_scroll_into_view(this.__skId, block, inline);
  if (out.charCodeAt(0) === 33 /* ! */) warnCompat("scrollIntoView", out.slice(1));
  else if (out === "stale") noteStale();
});
