import { createApp, h } from "@skathi-native/runtime";
import "./intrinsic.css";

const box = (id: string, children?: ReturnType<typeof h>[]) => h("div", { id }, children);
const content = (id: string) => box(id, [box(`${id}-content`)]);

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "panel-slot" }, [content("panel")]),
      h("div", { id: "sizes" }, [
        content("fit"),
        content("half"),
        h("div", { id: "basis-row" }, [content("basis")]),
        box("logical"),
      ]),
      h("div", { id: "functions" }, [
        box("fn-size"),
        box("fn-color", [h("span", {}, "tinted")]),
        box("fn-cycle"),
        h("div", { id: "fn-container" }, [box("fn-cq")]),
        box("fn-var"),
      ]),
      h("div", { id: "drl-parent" }, [box("drl-child"), box("drl-mix")]),
      box("drl-default"),
    ]),
}).mount();
