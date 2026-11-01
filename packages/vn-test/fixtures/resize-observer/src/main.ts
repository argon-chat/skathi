// A fixture for packages/vn-test/specs/resize-observer.spec.ts: ResizeObserver
// delivered where HTML delivers it, after the frame's layout and before its
// paint (crates/vn-tree/src/observe.rs, docs/RUNTIME.md §5.1).
//
// A perpetual requestAnimationFrame loop numbers the frames (`state.frame`).
// rAF runs before the frame's layout, and the devtools `eval` that starts a
// scenario runs after the frame's rAF, in the same wave, so a scenario
// started in frame N has `at = N`.
//
// - `grow`: `#list` gets three more 20 px rows (plain DOM writes, so they are
//   in the tree before the frame lays out). `#panel` takes its height from a
//   ResizeObserver on `#list`, through a Vue ref, so the patch is a microtask
//   of the callback. The callback records the frame it ran in and the height
//   it saw; the first rAF after records the panel's and the list's painted
//   heights: the frame that was presented.
// - `loop`: a ResizeObserver on `#inner` widens `#outer`, its parent, from
//   its callback. `#outer` is shallower than what was just reported, so it is
//   skipped: the loop error, as an `error` event at `window` and through
//   `window.onerror`; another observer on `#outer` hears the new width at the
//   next frame.
//
// Render functions, as in fixtures/layout.
import { createApp, defineComponent, h, onMounted, ref } from "vue-native";

const ROW = 20;

const state = {
  frame: 0,
  grow: null as null | {
    at: number;
    /** The callback: the frame it ran in and the list height it saw. */
    seen: { frame: number; height: number }[];
    /** The first rAF after: the painted heights of the panel and the list. */
    next: { frame: number; panel: number; list: number } | null;
  },
  loop: null as null | { at: number },
  /** `#outer`'s observer: every width it heard, and when. */
  outer: [] as { frame: number; width: number }[],
  /** The loop error, as each route delivered it. */
  errors: [] as { via: string; frame: number; message: string; args?: unknown[] }[],
  run(name: string) {
    if (name === "grow") {
      state.grow = { at: state.frame, seen: [], next: null };
      const list = document.getElementById("list")!;
      for (let i = 0; i < 3; i++) {
        const row = document.createElement("div");
        row.style.height = `${ROW}px`;
        list.appendChild(row);
      }
    } else if (name === "loop") {
      state.loop = { at: state.frame };
      document.getElementById("inner")!.style.width = "150px";
    }
    return state.frame;
  },
};
(globalThis as { __VN_TEST_RO__?: unknown }).__VN_TEST_RO__ = state;

const tick = () => {
  state.frame++;
  const g = state.grow;
  if (g && !g.next && state.frame > g.at) {
    const height = (id: string) => document.getElementById(id)!.getBoundingClientRect().height;
    g.next = { frame: state.frame, panel: height("panel"), list: height("list") };
  }
  requestAnimationFrame(tick);
};
requestAnimationFrame(tick);

window.addEventListener("error", (e) => {
  state.errors.push({ via: "listener", frame: state.frame, message: (e as ErrorEvent).message });
});
window.onerror = (...args: unknown[]) => {
  state.errors.push({ via: "onerror", frame: state.frame, message: String(args[0]), args });
};

const Root = defineComponent({
  setup() {
    const panelHeight = ref(0);
    onMounted(() => {
      new ResizeObserver((entries) => {
        const height = entries[0]!.contentRect.height;
        state.grow?.seen.push({ frame: state.frame, height });
        panelHeight.value = height;
      }).observe(document.getElementById("list")!);

      let armed = true;
      new ResizeObserver(() => {
        if (!state.loop || !armed) return;
        armed = false;
        document.getElementById("outer")!.style.width = "260px";
      }).observe(document.getElementById("inner")!);
      new ResizeObserver((entries) => {
        state.outer.push({ frame: state.frame, width: entries[0]!.contentRect.width });
      }).observe(document.getElementById("outer")!);
    });
    return () =>
      h("div", { id: "root", style: { padding: "10px", background: "white" } }, [
        h(
          "div",
          {
            id: "panel",
            style: { height: `${panelHeight.value}px`, overflow: "hidden", width: "200px", background: "#ccc" },
          },
          [
            h("div", { id: "list", style: { display: "flex", flexDirection: "column" } }, [
              h("div", { class: "row", style: { height: `${ROW}px` } }),
              h("div", { class: "row", style: { height: `${ROW}px` } }),
            ]),
          ],
        ),
        h("div", { id: "outer", style: { width: "300px", height: "40px", marginTop: "10px", background: "#eee" } }, [
          h("div", { id: "inner", style: { width: "100px", height: "20px", background: "#888" } }),
        ]),
      ]);
  },
});

createApp(Root).mount();
