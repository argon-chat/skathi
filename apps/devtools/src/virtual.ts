// A windowed list that owns its own scrolling. ## Why not `overflow: auto` The engine has real
// scroll containers — `overflow: auto` works, the wheel works, `scroll` events fire.

import { computed, reactive, ref } from "vue";

export interface VirtualList {
  scroll: number;
  readonly total: number;
  readonly start: number;
  readonly end: number;
  /** Where the first rendered row sits, so the slice lines up with the offset. */
  readonly padTop: number;
  readonly atBottom: boolean;
  readonly thumb: { top: number; height: number } | null;
  offsetOf(i: number): number;
  onWheel(event: { deltaY: number }): void;
  ensureVisible(i: number): void;
  toBottom(): void;
  dragThumb(dy: number): void;
}

/**
 * A function so the caller can return a constant-height array cheaply or compute wrapped console
 * lines. @param height the viewport's height in pixels.
 */
export function useVirtual(heights: () => number[], height: () => number): VirtualList {
  const scroll = ref(0);

  const offsets = computed(() => {
    const hs = heights();
    const out = new Array<number>(hs.length + 1);
    out[0] = 0;
    for (let i = 0; i < hs.length; i++) out[i + 1] = out[i]! + hs[i]!;
    return out;
  });

  const total = computed(() => {
    const o = offsets.value;
    return o[o.length - 1] || 0;
  });

  const maxScroll = computed(() => Math.max(0, total.value - height()));
  const clamp = (v: number) => Math.max(0, Math.min(maxScroll.value, v));

  function indexAt(y: number): number {
    const o = offsets.value;
    let lo = 0;
    let hi = o.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (o[mid]! <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  const OVERSCAN = 2;

  const start = computed(() => Math.max(0, indexAt(clamp(scroll.value)) - OVERSCAN));
  const end = computed(() => {
    const n = offsets.value.length - 1;
    return Math.min(n, indexAt(clamp(scroll.value) + height()) + 1 + OVERSCAN);
  });
  const padTop = computed(() => offsets.value[start.value]! - clamp(scroll.value));
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
      // The engine reports wheel deltas in CSS pixels already (sk-tree's `EV_WHEEL` payload is the
      // gpui delta), so there is no line/page mode to decode the way a browser's `deltaMode` would
      // need.
      scroll.value = clamp(scroll.value + (event.deltaY || 0));
    },
    ensureVisible(i: number) {
      const o = offsets.value;
      if (i < 0 || i + 1 >= o.length) return;
      const top = o[i]!;
      const bottom = o[i + 1]!;
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
 * It is exact for ASCII in that font and slightly generous otherwise, which is the right direction
 * to be wrong in — an over-tall row leaves a gap, an under-tall one overlaps the next line.
 */
export function wrappedLines(text: string, width: number, charWidth = 6.62): number {
  const perLine = Math.max(8, Math.floor(width / charWidth));
  let lines = 0;
  for (const line of text.split("\n")) {
    lines += Math.max(1, Math.ceil(line.length / perLine));
  }
  return Math.max(1, lines);
}
