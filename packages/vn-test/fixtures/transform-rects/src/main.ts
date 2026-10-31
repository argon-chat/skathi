// A fixture for packages/vn-test/specs/transform-rects.spec.ts:
// getBoundingClientRect() under CSS transforms.
//
// reka's PopperContent lays its wrapper out at (0, 0), `position: fixed`, and
// moves it with the `transform: translate(x, y)` floating-ui computes; shadcn's
// DialogContent is centred with `translate(-50%, -50%)`. Anything anchored to
// an element inside either reads that element's getBoundingClientRect(), which
// a browser answers with transforms applied. Ours answered the layout box, so
// the Argon profile popover's three-dot menu opened in the window's top-left
// corner. `Popper` below does what PopperContent does for `side: bottom,
// align: start`: measure the anchor once mounted, write the translate.
//
// Render functions and a `.css` import, as in fixtures/layout.
import { createApp, defineComponent, h, onMounted, ref } from "vue-native";
import "./transform.css";

type Rect = [number, number, number, number];
const rect = (el: Element | null): Rect | null => {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return [r.left, r.top, r.width, r.height].map((v) => Math.round(v * 10) / 10) as Rect;
};

const state = {
  /** Each popper's anchor, as measured when it was placed. */
  anchors: {} as Record<string, Rect | null>,
  /** ResizeObserver entries for `#zoom`, after the first. */
  zoomResizes: 0,
  /** IntersectionObserver: the last entry per target id. */
  io: {} as Record<string, { isIntersecting: boolean; rect: Rect }>,
  startZoom: () => {},
  sampleZoom: () => {
    const el = document.getElementById("zoom") as HTMLElement;
    return {
      offsetWidth: el.offsetWidth,
      clientWidth: el.clientWidth,
      rect: rect(el),
      clientRectWidth: el.getClientRects()[0]?.width ?? 0,
    };
  },
};
(globalThis as { __VN_TEST_TRANSFORM__?: unknown }).__VN_TEST_TRANSFORM__ = state;

const Popper = defineComponent({
  props: { id: { type: String, required: true }, anchor: { type: String, required: true } },
  setup(props, { slots }) {
    const at = ref<[number, number] | null>(null);
    onMounted(() => {
      const anchor = rect(document.getElementById(props.anchor));
      state.anchors[props.id] = anchor;
      if (anchor) at.value = [anchor[0], anchor[1] + anchor[3] + 4];
    });
    return () =>
      h(
        "div",
        {
          id: props.id,
          class: "popper",
          style: { transform: at.value ? `translate(${at.value[0]}px, ${at.value[1]}px)` : "translate(0px, -2000px)" },
        },
        slots.default?.(),
      );
  },
});

const menuOpen = ref(false);
const selectOpen = ref(false);
const zooming = ref(false);
state.startZoom = () => {
  zooming.value = true;
};

const Root = defineComponent({
  setup() {
    onMounted(() => {
      let first = true;
      new ResizeObserver(() => {
        if (first) first = false;
        else state.zoomResizes++;
      }).observe(document.getElementById("zoom")!);
      const io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          const r = e.boundingClientRect;
          state.io[(e.target as HTMLElement).id] = { isIntersecting: e.isIntersecting, rect: [r.left, r.top, r.width, r.height] };
        }
      });
      io.observe(document.getElementById("far")!);
      io.observe(document.getElementById("near")!);
    });
    return () =>
      h("div", { id: "root" }, [
        h("div", { id: "trigger", class: "trigger" }),
        // The popover, open: a wrapper moved under the trigger, with the menu
        // button inside.
        h(Popper, { id: "pop", anchor: "trigger" }, () =>
          h("div", { class: "pop-body" }, [
            h("div", { id: "menu-btn", class: "menu-btn", onClick: () => (menuOpen.value = !menuOpen.value) }),
          ]),
        ),
        // The dropdown the menu button opens: a second popper anchored to an
        // element inside the first.
        menuOpen.value ? h(Popper, { id: "menu", anchor: "menu-btn" }, () => h("div", { class: "menu-body" })) : null,
        h("div", { id: "dialog", class: "dialog" }, [
          h("div", { id: "select-trigger", class: "select-trigger", onClick: () => (selectOpen.value = !selectOpen.value) }),
        ]),
        selectOpen.value
          ? h(Popper, { id: "select", anchor: "select-trigger" }, () => h("div", { class: "select-body" }))
          : null,
        h("div", { id: "zoom", class: zooming.value ? "zoom zooming" : "zoom" }),
        h("div", { id: "far", class: "far" }),
        h("div", { id: "near", class: "near" }),
      ]);
  },
});

createApp(Root).mount();
