export const draft = $state({ text: localStorage.getItem("draft") ?? "" });

let timer: ReturnType<typeof setTimeout> | undefined;
const write = () => localStorage.setItem("draft", draft.text);

$effect.root(() => {
  $effect(() => {
    void draft.text;
    clearTimeout(timer);
    timer = setTimeout(write, 500);
  });
});
addEventListener("pagehide", write);
