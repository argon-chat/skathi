// CSS paints the popover whole, layers included, before the menu; ours painted the layers over the
// menu's items, and the glass zone took their clicks.
import { createApp, defineComponent, h } from "@skathi-native/runtime";
import "./stacking.css";

const state = {
  clicks: [] as string[],
};
(globalThis as { __SK_TEST_STACKING__?: unknown }).__SK_TEST_STACKING__ = state;

const clicked = (id: string) => () => {
  state.clicks.push(id);
};

const Root = defineComponent({
  setup() {
    return () =>
      h("div", { id: "root" }, [
        h("div", { id: "popover", class: "popover" }, [
          h("div", { class: "popover-inner" }, [
            h("div", { id: "card-bg", class: "card-bg" }),
            h("div", { class: "hero-spacer" }),
            h("div", { id: "glass", class: "glass-zone", onClick: clicked("glass") }),
          ]),
        ]),
        h("div", { id: "menu", class: "menu" }, [
          h("div", { id: "item-1", class: "item", onClick: clicked("item-1") }),
          h("div", { id: "item-2", class: "item", onClick: clicked("item-2") }),
          h("div", { id: "item-3", class: "item", onClick: clicked("item-3") }),
        ]),
        h("div", { id: "neg", class: "neg" }, [h("div", { class: "neg-content" }), h("div", { class: "neg-behind" })]),
        h("div", { class: "flow" }, [
          h("div", { id: "faded", class: "faded" }, [h("div", { class: "lifted" })]),
          h("div", { id: "cover", class: "cover" }),
        ]),
      ]);
  },
});

createApp(Root).mount();
