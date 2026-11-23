import { createApp, h } from "@skathi-native/runtime";

function startup(): void {
  throw new Error("entry-throws fixture: startup failed on purpose");
}

startup();
createApp({ render: () => h("div", "unreachable") }).mount("#app");
