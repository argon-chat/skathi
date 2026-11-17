// Vue 3 features on Skathi's renderer, as the "Vue support" guide lists them: Suspense, Teleport, KeepAlive,
// transitions, async components, v-model, directives, refs, provide/inject, error hooks, scoped CSS and the router.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/vue-features/src/main.ts",
  window: [640, 900],
  label: "vue-features",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const js = (expr: string): Promise<unknown> => app.eval(expr);
const typed = <V>(expr: string): Promise<V> => app.eval(expr);
const text = (selector: string) =>
  typed<string | null>(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`);
const color = (selector: string) =>
  typed<string>(`getComputedStyle(document.querySelector(${JSON.stringify(selector)})).color`);
const events = () => typed<string[]>("__vf");
async function until(cond: string, timeout = 5000): Promise<boolean> {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(30)) {
    if ((await app.eval(`String(!!(${cond}))`)) === "true") return true;
  }
  return false;
}

describe("vue features", () => {
  test(
    "<Suspense> shows the fallback until async setup resolves, then the content",
    async () => {
      await expect(app.locator("#sus-fallback")).toHaveText("loading");
      bunExpect(await text("#sus-done")).toBeNull();
      bunExpect(await js("__release('suspense')")).toBe(true);
      await expect(app.locator("#sus-done")).toHaveText("resolved");
      bunExpect(await text("#sus-fallback")).toBeNull();
      bunExpect((await events()).filter((e) => e.startsWith("suspense:"))).toEqual([
        "suspense:pending",
        "suspense:resolve",
      ]);
    },
    T,
  );

  test(
    "<Teleport> moves content to the document root and to a deferred target, and stays inline when disabled",
    async () => {
      bunExpect(await js(`document.querySelector("#tp-body").parentElement === document.body`)).toBe(true);
      bunExpect(await js(`document.querySelector("#tp-deferred").parentElement.id`)).toBe("tp-late");
      bunExpect(await js(`document.querySelector("#tp-disabled").parentElement.id`)).toBe("teleport");
    },
    T,
  );

  test(
    "<KeepAlive> keeps a component's state across a switch and calls onActivated",
    async () => {
      await app.locator("#ka-inc-a").click();
      await expect(app.locator("#ka-a")).toHaveText("a 1 activated 1");
      await app.locator("#ka-switch").click();
      await expect(app.locator("#ka-b")).toHaveText("b 0 activated 1");
      await app.locator("#ka-switch").click();
      await expect(app.locator("#ka-a")).toHaveText("a 1 activated 2");
    },
    T,
  );

  test(
    "<Transition> runs the CSS classes and JS hooks, and removes the element after leaving",
    async () => {
      await app.locator("#tr-toggle").click();
      bunExpect(await until(`document.querySelector("#tr-el")?.classList.contains("fade-enter-active")`)).toBe(true);
      bunExpect(await until(`__vf.some((e) => e.startsWith("tr:after-enter"))`)).toBe(true);
      bunExpect(await js(`document.querySelector("#tr-el").className`)).toBe("");
      await app.locator("#tr-toggle").click();
      bunExpect(await until(`document.querySelector("#tr-el")?.classList.contains("fade-leave-active")`)).toBe(true);
      bunExpect(await until(`!document.querySelector("#tr-el")`)).toBe(true);
      bunExpect((await events()).filter((e) => e.startsWith("tr:"))).toEqual([
        "tr:before-enter",
        "tr:enter:fade-enter-from fade-enter-active",
        "tr:after-enter:",
        "tr:leave:fade-leave-from fade-leave-active",
        "tr:after-leave",
      ]);
    },
    T,
  );

  test(
    "<TransitionGroup> plays enter and leave on list items",
    async () => {
      await app.locator("#tg-add").click();
      await app.locator("#tg-remove").click();
      const items = () =>
        typed<string>(
          `Array.from(document.querySelectorAll(".tg-item"), (e) => e.textContent + ":" + e.className).join(" ")`,
        );
      bunExpect(await until(`document.querySelector(".tg-item").classList.contains("list-leave-active")`)).toBe(true);
      bunExpect(await items()).toContain("4:tg-item list-enter");
      bunExpect(await until(`document.querySelectorAll(".tg-item").length === 3`)).toBe(true);
      bunExpect(await until(`!document.querySelector("#tg .list-enter-active")`)).toBe(true);
      bunExpect(await items()).toBe("2:tg-item 3:tg-item 4:tg-item");
    },
    T,
  );

  test(
    "defineAsyncComponent renders a code-split component and a loading component while it loads",
    async () => {
      bunExpect(await js(`Array.from(document.querySelectorAll("#async > *"), (e) => e.textContent).join(",")`)).toBe(
        "imported,loading",
      );
      bunExpect(await js("__release('lazy')")).toBe(true);
      bunExpect(await until(`!document.querySelector("#lazy-loading")`)).toBe(true);
      bunExpect(await js(`Array.from(document.querySelectorAll("#async > *"), (e) => e.textContent).join(",")`)).toBe(
        "imported,slow",
      );
    },
    T,
  );

  test(
    "v-model on text fields, selects and a component (defineModel with modifiers)",
    async () => {
      await app.locator("#m-text").type("hello");
      await app.locator("#m-area").type("multi");
      await app.locator("#m-num").type("42");
      await app.locator("#m-trim").type("  pad  ");
      await app.locator("#dm-input").type("!");
      await app.locator("#dm-title").click();
      await js(`(() => {
        const one = document.querySelector("#m-select");
        one.value = "c";
        one.dispatchEvent(new Event("change"));
        const many = document.querySelector("#m-many");
        many.options[1].selected = true;
        many.dispatchEvent(new Event("change"));
      })()`);
      await expect(app.locator("#m-out")).toHaveText(
        JSON.stringify({
          text: "hello",
          area: "multi",
          picked: "c",
          many: ["y"],
          num: 42,
          trimmed: "pad",
          model: "start!",
          title: "NEW",
        }),
      );
    },
    T,
  );

  test(
    "custom directives get arg, modifiers and updates; v-once and v-memo skip updates",
    async () => {
      bunExpect(await color("#tinted")).toBe("rgb(255, 0, 0)");
      bunExpect(await js(`document.querySelector("#tinted").dataset.arg`)).toBe("arg:mod");
      await app.locator("#tinted").click();
      bunExpect(await until(`getComputedStyle(document.querySelector("#tinted")).color === "rgb(0, 0, 255)"`)).toBe(
        true,
      );
      await app.locator("#count-inc").click();
      await expect(app.locator("#count")).toHaveText("2");
      bunExpect(await text("#once")).toBe("0");
      bunExpect(await text("#memo")).toBe("0");
      await app.locator("#memo-bump").click();
      await expect(app.locator("#memo")).toHaveText("2");
    },
    T,
  );

  test(
    "template refs, provide/inject, useId, pinia and nextTick",
    async () => {
      bunExpect(await text("#refs-out")).toBe("input INPUT, list 3");
      bunExpect(await text("#inject")).toBe("from-parent from-app");
      bunExpect(await text("#ids")).toBe("v-0 v-1");
      bunExpect(await events()).toContain("nextTick:1");
      await app.locator("#store").click();
      await expect(app.locator("#store")).toHaveText("1 2");
    },
    T,
  );

  test(
    "onErrorCaptured and app.config.errorHandler see an error thrown by a lifecycle hook",
    async () => {
      bunExpect(await events()).toContain("captured:mounted-boom:mounted hook");
      bunExpect(await events()).toContain("errorHandler:mounted-boom:mounted hook");
    },
    T,
  );

  test(
    "scoped styles with :deep, :slotted, :global and v-bind() in CSS",
    async () => {
      bunExpect(await color("#styled")).toBe("rgb(10, 20, 30)");
      bunExpect(await color("#deep")).toBe("rgb(255, 0, 255)");
      bunExpect(await color("#slotted")).toBe("rgb(0, 200, 200)");
      bunExpect(await color("#global")).toBe("rgb(200, 100, 0)");
      bunExpect(await color("#vbind")).toBe("rgb(0, 128, 0)");
      await app.locator("#vbind").click();
      bunExpect(await until(`getComputedStyle(document.querySelector("#vbind")).color === "rgb(0, 0, 255)"`)).toBe(
        true,
      );
    },
    T,
  );

  test(
    "vue-router with memory history navigates and goes back",
    async () => {
      await expect(app.locator("#route")).toHaveText("route a");
      await app.locator("#route-b").click();
      await expect(app.locator("#route")).toHaveText("route b");
      await app.locator("#route-back").click();
      await expect(app.locator("#route")).toHaveText("route a");
    },
    T,
  );
});
