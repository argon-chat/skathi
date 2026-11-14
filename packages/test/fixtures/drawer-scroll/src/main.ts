// In Chromium the row is held to the drawer's height (its automatic minimum size is the smaller of
// its content and its definite `height: 100%`), so the scroller is too and scrolls.
import { createApp, h } from "@skathi-native/runtime";
import "./drawer.css";

const ROWS = 36;

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "drawer", class: "drawer" }, [
        h("div", { class: "handle" }),
        h("div", { id: "header", class: "header" }, [
          h("div", [h("h2", "Settings"), h("p", "Manage your settings")]),
          h("div", { class: "close" }),
        ]),
        h("div", { id: "layout", class: "layout" }, [
          h(
            "nav",
            { id: "nav", class: "nav" },
            Array.from({ length: 8 }, (_, i) => h("div", { class: "nav-item" }, `Category ${i + 1}`)),
          ),
          h(
            "div",
            { id: "content", class: "content" },
            Array.from({ length: ROWS }, (_, i) =>
              h(
                "div",
                { class: i === ROWS - 1 ? "row last" : "row", id: i === ROWS - 1 ? "last" : undefined },
                `Row ${i + 1}`,
              ),
            ),
          ),
        ]),
      ]),
    ]),
}).mount();
