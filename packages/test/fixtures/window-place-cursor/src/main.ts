import { createApp, h } from "@skathi-native/runtime";

createApp({ render: () => h("div", { id: "app" }, "under the pointer") }).mount();
