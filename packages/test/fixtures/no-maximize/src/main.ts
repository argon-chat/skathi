import { createApp, h } from "@skathi-native/runtime";
import "./no-maximize.css";

createApp({
  render: () =>
    h("div", { id: "app" }, [
      h("header", { id: "bar" }, [h("span", { id: "max" }, "max")]),
      h("main", { id: "main" }, "content"),
    ]),
}).mount();
