import { createApp, h, ref } from "@skathi-native/runtime";

const log: string[] = [];
const shown = ref(true);
const api = {
  log,
  locked: () => {
    const el = document.pointerLockElement as { id?: string } | null;
    return el ? (el.id ?? "?") : null;
  },
  request: () => {
    document
      .getElementById("pad")
      ?.requestPointerLock()
      .then(
        () => log.push("resolved"),
        (e: DOMException) => log.push(`rejected:${e.name}`),
      );
  },
  exit: () => document.exitPointerLock(),
  remove: () => (shown.value = false),
  restore: () => (shown.value = true),
};
(globalThis as unknown as { __SK_TEST_LOCK__: typeof api }).__SK_TEST_LOCK__ = api;

document.addEventListener("pointerlockchange", () => log.push(`change:${api.locked()}`));
document.addEventListener("pointerlockerror", () => log.push("error"));
document.addEventListener("keydown", (e) => log.push(`keydown:${e.key}`));

const App = {
  render: () =>
    h("div", { id: "page", style: { padding: "16px", height: "100%", background: "#fff" } }, [
      shown.value
        ? h("div", {
            id: "pad",
            style: { width: "200px", height: "120px", background: "#dde" },
            onClick: api.request,
            onMousemove: (e: MouseEvent) =>
              log.push(`move:${Math.round(e.clientX)},${Math.round(e.clientY)} by ${e.movementX},${e.movementY}`),
          })
        : null,
      h("div", { id: "other", style: { width: "80px", height: "40px", marginTop: "12px", background: "#edd" } }),
    ]),
};

createApp(App).mount();
