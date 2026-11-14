// Ours answered the layout box, so the Argon profile popover's three-dot menu opened in the
// window's top-left corner.
import { createApp, defineComponent, h, onMounted, ref } from "@skathi-native/runtime";
import "./transform.css";

const dom = document as unknown as SkDomDocument;

type Rect = [number, number, number, number];
const rect = (el: Element | null): Rect | null => {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return [r.left, r.top, r.width, r.height].map((v) => Math.round(v * 10) / 10) as Rect;
};

const state = {
  anchors: {} as Record<string, Rect | null>,
  zoomResizes: 0,
  io: {} as Record<string, { isIntersecting: boolean; rect: Rect }>,
  startZoom: () => {},
  sampleZoom: () => {
    const el = dom.getElementById("zoom") as HTMLElement;
    return {
      offsetWidth: el.offsetWidth,
      clientWidth: el.clientWidth,
      rect: rect(el),
      clientRectWidth: el.getClientRects()[0]?.width ?? 0,
    };
  },
};
(globalThis as { __SK_TEST_TRANSFORM__?: unknown }).__SK_TEST_TRANSFORM__ = state;

const Popper = defineComponent({
  props: { id: { type: String, required: true }, anchor: { type: String, required: true } },
  setup(props, { slots }) {
    const at = ref<[number, number] | null>(null);
    onMounted(() => {
      const anchor = rect(dom.getElementById(props.anchor));
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
      }).observe(dom.getElementById("zoom")!);
      const io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          const r = e.boundingClientRect;
          state.io[(e.target as HTMLElement).id] = {
            isIntersecting: e.isIntersecting,
            rect: [r.left, r.top, r.width, r.height],
          };
        }
      });
      io.observe(dom.getElementById("far")!);
      io.observe(dom.getElementById("near")!);
    });
    return () =>
      h("div", { id: "root" }, [
        h("div", { id: "trigger", class: "trigger" }),
        h(Popper, { id: "pop", anchor: "trigger" }, () =>
          h("div", { class: "pop-body" }, [
            h("div", { id: "menu-btn", class: "menu-btn", onClick: () => (menuOpen.value = !menuOpen.value) }),
          ]),
        ),
        menuOpen.value ? h(Popper, { id: "menu", anchor: "menu-btn" }, () => h("div", { class: "menu-body" })) : null,
        h("div", { id: "dialog", class: "dialog" }, [
          h("div", {
            id: "select-trigger",
            class: "select-trigger",
            onClick: () => (selectOpen.value = !selectOpen.value),
          }),
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
