import { createApp, h, ref } from "@skathi-native/runtime";
import "./color-picker.css";

const log: string[] = [];
(globalThis as { log?: string[] }).log = log;

const App = {
  setup() {
    const dark = ref(false);
    (globalThis as { setDark?: (on: boolean) => void }).setDark = (on) => (dark.value = on);
    const note = (e: Event) =>
      log.push(`${e.type}:${(e.target as HTMLInputElement).id}:${(e.target as HTMLInputElement).value}`);
    return () =>
      h("div", { id: "page", class: dark.value ? "dark" : "" }, [
        h("input", { id: "plain", type: "color", value: "#ff0000", onInput: note, onChange: note, onShowpicker: note }),
        h("input", { id: "alpha", type: "color", alpha: "", value: "#3366cc80", onInput: note, onChange: note }),
        h("input", {
          id: "custom",
          type: "color",
          value: "#00ff00",
          onShowpicker: (e: Event) => {
            e.preventDefault();
            note(e);
          },
        }),
        h(
          "button",
          { id: "pick", onClick: () => (document.getElementById("plain") as HTMLInputElement).showPicker() },
          "showPicker()",
        ),
        h("input", { id: "p3", type: "color", colorspace: "display-p3", value: "#ff0000", onInput: note }),
        h("input", {
          id: "hdr",
          type: "color",
          hdr: "",
          alpha: "",
          value: "color(srgb-linear 4 2 0.5)",
          onInput: note,
          onChange: note,
        }),
      ]);
  },
};

createApp(App).mount("#app");
