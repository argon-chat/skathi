import { createApp, h, ref } from "vue-native";

const state = { log: [] as string[] };
(globalThis as unknown as { __VN_TEST_HIT__: unknown }).__VN_TEST_HIT__ = state;

const idOf = (e: Event) => (e.target as HTMLElement | null)?.id || (e.target as HTMLElement | null)?.className || "?";
const note = (what: string) => (e: Event) => state.log.push(`${e.type}:${what} t=${idOf(e)}`);

document.addEventListener("mousedown", (e) => state.log.push(`mousedown:document t=${idOf(e)}`));

const abs = (left: number, top: number, width: number, height: number, extra: Record<string, string> = {}) => ({
  position: "absolute",
  left: `${left}px`,
  top: `${top}px`,
  width: `${width}px`,
  height: `${height}px`,
  ...extra,
});

const tick = ref(0);
setInterval(() => tick.value++, 100);

const App = {
  render: () =>
    h("div", { id: "page", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      h("div", {
        id: "backdrop",
        style: abs(10, 10, 150, 90, { background: "#ddd" }),
        onMousedown: note("backdrop"),
        onClick: note("backdrop"),
      }),
      h("div", { id: "card", style: abs(30, 30, 110, 50, { background: "#fff" }) }, [
        h("span", { id: "card-text" }, "card"),
      ]),
      h(
        "div",
        {
          id: "overlay",
          style: abs(170, 10, 150, 90, { background: "#ccd" }),
          onClick: (e: MouseEvent) => {
            if (e.target === e.currentTarget) note("overlay-self")(e);
          },
        },
        [h("div", { id: "pic", style: { width: "40px", height: "40px", margin: "25px", background: "#88a" } })],
      ),
      h("button", { id: "btn", style: abs(330, 10, 120, 40), onClick: note("btn") }, [
        h("span", { id: "btn-label" }, "Label"),
      ]),
      h("div", { id: "under", style: abs(10, 110, 150, 60, { background: "#dfd" }), onClick: note("under") }),
      h(
        "div",
        { id: "decor", style: abs(10, 110, 150, 60, { pointerEvents: "none", background: "rgba(0,0,0,0.1)" }) },
        [h("div", { id: "decor-auto", style: abs(110, 0, 40, 60, { pointerEvents: "auto", background: "#afa" }) })],
      ),
      h("div", { id: "parent", style: abs(170, 110, 150, 30, { background: "#fdd" }), onClick: note("parent") }, [
        h("div", { id: "outside", style: abs(0, 40, 150, 20, { background: "#faa" }) }),
      ]),
      h("input", { id: "field", value: "field", style: abs(330, 110, 120, 28) }),
      h("div", { id: "field-cover", style: abs(330, 110, 120, 28, { background: "rgba(0,0,255,0.15)" }) }),
      h(
        "div",
        { id: "list", style: abs(10, 190, 150, 80, { overflow: "auto", background: "#eef" }) },
        Array.from({ length: 12 }, (_, i) =>
          h("div", { id: `line${i + 1}`, style: { height: "24px" } }, `Line ${i + 1}`),
        ),
      ),
      h("div", { id: "list-cover", style: abs(10, 190, 150, 30, { background: "rgba(255,0,0,0.2)" }) }),
      h("div", { id: "row", style: abs(170, 190, 150, 40, { display: "flex" }), onClick: note("row") }, [
        h("div", { id: "ra", style: { width: "75px", height: "40px", background: "#ffd" } }),
        h("div", { id: "rb", style: { width: "75px", height: "40px", background: "#dff" } }),
      ]),
      h(
        "div",
        { id: "clist", style: abs(330, 190, 150, 160) },
        Array.from({ length: 4 }, (_, i) =>
          h(
            "div",
            {
              class: "crow",
              id: `crow${i}`,
              style: { height: "36px", marginBottom: "4px" },
              onClick: note(`crow${i}`),
            },
            [
              h("div", {
                class: "c-under",
                id: `cu${i}`,
                style: { height: "20px", background: "#cfc" },
                onClick: note(`cu${i}`),
              }),
              h("div", {
                class: "c-cover",
                id: `cc${i}`,
                style: { height: "20px", marginTop: "-20px", background: "#9c9" },
              }),
              h("div", { class: "c-foot", style: { height: "16px", display: "flex" } }, [
                h("span", { style: { width: "40px" } }, "a"),
                h("span", { style: { width: "40px" } }, "b"),
              ]),
            ],
          ),
        ),
      ),
      h("div", { id: "tick", style: abs(10, 370, 200, 20) }, `tick ${tick.value}`),
    ]),
};

createApp(App).mount("#app");
