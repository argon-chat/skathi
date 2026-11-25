import { createApp, h } from "@skathi-native/runtime";
import "./state-specificity.css";

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("label", { id: "l" }, [h("input", { id: "i", type: "number" })]),
      h("label", { id: "plain" }, [h("input", { id: "j", type: "number" })]),
    ]),
}).mount();
