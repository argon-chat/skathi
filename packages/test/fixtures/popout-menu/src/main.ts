import { createApp, h, reactive, SkWindow, type SkWindowTarget } from "@skathi-native/runtime";

type Rect = { x: number; y: number; width: number; height: number };
type Mapper = { mapFromMain(target: SkWindowTarget | null): Rect | null };

const S = reactive({
  menuOpen: true,
  visible: false,
  bounds: { x: 120, y: 160, width: 160, height: 120 },
  picked: "",
  clicks: 0,
  hovered: "",
  blurs: 0,
  control: false,
});
let menu: Mapper | null = null;
let anchor: SkWindowTarget | null = null;

window.addEventListener("blur", () => S.blurs++);

/** Puts the menu's top-left at the anchor's bottom-left and shows it. */
function placeBelow(): Rect | null {
  const r = menu?.mapFromMain(anchor) ?? null;
  if (!r) return null;
  S.bounds = { ...S.bounds, x: S.bounds.x + r.x, y: S.bounds.y + r.y + r.height };
  S.visible = true;
  return r;
}

const item = (name: string) =>
  h(
    "div",
    {
      class: `item ${name}`,
      style: { height: "32px", padding: "0 12px", background: S.hovered === name ? "#0090ff" : "#2a2a2a" },
      onMouseenter: () => (S.hovered = name),
      onClick: () => {
        S.picked = name;
        S.clicks++;
      },
    },
    name,
  );

createApp({
  setup() {
    return () =>
      h("div", { id: "app", style: { padding: "12px" } }, [
        h(
          "div",
          {
            id: "anchor",
            ref: (el: unknown) => (anchor = el as SkWindowTarget | null),
            style: { marginLeft: "60px", marginTop: "40px", width: "100px", height: "24px", background: "#444" },
          },
          "anchor",
        ),
        S.menuOpen
          ? h(
              SkWindow,
              {
                ref: (r: unknown) => (menu = r as Mapper | null),
                title: "Menu",
                chrome: "hidden",
                backdrop: "transparent",
                resizable: false,
                focusable: false,
                taskbar: false,
                visible: S.visible,
                bounds: S.bounds,
              },
              () =>
                h("div", { id: "menu-root", style: { position: "absolute", inset: "0" } }, [item("one"), item("two")]),
            )
          : null,
        S.control
          ? h(SkWindow, { title: "Control", bounds: { x: 420, y: 160, width: 160, height: 100 } }, () =>
              h("div", { class: "ctl", style: { width: "100%", height: "100%" } }, "control"),
            )
          : null,
      ]);
  },
}).mount();

(globalThis as Record<string, unknown>).__SK_TEST_MENU__ = {
  S,
  placeBelow,
  map: () => menu?.mapFromMain(anchor) ?? null,
};
