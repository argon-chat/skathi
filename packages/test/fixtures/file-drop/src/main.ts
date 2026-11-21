import { createApp, defineComponent, h, ref } from "@skathi-native/runtime";
import { appFiles } from "skathi:app";
import { useDropZone } from "@vueuse/core";

const VueDrop = defineComponent({
  setup() {
    const target = ref<HTMLElement>();
    const received = ref("");
    const { isOverDropZone } = useDropZone(target, {
      dataTypes: ["text/plain"],
      onDrop(files) {
        received.value = files?.map((f) => f.name).join(",") ?? "";
      },
    });
    return () =>
      h(
        "div",
        { id: "vueuse", ref: target, style: "height: 35px" },
        isOverDropZone.value ? "over" : received.value || "idle",
      );
  },
});

const log: unknown[] = [];
const state = {
  log,
  drop: [] as unknown[],
  files: [] as File[],
  handles: [] as (FileSystemHandle | null)[],
  entries: [] as (FileSystemEntry | null)[],
  retained: null as DataTransferItem | null,
  path: appFiles.getPathForFile,
  protectedEntry: false,
  allow: true,
};
(globalThis as { __dropTest?: unknown }).__dropTest = state;

function onDrag(e: DragEvent) {
  const dt = e.dataTransfer!;
  log.push([
    e.type,
    (e.target as Element).id,
    (e.relatedTarget as Element | null)?.id ?? null,
    dt.files.length,
    Array.from(dt.types),
    Array.from(dt.items).map((i) => [i.kind, i.type, i.getAsFile()?.name ?? null]),
    dt.getData("text/plain"),
    e.ctrlKey,
  ]);
  if (e.type === "dragover") state.protectedEntry = dt.items[0]?.webkitGetAsEntry() === null;
  if (e.type === "dragover" && state.allow) {
    dt.dropEffect = "copy";
    e.preventDefault();
  }
  if (e.type === "drop") {
    e.preventDefault();
    state.files = Array.from(dt.files);
    state.entries = Array.from(dt.items).map((item) => item.webkitGetAsEntry());
    state.retained = dt.items[0] ?? null;
    state.drop = state.files.map((f) => [f.name, f.size, f.type, f.webkitRelativePath]);
    void Promise.all(
      Array.from(dt.items)
        .filter((i) => i.kind === "file")
        .map((i) => i.getAsFileSystemHandle?.() ?? null),
    ).then((h) => {
      state.handles = h;
    });
  }
}
createApp(
  defineComponent({
    setup: () => () =>
      h("div", { id: "outer" }, [
        h(VueDrop),
        h(
          "div",
          {
            id: "zone",
            style: "width: 300px; height: 100px;",
            onDragenter: onDrag,
            onDragover: onDrag,
            onDragleave: onDrag,
            onDrop: onDrag,
          },
          [h("div", { id: "child", style: "width: 90px; height: 45px;" }, "Drop here")],
        ),
        h("div", { id: "away", style: "width: 120px; height: 35px;" }, "Outside"),
        h("input", {
          id: "single",
          type: "file",
          accept: "image/png",
          onInput: () => log.push("input"),
          onChange: () => log.push("change"),
        }),
        h("input", { id: "multi", type: "file", multiple: true }),
      ]),
  }),
).mount("#app");
