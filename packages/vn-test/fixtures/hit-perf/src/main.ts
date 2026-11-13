// A counter changes every animation frame, so the window draws every frame; with `VN_NO_CACHE=1`
// every element is prepainted every frame, the worst case.
import { createApp, h, ref } from "vue-native";

const parsed = Number.parseInt((globalThis as unknown as { __VN_SCENARIO__?: string }).__VN_SCENARIO__ ?? "", 10);
const COUNT = Number.isFinite(parsed) && parsed > 0 ? parsed : 600;

const frame = ref(0);
const loop = () => {
  frame.value++;
  requestAnimationFrame(loop);
};
requestAnimationFrame(loop);

const cards = Array.from({ length: COUNT }, (_, i) => i);

const Counter = { render: () => h("div", { id: "frame", style: { height: "20px" } }, `frame ${frame.value}`) };

const App = {
  render: () =>
    h("div", { style: { display: "flex", flexDirection: "column", height: "100%", background: "#fff" } }, [
      h(Counter),
      h(
        "div",
        { style: { display: "flex", flexWrap: "wrap", gap: "2px", overflow: "hidden", flexGrow: "1" } },
        cards.map((i) =>
          h("div", { key: i, class: "card", style: { width: "56px", height: "28px", background: "#eef", display: "flex" } }, [
            h("div", { style: { width: "8px", background: i % 2 ? "#cdf" : "#fdc" } }),
            h("div", { style: { display: "flex", flexDirection: "column" } }, [
              h("span", { style: { fontSize: "9px" } }, `#${i}`),
              h("span", { style: { fontSize: "9px", color: "#666" } }, "card"),
            ]),
          ]),
        ),
      ),
    ]),
};

createApp(App).mount("#app");
