import { createApp, h } from "@skathi-native/runtime";
import "./fullscreen.css";

createApp({
  render: () =>
    h("div", { id: "app" }, [
      h("header", { id: "bar" }, [h("span", { id: "brand" }, "Showcase / Window")]),
      h("main", { id: "main" }, "content"),
    ]),
}).mount();
