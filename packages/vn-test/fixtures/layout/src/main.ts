// A fixture for packages/vn-test/specs/layout.spec.ts — the m26 client sweep's
// layout and paint findings in the Argon client, each reduced to its shape
// (research/reports/m26/client-sweep-1.md, fixes in fixes-layout-1.md).
//
// One section per finding, each laid out where the spec can hit it. The class
// names spell out the Tailwind utilities the client wrote, so a rule maps back
// to `apps/client` (MessageItem.vue). Render functions rather than an SFC: this
// package is `"type": "module"`, and the dev builder's CommonJS interop hands
// such an importer a `.vue` module's NAMESPACE as its default export.
import { createApp, h } from "vue-native";
import "./layout.css";

const range = (n: number) => Array.from({ length: n }, (_, i) => i + 1);
const two = (i: number) => (i < 10 ? `0${i}` : `${i}`);

/** #1: a chat row's timestamp, `hidden group-hover/msg:flex`, in a scroller. */
const messages = () =>
  h(
    "div",
    { id: "msgs", class: "msgs" },
    range(30).map((i) =>
      h("div", { id: `msg-${i}`, key: i, class: "group-msg msg" }, [
        h("div", { class: "gutter" }, [
          h("span", { id: `ts-${i}`, class: "hidden group-msg-hover-flex ts" }, `12:${two(i)}`),
        ]),
        h("div", { class: "body" }, `message ${i}`),
      ]),
    ),
  );

createApp({
  render: () => h("div", { id: "root" }, [messages()]),
}).mount();
