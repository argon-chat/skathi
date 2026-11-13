// `stroke-opacity: 0.28` is dropped here so the line reads back as the list's `color` at full
// strength.
import { createApp, h, onMounted, onUnmounted, ref, watch } from "vue-native";
import "./rope.css";

const ROWS = ["564gtr", "4536456", "annonces", "rurune-box", "smazka", "br", "rubka", "5345"];
const LINKS = [{ from: "rubka", to: ["rurune-box", "smazka"] }];

const LANE_X = [3.5, 6.5];
const TICK = 4;

const state = { measured: 0, paths: [] as string[], height: 0, settled: false };
(globalThis as any).__VN_TEST_ROPE__ = state;

const Connectors = {
  props: { container: Object },
  setup(props: { container: HTMLElement | null }) {
    const paths = ref<{ from: string; d: string }[]>([]);
    const height = ref(0);

    function rowCenter(container: HTMLElement, base: DOMRect, id: string): number | null {
      const row = container.querySelector<HTMLElement>(`.channel-item[data-channel-id="${id}"] .channel-inner`);
      if (!row) return null;
      const rect = row.getBoundingClientRect();
      if (rect.height === 0) return null;
      return Math.round(rect.top + rect.height / 2 - base.top + container.scrollTop) + 0.5;
    }

    function measure() {
      const container = props.container;
      if (!container) return;
      const base = container.getBoundingClientRect();
      const next: { from: string; d: string }[] = [];
      LINKS.forEach((link, index) => {
        const hq = rowCenter(container, base, link.from);
        if (hq === null) return;
        const targets = link.to.map((id) => rowCenter(container, base, id)).filter((y): y is number => y !== null);
        if (targets.length === 0) return;
        const x = LANE_X[index % LANE_X.length];
        const ys = [hq, ...targets];
        let d = `M${x} ${Math.min(...ys)} V${Math.max(...ys)}`;
        for (const y of ys) d += ` M${x} ${y} h${TICK}`;
        next.push({ from: link.from, d });
      });
      height.value = container.scrollHeight;
      paths.value = next;
      state.measured++;
      state.paths = next.map((p) => p.d);
      state.height = height.value;
    }

    let frame = 0;
    let settle: ReturnType<typeof setTimeout> | null = null;
    const schedule = () => {
      state.settled = false;
      if (!frame) {
        frame = requestAnimationFrame(() => {
          frame = 0;
          measure();
        });
      }
      if (settle) clearTimeout(settle);
      settle = setTimeout(() => {
        settle = null;
        measure();
        state.settled = true;
      }, 320);
    };
    // The scroller is the parent's template ref, so it arrives after this mounts; the client
    // watches the prop for the same reason.
    let ro: ResizeObserver | null = null;
    const observe = (el: HTMLElement | null | undefined) => {
      ro?.disconnect();
      ro = null;
      if (el) {
        ro = new ResizeObserver(schedule);
        ro.observe(el);
      }
      schedule();
    };
    watch(() => props.container, observe);
    onMounted(() => observe(props.container));
    onUnmounted(() => {
      ro?.disconnect();
      if (settle) clearTimeout(settle);
    });

    return () =>
      paths.value.length > 0
        ? h(
            "svg",
            { id: "rope", class: "broadcast-connectors", "aria-hidden": "true", style: { height: `${height.value}px` } },
            paths.value.map((p) => h("path", { key: p.from, d: p.d, "data-from": p.from })),
          )
        : null;
  },
};

const List = {
  setup() {
    const scrollEl = ref<HTMLElement | null>(null);
    return () =>
      h("div", { id: "list", class: "chat-list" }, [
        h("div", { ref: scrollEl, id: "scroll", class: "chat-list-scroll" }, [
          h(Connectors, { container: scrollEl.value }),
          ...ROWS.map((id) =>
            h("div", { class: "channel-item", "data-channel-id": id }, [h("div", { class: "channel-inner" }, id)]),
          ),
        ]),
      ]);
  },
};

createApp({ render: () => h("div", { id: "root" }, [h(List)]) }).mount();
