import { createApp, defineComponent, h } from "@skathi-native/runtime";
import "./style.css";

createApp(
  defineComponent({
    setup() {
      return () =>
        h("div", [
          h("div", { id: "bounds" }, [h("div", { id: "sheet" }), h("div", { id: "limits" })]),
          h("div", { id: "fast", class: "fade" }),
          h("div", { id: "slow", class: "fade" }),
          h("button", { id: "prior" }, "Focus"),
          h("div", { id: "card", class: "card scoped", tabindex: 0 }, [
            h("div", { id: "poster", class: "poster scoped" }),
          ]),
          h("div", { id: "stack" }, [h("div", { id: "relative" }), h("div", { id: "later" })]),
          h("div", { id: "grid" }, [h("div", { id: "gridcap" })]),
          h("div", { id: "absparent" }, [h("div", { id: "abscap" })]),
          h("svg", { id: "size-svg", width: 160, height: 20, viewBox: "0 0 80 10" }, [
            h("rect", { width: 80, height: 10, fill: "#fff" }),
            h("rect", { id: "svgcap", height: 10, fill: "#00ff00" }),
          ]),
        ]);
    },
  }),
).mount();
