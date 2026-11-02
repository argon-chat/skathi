// A fixture for packages/vn-test/specs/call-timer.spec.ts: Argon's call timer
// in the titlebar (apps/client AppTitlebar.vue's `.voice-timer`), with the
// same markup and inline styles as its two components. That is
// motionCounter/Counter.vue (a 9 px flex row, gap 1, a 2 px pad, clipped) and
// motionCounter/Digit.vue: a cell `width: 1ch` and one font line high that
// clips a strip of 30 digits, resting on the middle run through an inline
// translateY, and rolled with `el.animate()`.
//
// The cell's `1ch` is what hid the timer: the unit was dropped, so every cell
// was 0 px wide. It is half an em of the cell's own 9px now. The root font
// size is 14px, as `@argon/theme` sets it.
//
// The time is @argon/core's startTimer: a 1 s setInterval counting seconds,
// split into minutes and seconds. `__VN_TEST_TIMER__.start()` runs it,
// `set(s)` jumps to a time, and `cells()` reads back each cell's box and the
// digit on show in it.
//
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h, reactive, ref, watch, type VNode } from "vue-native";

document.documentElement.style.fontSize = "14px";

const CELLS = Array.from({ length: 30 }, (_, i) => i % 10);
const REST = 10;
const FONT = 9;
const mod10 = (n: number) => ((n % 10) + 10) % 10;

const interval = reactive({ min: 0, sec: 0 });
let total = 0;
let timer: ReturnType<typeof setInterval> | null = null;
const show = (s: number) => {
  total = s;
  interval.min = Math.floor((s % 3600) / 60);
  interval.sec = s % 60;
};

/** Digit.vue: one place, a clipped strip that rolls to the new digit. */
const Digit = {
  props: { place: Number, value: Number, height: Number },
  setup(props: { place: number; value: number; height: number }) {
    const strip = ref<HTMLElement | null>(null);
    const at = () => Math.floor(props.value / props.place);
    const offset = (cell: number) => `translateY(${-cell * props.height}px)`;
    watch(at, (next, prev) => {
      let step = mod10(next - prev);
      if (step > 5 || (step === 5 && next < prev)) step -= 10;
      if (step === 0 || !strip.value) return;
      const from = REST + mod10(prev);
      strip.value.animate([{ transform: offset(from) }, { transform: offset(from + step) }], {
        duration: 900,
        easing: "cubic-bezier(0.34, 1.56, 0.64, 1)",
      });
    });
    return (): VNode =>
      h(
        "div",
        {
          class: "cell",
          style: { height: `${props.height}px`, position: "relative", width: "1ch", overflow: "hidden", fontVariantNumeric: "tabular-nums" },
        },
        [
          h(
            "div",
            { ref: strip, class: "strip", style: { position: "absolute", top: 0, left: 0, right: 0, transform: offset(REST + mod10(at())) } },
            CELLS.map((n) =>
              h("span", { style: { height: `${props.height}px`, display: "flex", alignItems: "center", justifyContent: "center" } }, String(n)),
            ),
          ),
        ],
      );
  },
};

/** Counter.vue with the titlebar's props: two places, gap 1, 9 px. */
const Counter = {
  props: { value: Number, id: String },
  setup(props: { value: number; id: string }) {
    return (): VNode =>
      h("div", { id: props.id, style: { position: "relative", display: "inline-block" } }, [
        h(
          "div",
          {
            style: {
              fontSize: `${FONT}px`,
              display: "flex",
              gap: "1px",
              overflow: "hidden",
              borderRadius: "4px",
              paddingLeft: "2px",
              paddingRight: "2px",
              lineHeight: 1,
              color: "#9aa0a6",
              fontWeight: "bold",
            },
          },
          [10, 1].map((place) => h(Digit, { key: place, place, value: props.value, height: FONT })),
        ),
      ]);
  },
};

createApp({
  render: () =>
    h("div", { id: "root", style: { padding: "8px", background: "#202225" } }, [
      h("span", { id: "timer", style: { display: "inline-flex", alignItems: "center", gap: "1px" } }, [
        h(Counter, { id: "min", value: interval.min }),
        h("span", { style: { fontSize: "9px", color: "#9aa0a6" } }, ":"),
        h(Counter, { id: "sec", value: interval.sec }),
      ]),
    ]),
}).mount();

(globalThis as Record<string, unknown>).__VN_TEST_TIMER__ = {
  start() {
    if (timer) clearInterval(timer);
    timer = setInterval(() => show(total + 1), 1000);
  },
  stop() {
    if (timer) clearInterval(timer);
    timer = null;
  },
  set: show,
  get seconds() {
    return total;
  },
  /** Each cell's box, and the digit whose span sits in it now. */
  cells() {
    return Array.from(document.querySelectorAll(".cell")).map((cell) => {
      const r = cell.getBoundingClientRect();
      const spans = Array.from(cell.querySelectorAll("span"));
      const inView = spans.find((s) => Math.abs(s.getBoundingClientRect().top - r.top) < 1);
      return { w: r.width, h: r.height, digit: inView?.textContent ?? null };
    });
  },
};
