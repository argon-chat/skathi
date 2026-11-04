// A fixture for packages/vn-test/specs/overlay.spec.ts: an overlay, the way
// Argon's in-game voice HUD is one (docs/MULTIWINDOW.md §7.11).
//
// `#hud` is a small frameless `<VnWindow>` on a transparent backdrop, on top of
// everything, that the mouse goes through (`ignoreMouseEvents`), that is never
// activated (`focusable: false`) and that screen capture leaves out
// (`contentProtection`). Nearly all of it is left unpainted; `#hud-box`, an
// opaque square in its top-left corner, is what the spec reads off the SCREEN
// to see that the window still presents new frames while it is click-through.
//
// `#cover` is a second overlay, created hidden, that covers a whole monitor
// chosen by index (`cover="monitor"` + `monitor`). It paints nothing but
// `#cover-box`, so it is invisible apart from that even when it is shown, and
// it is on top only when `coverOnTop` says so.
//
// `__VN_TEST_OV__.S` is the state the spec flips: `ignore`, `protect`,
// `focusable` (creation-time — flipping it only advises), `color` (the box),
// `coverVisible`, `coverOnTop`, `monitor`.
//
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h, reactive, VnWindow } from "vue-native";

const S = reactive({
  ignore: true,
  protect: true,
  focusable: false,
  color: "#ff0000",
  coverVisible: false,
  coverOnTop: false,
  monitor: 0 as number | undefined,
});

createApp({
  setup() {
    return () =>
      h("div", { id: "app", style: { padding: "12px" } }, [
        h("div", { id: "main-text" }, "the app"),
        h(
          VnWindow,
          {
            id: "hud",
            title: "HUD",
            chrome: "hidden",
            backdrop: "transparent",
            bounds: { x: 40, y: 40, width: 160, height: 100 },
            resizable: false,
            alwaysOnTop: true,
            focusable: S.focusable,
            ignoreMouseEvents: S.ignore,
            contentProtection: S.protect,
          },
          () =>
            h("div", { id: "hud-root", style: { position: "absolute", inset: "0" } }, [
              h("div", {
                id: "hud-box",
                style: { position: "absolute", left: "8px", top: "8px", width: "32px", height: "32px", background: S.color },
              }),
            ]),
        ),
        h(
          VnWindow,
          {
            id: "cover",
            title: "Cover",
            chrome: "hidden",
            backdrop: "transparent",
            cover: "monitor",
            monitor: S.monitor,
            visible: S.coverVisible,
            alwaysOnTop: S.coverOnTop,
            focusable: false,
            ignoreMouseEvents: true,
          },
          () =>
            h("div", { id: "cover-root", style: { position: "absolute", inset: "0" } }, [
              h("div", {
                id: "cover-box",
                style: { position: "absolute", right: "24px", top: "24px", width: "48px", height: "48px", background: "#ff00ff" },
              }),
            ]),
        ),
      ]);
  },
}).mount();

(globalThis as Record<string, unknown>).__VN_TEST_OV__ = { S };
