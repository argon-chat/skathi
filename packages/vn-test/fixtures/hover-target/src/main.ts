// A fixture for packages/vn-test/specs/hover-target.spec.ts (m26, docs/RUNTIME.md
// §5.18): `:hover`, `group-hover` and `mouseenter` / `mouseleave` follow the
// topmost element under the pointer and its ancestors, resolved once a frame,
// as in a browser. gpui's own hover followed every hitbox under the pointer.
//
// * `#under` has a `:hover` rule; `#cover`, painted over its top half, listens
//   for nothing. Over the cover, `#under` is not hovered.
// * `#a-wrap > #a` and `#b-wrap > #b`: moving from `#a` to `#b` is two
//   `mouseleave`s, innermost first, then two `mouseenter`s, outermost first.
// * `#menu` is a dropdown trigger with a `:hover` rule, and `#sub` its panel,
//   laid out below it, outside its box. Over the panel the trigger keeps its
//   hover and hears no `mouseleave`.
// * `#grp` is a group whose `.dot` has a `#grp:hover .dot` rule; `#grp-cover`
//   covers its right half.
// * `#inert` is `pointer-events: none` with a `:hover` rule, around `#live`,
//   which can be hit. Over `#live`, `#inert` is hovered.
//
// `__VN_TEST_HOVER__.log` records `<event>:<listener id>`.
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
