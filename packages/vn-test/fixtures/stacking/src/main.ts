// A fixture for packages/vn-test/specs/stacking.spec.ts: CSS stacking contexts
// painted atomically.
//
// The Argon profile popover and the menu its ⋯ button opens are two portalled
// `fixed z-50` overlays, the popover first. Inside the popover, `.card-bg`
// (`z-index: 0`), `.hero-spacer` (1) and `.glass-zone` (2) each make a stacking
// context. CSS paints the popover whole, layers included, before the menu; ours
// painted the layers over the menu's items, and the glass zone took their
// clicks. Beside it: a negative z-index on an in-flow box, and an `opacity`
// context at z auto with a z-100 box inside it.
//
// Render functions and a `.css` import, as in fixtures/layout.
import { createApp, defineComponent, h } from "vue-native";
import "./stacking.css";

const state = {
  /** Every click, by the id of the box whose listener took it. */
  clicks: [] as string[],
};
(globalThis as { __VN_TEST_STACKING__?: unknown }).__VN_TEST_STACKING__ = state;

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
        h("div", { id: "neg", class: "neg" }, [
          h("div", { class: "neg-content" }),
          h("div", { class: "neg-behind" }),
        ]),
        h("div", { class: "flow" }, [
          h("div", { id: "faded", class: "faded" }, [h("div", { class: "lifted" })]),
          h("div", { id: "cover", class: "cover" }),
        ]),
      ]);
  },
});

createApp(Root).mount();
