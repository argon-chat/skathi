import { createApp, defineComponent, h } from "vue-native";
import "./hover-target.css";

const state = { log: [] as string[] };
(globalThis as { __VN_TEST_HOVER__?: unknown }).__VN_TEST_HOVER__ = state;

const watch = (id: string) => ({
  id,
  onMouseenter: () => state.log.push(`mouseenter:${id}`),
  onMouseleave: () => state.log.push(`mouseleave:${id}`),
});

const Root = defineComponent({
  setup() {
    return () =>
      h("div", { id: "page" }, [
        h("div", { ...watch("under"), class: "box" }),
        h("div", { id: "cover", class: "box" }),
        h("div", { ...watch("a-wrap"), class: "box wrap" }, [h("div", { ...watch("a"), class: "leaf" })]),
        h("div", { ...watch("b-wrap"), class: "box wrap" }, [h("div", { ...watch("b"), class: "leaf" })]),
        h("div", { ...watch("menu"), class: "box" }, [h("div", { ...watch("sub") })]),
        h("div", { id: "grp", class: "box" }, [h("div", { id: "dot", class: "dot" })]),
        h("div", { id: "grp-cover", class: "box" }),
        h("div", { ...watch("inert"), class: "box" }, [h("div", { id: "live" })]),
      ]);
  },
});

createApp(Root).mount("#app");
