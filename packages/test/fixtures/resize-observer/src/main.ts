import { createApp, defineComponent, h, onMounted, ref } from "@skathi-native/runtime";

const dom = document as unknown as SkDomDocument;

const ROW = 20;

const state = {
  frame: 0,
  grow: null as null | {
    at: number;
    seen: { frame: number; height: number }[];
    next: { frame: number; panel: number; list: number } | null;
  },
  loop: null as null | { at: number },
  outer: [] as { frame: number; width: number }[],
  errors: [] as { via: string; frame: number; message: string; args?: unknown[] }[],
  run(name: string) {
    if (name === "grow") {
      state.grow = { at: state.frame, seen: [], next: null };
      const list = dom.getElementById("list")!;
      for (let i = 0; i < 3; i++) {
        const row = dom.createElement("div");
        row.style.height = `${ROW}px`;
        list.appendChild(row);
      }
    } else if (name === "loop") {
      state.loop = { at: state.frame };
      dom.getElementById("inner")!.style.width = "150px";
    }
    return state.frame;
  },
};
(globalThis as { __SK_TEST_RO__?: unknown }).__SK_TEST_RO__ = state;

const tick = () => {
  state.frame++;
  const g = state.grow;
  if (g && !g.next && state.frame > g.at) {
    const height = (id: string) => dom.getElementById(id)!.getBoundingClientRect().height;
    g.next = { frame: state.frame, panel: height("panel"), list: height("list") };
  }
  requestAnimationFrame!(tick);
};
requestAnimationFrame!(tick);

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
      }).observe(dom.getElementById("list")!);

      let armed = true;
      new ResizeObserver(() => {
        if (!state.loop || !armed) return;
        armed = false;
        dom.getElementById("outer")!.style.width = "260px";
      }).observe(dom.getElementById("inner")!);
      new ResizeObserver((entries) => {
        state.outer.push({ frame: state.frame, width: entries[0]!.contentRect.width });
      }).observe(dom.getElementById("outer")!);
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
