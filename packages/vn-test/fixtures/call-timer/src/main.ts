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
          style: {
            height: `${props.height}px`,
            position: "relative",
            width: "1ch",
            overflow: "hidden",
            fontVariantNumeric: "tabular-nums",
          },
        },
        [
          h(
            "div",
            {
              ref: strip,
              class: "strip",
              style: { position: "absolute", top: 0, left: 0, right: 0, transform: offset(REST + mod10(at())) },
            },
            CELLS.map((n) =>
              h(
                "span",
                {
                  style: {
                    height: `${props.height}px`,
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                  },
                },
                String(n),
              ),
            ),
          ),
        ],
      );
  },
};

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
  cells() {
    return Array.from(document.querySelectorAll(".cell")).map((cell) => {
      const r = cell.getBoundingClientRect();
      const spans = Array.from(cell.querySelectorAll("span"));
      const inView = spans.find((s) => Math.abs(s.getBoundingClientRect().top - r.top) < 1);
      return { w: r.width, h: r.height, digit: inView?.textContent ?? null };
    });
  },
};
