import { createApp, h, ref, vModelSelect, withDirectives } from "vue-native";
import "./select.css";

const log: string[] = [];
(globalThis as { log?: string[] }).log = log;

const options = (labels: string[]) => labels.map((label, i) => h("option", { value: i }, label));

const App = {
  setup() {
    const model = ref(1);
    const behind = ref(0);
    return () =>
      h("div", { id: "page" }, [
        withDirectives(
          h(
            "select",
            {
              id: "model",
              class: "box",
              "onUpdate:modelValue": (v: number) => (model.value = v),
              onInput: () => log.push("input"),
              onChange: (e: Event) => log.push(`change:${(e.target as HTMLSelectElement).value}`),
            },
            options(["Everybody", "Contacts", "Nobody at all"]),
          ),
          [[vModelSelect, model.value, undefined, { number: true }]],
        ),
        h("select", { id: "flat", class: "box" }, options(["Everybody", "Contacts", "Nobody at all"])),
        h("select", { id: "groups", class: "box" }, [
          h("option", { value: "a" }, "Alpha"),
          h("optgroup", { label: "Group" }, [
            h("option", { value: "b", disabled: true }, "Bravo"),
            h("option", { value: "c" }, "Charlie"),
          ]),
          h("option", { value: "d" }, "Delta"),
        ]),
        h(
          "select",
          {
            id: "guarded",
            class: "box",
            onMousedown: (e: Event) => e.preventDefault(),
            onKeydown: (e: KeyboardEvent) => e.key === "ArrowDown" && e.preventDefault(),
          },
          options(["One", "Two"]),
        ),
        h("select", { id: "far", class: "box" }, options(["One", "Two", "Three", "Four", "Five"])),
        h(
          "select",
          { id: "long", class: "box", onInput: () => log.push("long:input") },
          options(Array.from({ length: 40 }, (_, i) => `Item ${i}`)),
        ),
        h("button", { id: "behind", onClick: () => behind.value++ }, `behind ${behind.value}`),
        h("div", { id: "out" }, `model ${model.value}`),
      ]);
  },
};

createApp(App).mount("#app");
