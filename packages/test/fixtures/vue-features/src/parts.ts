import { defineComponent, h, inject, onActivated, onMounted, ref, type Directive } from "vue";
import { defineStore } from "pinia";
import { log } from "./log";

export const RouteA = defineComponent(() => () => h("p", { id: "route" }, "route a"));
export const RouteB = defineComponent(() => () => h("p", { id: "route" }, "route b"));

function counter(name: string) {
  return defineComponent({
    name,
    setup() {
      const count = ref(0);
      const activations = ref(0);
      onActivated(() => activations.value++);
      return () =>
        h("div", { class: "ka" }, [
          h("button", { id: `ka-inc-${name}`, onClick: () => count.value++ }, "inc"),
          h("span", { id: `ka-${name}` }, `${name} ${count.value} activated ${activations.value}`),
        ]);
    },
  });
}
export const KaA = counter("a");
export const KaB = counter("b");

export const Thrower = defineComponent(() => {
  onMounted(() => {
    throw new Error("mounted-boom");
  });
  return () => h("span", { id: "thrower" }, "thrower");
});

export const InjectChild = defineComponent(() => {
  const fromParent = inject<string>("fromParent", "missing");
  const fromApp = inject<string>("appLevel", "missing");
  return () => h("span", { id: "inject" }, `${fromParent} ${fromApp}`);
});

export const vTint: Directive<HTMLElement, string> = {
  mounted(el, binding) {
    el.style.color = binding.value;
    el.dataset.arg = `${binding.arg ?? ""}:${Object.keys(binding.modifiers).join(",")}`;
    log("directive:mounted");
  },
  updated(el, binding) {
    el.style.color = binding.value;
    log("directive:updated");
  },
};

export const useCounter = defineStore("counter", {
  state: () => ({ n: 0 }),
  getters: { doubled: (s) => s.n * 2 },
  actions: {
    inc() {
      this.n++;
    },
  },
});
