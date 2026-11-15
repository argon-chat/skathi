import { reactive } from "vue";

/** The effects drawn over the whole window, by components/FullWindow.vue. */
export const fullWindow = reactive({
  snow: false,
  fireflies: false,
  /** 0 intact to 1 fallen apart; null while no shatter runs. */
  shatter: null as number | null,
  /** Where the crack starts, in client pixels. */
  impact: { x: 0, y: 0 },
});

const APART_MS = 1300;
const HOLD_MS = 600;
const BACK_MS = 1300;

/** Breaks the window from the point (x, y) and puts it back together. */
export function shatter(x: number, y: number) {
  if (fullWindow.shatter !== null) return;
  fullWindow.impact = { x, y };
  fullWindow.shatter = 0;
  let start = -1;
  const step = (now: number) => {
    if (start < 0) start = now;
    const t = now - start;
    if (t >= APART_MS + HOLD_MS + BACK_MS) {
      fullWindow.shatter = null;
      return;
    }
    fullWindow.shatter = t < APART_MS ? t / APART_MS : Math.min(1, 1 - (t - APART_MS - HOLD_MS) / BACK_MS);
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}
