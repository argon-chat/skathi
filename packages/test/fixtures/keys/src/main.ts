import { createApp, h, ref } from "@skathi-native/runtime";

const dom = document as unknown as SkDomDocument;

type Log = { log: string[] };
const out: Log = { log: [] };
(globalThis as unknown as { __SK_TEST_KEYS__: Log }).__SK_TEST_KEYS__ = out;

const idOf = (e: Event) => (e.target as { id?: string } | null)?.id || (e.target === document ? "document" : "?");
const note = (who: string) => (e: KeyboardEvent) => out.log.push(`${who}:${e.type}:${e.key} t=${idOf(e)}`);

dom.addEventListener("keydown", note("document"));
dom.addEventListener("keyup", note("document"));

const fieldOpen = ref(true);

const App = {
  render: () =>
    h(
      "div",
      {
        id: "page",
        style: {
          display: "flex",
          flexDirection: "column",
          gap: "12px",
          padding: "16px",
          background: "#fff",
          height: "100%",
        },
      },
      [
        h("div", { id: "wrap", onKeydown: note("wrap"), style: { padding: "8px", background: "#eef" } }, [
          h("div", { id: "box", onKeydown: note("box"), style: { padding: "8px", background: "#dde" } }, [
            h("div", { id: "inner", tabindex: 0, style: { padding: "8px", background: "#ccd" } }, "inner"),
          ]),
        ]),
        fieldOpen.value
          ? h("input", {
              id: "field",
              onKeydown: (e: KeyboardEvent) => {
                note("field")(e);
                if (e.key === "Escape") fieldOpen.value = false;
              },
              style: { width: "200px" },
            })
          : h("button", { id: "reopen", onClick: () => (fieldOpen.value = true) }, "reopen"),
        h("input", { id: "plain", style: { width: "200px" } }),
        h("input", {
          id: "suggest",
          onKeydown: (e: KeyboardEvent) => {
            if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Enter") return;
            e.preventDefault();
            const active = (dom.activeElement as { id?: string } | null)?.id ?? "none";
            (e.target as HTMLInputElement).focus();
            out.log.push(`suggest:keydown:${e.key} active=${active}`);
          },
          style: { width: "200px" },
        }),
      ],
    ),
};

createApp(App).mount("#app");
