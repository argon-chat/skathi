// A promise does not survive the eval round trip either, so `run()` records how each call SETTLED
// into `results`, and the spec polls that.
import { createApp, h, ref } from "@skathi-native/runtime";
import { appShell } from "skathi:app";

type Verb = "openExternal" | "openPath" | "showItemInFolder" | "trashItem";
const results: string[] = [];
const g = globalThis as unknown as Record<string, unknown>;
g.__SK_TEST_SHELL__ = {
  results,
  run(verb: Verb, ...args: unknown[]) {
    const call = (appShell[verb] as (...a: unknown[]) => Promise<void>)(...args);
    call.then(
      () => results.push(`${verb} ok`),
      (e: { name?: string; code?: string }) => results.push(`${verb} ${e.name} ${e.code ?? ""}`.trim()),
    );
  },
};

const spanClicks = ref(0);

createApp({
  render: () =>
    h("div", { class: "fixture" }, [
      // An https link whose CHILD also listens: one physical click reaches two listening boxes,
      // and must still be one navigation.
      h("a", { class: "ext", href: "https://example.invalid/sk-test?q=a b" }, [
        h("span", { class: "ext-label", onClick: () => spanClicks.value++ }, "external link"),
      ]),
      h("a", { class: "mail", href: "mailto:nobody@example.invalid" }, "mail link"),
      h("a", { class: "custom", href: "sktest-refused://x" }, "custom-scheme link"),
      h("a", { class: "file", href: "file:///C:/Windows/System32/calc.exe" }, "file link"),
      h(
        "a",
        { class: "routed", href: "https://example.invalid/routed", onClick: (e: Event) => e.preventDefault() },
        "cancelled link",
      ),
      h("div", { class: "count" }, `span clicks: ${spanClicks.value}`),
    ]),
}).mount();
