import { ref, watch } from "vue";

export const draft = ref(localStorage.getItem("draft") ?? "");

let timer: ReturnType<typeof setTimeout> | undefined;
const write = () => localStorage.setItem("draft", draft.value);

watch(draft, () => {
  clearTimeout(timer);
  timer = setTimeout(write, 500);
});
addEventListener("pagehide", write);
