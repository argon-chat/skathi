import { createApp, h, onMounted } from "@skathi-native/runtime";
import { select } from "d3-selection";
import { type D3ZoomEvent, zoom } from "d3-zoom";

createApp({
  setup() {
    onMounted(() => {
      const map = document.querySelector<HTMLElement>("#map")!;
      select(map).call(
        zoom<HTMLElement, unknown>().on("zoom", (event: D3ZoomEvent<HTMLElement, unknown>) => {
          map.dataset.scale = String(event.transform.k);
          map.style.transform = `scale(${event.transform.k})`;
        }),
      );
    });
    return () =>
      h("div", { id: "scrollbox", style: { height: "220px", overflow: "auto" } }, [
        h("div", { id: "probe", style: { height: "64px", background: "#506890" } }, "Gesture target"),
        h("div", { style: { height: "80px" } }, [
          h("div", { id: "map", style: { width: "160px", height: "64px", background: "#867048" } }, "d3-zoom"),
        ]),
        h("div", { style: { height: "700px", flexShrink: 0 } }),
      ]);
  },
}).mount();
