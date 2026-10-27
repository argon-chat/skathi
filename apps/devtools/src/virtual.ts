// A windowed list that owns its own scrolling.
//
// ## Why not `overflow: auto`
//
// The engine has real scroll containers — `overflow: auto` works, the wheel
// works, `scroll` events fire. When this was written it had no way for JS to
// MOVE one: `element.scrollIntoView()` was an advisory no-op and `scrollTop`
// was effectively read-only from the app's side. **m23 fixed that** (`scrollTop`
// is writable, `scrollTo`/`scrollBy`/`scrollIntoView()` are real — see
// crates/vn-tree/src/scroll.rs), so this list could now be rewritten on a
// native container. It has not been: the other two things it buys below (a
// 10 000-node tree costing what 50 do, and a thumb styled to match the panel)
// are unaffected by that, and the rewrite would change how the inspector
// behaves for no visible gain. The original argument, for the record — the two
// lists that matter here are exactly the two that have to move themselves:
//
//   * the element tree — arrow-key navigation is useless if the row you just
//     selected is off-screen, and so is "the picker chose this node";
//   * the console — a log that does not follow the newest line is a log you
//     scroll by hand forever, which is why v1 reversed itself and showed the
//     newest FIRST. That was the honest fix available at the time. This is the
//     one that reads like a console.
//
// So the list keeps its own offset, renders only the slice in view, and draws
// its own scrollbar. It gets three things for free that the native container
// could not have given: scroll-to-index, a tree of 10 000 nodes costing the
// same as a tree of 50, and a thumb we can style to match.
//
// The viewport height is arithmetic rather than measurement — see `layout.ts`
// for why that is sound here and what it costs.

import { computed, reactive, ref } from "vue";

export interface VirtualList {
  /** Pixels scrolled from the top. Assignable. */
  scroll: number;
  /** Total content height. */
  readonly total: number;
  /** First rendered index, and one past the last. */
  readonly start: number;
  readonly end: number;
  /** Where the first rendered row sits, so the slice lines up with the offset. */
  readonly padTop: number;
  /** Is the offset at the bottom right now? */
  readonly atBottom: boolean;
  /** The scrollbar thumb, or `null` when everything fits. */
  readonly thumb: { top: number; height: number } | null;
  /** The top of row `i` in content space. */
  offsetOf(i: number): number;
  /** Wheel handler for the viewport. */
  onWheel(event: { deltaY: number }): void;
  /** Scrolls the least amount that puts row `i` fully in view. */
  ensureVisible(i: number): void;
  /** Pin to the bottom (a console that follows the newest line). */
  toBottom(): void;
  /** Drag the thumb by `dy` pixels of SCREEN movement. */
  dragThumb(dy: number): void;
}

/**
 * @param heights per-row heights, oldest first. A function so the caller can
 *   return a constant-height array cheaply or compute wrapped console lines.
 * @param height  the viewport's height in pixels.
 */
export function useVirtual(heights: () => number[], height: () => number): VirtualList {
  const scroll = ref(0);

  /** Running offsets: `offsets[i]` is the top of row `i`, `offsets[n]` the end. */
  const offsets = computed(() => {
    const hs = heights();
    const out = new Array<number>(hs.length + 1);
    out[0] = 0;
    for (let i = 0; i < hs.length; i++) out[i + 1] = out[i] + hs[i];
    return out;
  });

  const total = computed(() => {
    const o = offsets.value;
    return o[o.length - 1] || 0;
  });

  const maxScroll = computed(() => Math.max(0, total.value - height()));
  const clamp = (v: number) => Math.max(0, Math.min(maxScroll.value, v));

  /** Binary search for the row containing `y`. */
  function indexAt(y: number): number {
    const o = offsets.value;
    let lo = 0;
    let hi = o.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (o[mid] <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  // Two rows of overscan above and below: a wheel event lands between frames,
  // and a slice with no margin flickers an empty strip at the leading edge.
  const OVERSCAN = 2;

  const start = computed(() => Math.max(0, indexAt(clamp(scroll.value)) - OVERSCAN));
  const end = computed(() => {
    const n = offsets.value.length - 1;
    return Math.min(n, indexAt(clamp(scroll.value) + height()) + 1 + OVERSCAN);
  });
  const padTop = computed(() => offsets.value[start.value] - clamp(scroll.value));
  const atBottom = computed(() => scroll.value >= maxScroll.value - 1);
  const thumb = computed(() => {
    const view = height();
    if (total.value <= view || view <= 0) return null;
    // A thumb shorter than this cannot be grabbed.
    const h = Math.max(24, (view * view) / total.value);
    const travel = view - h;
    const top = maxScroll.value > 0 ? (clamp(scroll.value) / maxScroll.value) * travel : 0;
    return { top, height: h };
  });

  // `reactive`, not a bare object: it unwraps the refs, so a template can write
  // `list.start` rather than `list.start.value` (template unwrapping only
  // applies to top-level setup bindings, and this is a property of one).
  //
  // The double cast is `reactive`'s own typing, not a shortcut around it:
  // `UnwrapNestedRefs` gives every field the right value type, but the
  // compiler cannot see that through the object literal's `Ref<T>` fields, and
  // `VirtualList` is the shape a CALLER sees. Written once, here.
  return reactive({
    scroll,
    total,
    start,
    end,
    padTop,
    atBottom,
    thumb,
    offsetOf: (i: number) => offsets.value[Math.max(0, Math.min(i, offsets.value.length - 1))],
    onWheel(event: { deltaY: number }) {
      // The engine reports wheel deltas in CSS pixels already (vn-tree's
      // `EV_WHEEL` payload is the gpui delta), so there is no line/page mode to
      // decode the way a browser's `deltaMode` would need.
      scroll.value = clamp(scroll.value + (event.deltaY || 0));
    },
    ensureVisible(i: number) {
      const o = offsets.value;
      if (i < 0 || i + 1 >= o.length) return;
      const top = o[i];
      const bottom = o[i + 1];
      const view = height();
      if (top < scroll.value) scroll.value = clamp(top);
      else if (bottom > scroll.value + view) scroll.value = clamp(bottom - view);
    },
    toBottom() {
      scroll.value = maxScroll.value;
    },
    dragThumb(dy: number) {
      const view = height();
      if (total.value <= view) return;
      const h = Math.max(24, (view * view) / total.value);
      const travel = view - h;
      if (travel <= 0) return;
      scroll.value = clamp(scroll.value + (dy / travel) * maxScroll.value);
    },
  }) as unknown as VirtualList;
}

/**
 * How many lines a string occupies at `width` pixels in the console's
 * monospace face.
 *
 * An ESTIMATE, and it says so: nothing in this runtime can measure text yet, so
 * this divides by an average advance width for the 11 px mono face the console
 * uses. It is exact for ASCII in that font and slightly generous otherwise,
 * which is the right direction to be wrong in — an over-tall row leaves a gap,
 * an under-tall one overlaps the next line.
 */
export function wrappedLines(text: string, width: number, charWidth = 6.62): number {
  const perLine = Math.max(8, Math.floor(width / charWidth));
  let lines = 0;
  for (const line of text.split("\n")) {
    lines += Math.max(1, Math.ceil(line.length / perLine));
  }
  return Math.max(1, lines);
}
