import { createApp, h } from "@skathi-native/runtime";
import "./power.css";

createApp({
  render: () => h("div", { id: "power" }, "Power and pressure"),
}).mount();
