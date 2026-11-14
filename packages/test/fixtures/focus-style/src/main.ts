// It paints nothing, but its CSS `visibility` is `visible`, so it takes focus.
import { createApp, defineComponent, h, nextTick, onMounted, ref, Teleport } from "@skathi-native/runtime";

const dom = document as unknown as SkDomDocument;

type Out = { log: string[]; active: string[] };
const out: Out = { log: [], active: [] };
(globalThis as unknown as { __SK_TEST_FOCUS_STYLE__: Out }).__SK_TEST_FOCUS_STYLE__ = out;

const idOf = (t: unknown) => (t as { id?: string } | null)?.id || "-";
dom.addEventListener("focusin", (e) => out.log.push(`focusin t=${idOf(e.target)}`));

const tonesOpen = ref(false);

const Tones = defineComponent({
  setup() {
    const root = ref<HTMLElement | null>(null);
    const position = ref({ left: "0px", top: "0px", visibility: "hidden" as "hidden" | "visible" });
    onMounted(async () => {
      await nextTick();
      // place(): a reactive write, so the menu is still `visibility: hidden` when focus() runs on
      // the next line.
      position.value = { left: "24px", top: "220px", visibility: "visible" };
      root.value?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus({ preventScroll: true });
      out.active.push(idOf(dom.activeElement));
    });
    return () =>
      h(Teleport, { to: "body" }, [
        h(
          "div",
          {
            ref: root,
            id: "tones",
            role: "menu",
            style: {
              position: "fixed",
              display: "flex",
              gap: "4px",
              padding: "4px",
              background: "#dde",
              ...position.value,
            },
          },
          [0, 1, 2].map((i) =>
            h(
              "button",
              {
                id: `tone-${i}`,
                role: "menuitemradio",
                "aria-checked": i === 1 ? "true" : "false",
                style: { width: "36px", height: "36px" },
              },
              String(i),
            ),
          ),
        ),
      ]);
  },
});

const srOnly = {
  position: "absolute",
  width: "1px",
  height: "1px",
  padding: "0",
  margin: "-1px",
  overflow: "hidden",
  clip: "rect(0, 0, 0, 0)",
  whiteSpace: "nowrap",
  borderWidth: "0",
};

const App = {
  render: () =>
    h(
      "div",
      {
        id: "page",
        style: {
          position: "relative",
          display: "flex",
          flexDirection: "column",
          gap: "8px",
          padding: "16px",
          background: "#fff",
          height: "100%",
        },
      },
      [
        h(
          "button",
          {
            id: "opener",
            style: { width: "120px", height: "32px" },
            onClick: () => (tonesOpen.value = !tonesOpen.value),
          },
          "tones",
        ),
        h("button", { id: "other", style: { width: "120px", height: "32px" } }, "other"),
        h("div", { id: "hbox", style: { visibility: "hidden", display: "flex", gap: "4px" } }, [
          h("button", { id: "hchild", style: { width: "120px", height: "32px" } }, "hidden"),
        ]),
        h("div", { id: "nbox", style: { display: "none" } }, [h("button", { id: "nchild" }, "gone")]),
        h("input", { id: "sr", style: srOnly }),
        tonesOpen.value ? h(Tones) : null,
      ],
    ),
};

createApp(App).mount("#app");
