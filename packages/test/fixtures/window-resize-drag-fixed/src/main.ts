import { createApp, h } from "@skathi-native/runtime";
import { appWindow } from "skathi:app";

createApp({
  render: () =>
    h("div", { id: "app", style: { height: "100vh", background: "#1e1f22" } }, [
      h("div", {
        id: "right",
        style: { position: "fixed", right: "0", top: "10px", bottom: "10px", width: "5px", cursor: "e-resize" },
        onPointerdown: () => appWindow.startResizeDragging(),
      }),
      h("div", {
        id: "corner",
        style: { position: "fixed", right: "0", bottom: "0", width: "10px", height: "10px", cursor: "se-resize" },
        onPointerdown: () => appWindow.startResizeDragging("SouthEast"),
      }),
    ]),
}).mount();
