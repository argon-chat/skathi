// Svelte 5 features on Skathi's DOM, as the "Svelte support" guide lists them: runes, snippets, blocks, attachments
// and actions, transitions and motion, bindings, special elements, boundaries, stores, legacy syntax and reactivity.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/svelte-features/src/main.ts",
  window: [640, 900],
  label: "svelte-features",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const js = (expr: string): Promise<unknown> => app.eval(expr);
const typed = <V>(expr: string): Promise<V> => app.eval(expr);
const text = (selector: string) =>
  typed<string | null>(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`);
const events = () => typed<string[]>("__sf");
async function until(cond: string, timeout = 5000): Promise<boolean> {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(30)) {
    if ((await app.eval(`String(!!(${cond}))`)) === "true") return true;
  }
  return false;
}

describe("svelte features", () => {
  test(
    "runes: $props, $state, $derived, $effect and tick()",
    async () => {
      await expect(app.locator("#label")).toHaveText("props");
      await expect(app.locator("#count")).toHaveText("1 2");
      await app.locator("#inc").click();
      await expect(app.locator("#count")).toHaveText("2 4");
      bunExpect((await events()).filter((e) => e.startsWith("effect:") || e.startsWith("tick:"))).toEqual([
        "effect:0",
        "effect:1",
        "tick:1 2",
        "effect:2",
      ]);
    },
    T,
  );

  test(
    "$bindable, context and a snippet passed to a child",
    async () => {
      bunExpect(await text("#ctx")).toBe("from-context");
      bunExpect(await text("#snippet")).toBe("snip");
      await app.locator("#bindable").type("X");
      await expect(app.locator("#bound")).toHaveText("initX");
    },
    T,
  );

  test(
    "{#await} shows pending, then the value or the error; {#key} recreates its content",
    async () => {
      bunExpect(await text("#await")).toBe("pending");
      bunExpect(await js("__release('await')")).toBe(true);
      await expect(app.locator("#await")).toHaveText("awaited");
      await app.locator("#reject").click();
      await expect(app.locator("#await")).toHaveText("nope");
      await js(`(globalThis.__keyed = document.querySelector("#keyed"), 0)`);
      await app.locator("#key-bump").click();
      await expect(app.locator("#keyed")).toHaveText("1");
      bunExpect(await js(`document.querySelector("#keyed") !== globalThis.__keyed`)).toBe(true);
    },
    T,
  );

  test(
    "{@attach} and use: run on their element",
    async () => {
      bunExpect(await js(`getComputedStyle(document.querySelector("#attached")).color`)).toBe("rgb(0, 128, 0)");
      bunExpect(await js(`document.querySelector("#action").dataset.mark`)).toBe("marked");
      bunExpect(await events()).toEqual(bunExpect.arrayContaining(["attach", "action"]));
    },
    T,
  );

  test(
    "in: and out: transitions animate, and Tween and Spring reach their target",
    async () => {
      await app.locator("#tr-toggle").click();
      bunExpect(await until(`document.querySelector("#tr")?.getAnimations().length > 0`)).toBe(true);
      bunExpect(await until(`document.querySelector("#tr")?.getAnimations().length === 0`)).toBe(true);
      await app.locator("#tr-toggle").click();
      bunExpect(await js(`!!document.querySelector("#tr")`)).toBe(true);
      bunExpect(await until(`!document.querySelector("#tr")`)).toBe(true);
      await app.locator("#tween-go").click();
      bunExpect(await until(`document.querySelector("#tween").textContent === "100 100"`)).toBe(true);
    },
    T,
  );

  test(
    "bind:value on text fields and selects, bind:this, bind:clientWidth and <svelte:window bind:innerWidth>",
    async () => {
      await app.locator("#b-text").type("hi");
      bunExpect(await js(`document.querySelector("#b-select").querySelector(":checked").value`)).toBe("b");
      await js(`(() => {
        const select = document.querySelector("#b-select");
        select.value = "c";
        select.dispatchEvent(new Event("change", { bubbles: true }));
        const many = document.querySelector("#b-many");
        many.options[1].selected = true;
        many.dispatchEvent(new Event("change", { bubbles: true }));
      })()`);
      await expect(app.locator("#b-out")).toHaveText(
        JSON.stringify({ text: "hi", picked: "c", many: ["y"], el: "measured", width: 123, innerWidth: 640 }),
      );
      bunExpect(
        await js(`(() => {
          const s = document.querySelector("#b-select");
          return [s.selectedIndex, s.selectedOptions.length, s.selectedOptions[0].value, s.options[2].selected];
        })()`),
      ).toEqual([2, 1, "c", true]);
    },
    T,
  );

  test(
    "bind:checked, bind:indeterminate and bind:group on checkboxes and radios, with labels",
    async () => {
      const out = () => typed<string>(`document.querySelector("#s-out").textContent`);
      bunExpect(JSON.parse(await out())).toEqual({ agreed: false, mixed: true, fruits: ["pear"], size: "m" });
      bunExpect(await js(`document.querySelector("#s-m").checked`)).toBe(true);
      await app.locator("#s-cb-label").click();
      await app.locator("#s-mixed").click();
      await app.locator("#s-apple").click();
      await app.locator("#s-l-label").click();
      await expect(app.locator("#s-out")).toHaveText(
        JSON.stringify({ agreed: true, mixed: false, fruits: ["apple", "pear"], size: "l" }),
      );
      bunExpect(await js(`[...document.querySelectorAll("#toggles :checked")].map((e) => e.id).join()`)).toBe(
        "s-cb,s-mixed,s-apple,s-pear,s-l",
      );
      await app.locator("#s-cb").press("Space");
      await expect(app.locator("#s-out")).toHaveText(/"agreed":false/);
    },
    T,
  );

  test(
    "animate:flip moves the items of a keyed {#each} that stay",
    async () => {
      await js(`(() => {
        const tops = (globalThis.__tops = []);
        const sample = () => {
          tops.push(Math.round(document.querySelector("#flip .flip-item").getBoundingClientRect().top));
          if (tops.length < 20) requestAnimationFrame(sample);
        };
        document.querySelector("#flip-go").addEventListener("click", () => requestAnimationFrame(sample), { once: true });
      })()`);
      await app.locator("#flip-go").click();
      bunExpect(await until(`__tops.length >= 20`)).toBe(true);
      bunExpect(new Set(await typed<number[]>("__tops")).size).toBeGreaterThan(3);
      bunExpect(await text("#flip .flip-item")).toBe("3");
    },
    T,
  );

  test(
    "an error thrown by an onclick handler is reported as uncaught, past a <svelte:boundary>, as in a browser",
    async () => {
      await app.locator("#click-boom").click();
      bunExpect(await until(`__sf.includes("window-error:click-boom")`)).toBe(true);
      bunExpect(await events()).not.toContain("boundary:click-boom");
      bunExpect(await js(`!!document.querySelector("#click-boom")`)).toBe(true);
      const { entries } = await app.logs();
      const report = entries.find((e) => e.text.includes("click-boom"));
      bunExpect(report?.text).toMatch(/Error: click-boom\n\S/);
    },
    T,
  );

  test(
    "<svelte:element>, <svelte:boundary>, <svelte:head>, and window, document and body listeners",
    async () => {
      bunExpect(await js(`document.querySelector("#dynamic").tagName`)).toBe("SECTION");
      await app.locator("#tag").click();
      bunExpect(await until(`document.querySelector("#dynamic").tagName === "ARTICLE"`)).toBe(true);
      await app.locator("#boom-go").click();
      await expect(app.locator("#failed")).toHaveText("render-boom");
      bunExpect(await events()).toContain("boundary:render-boom");
      await app.locator("#reset").click();
      await expect(app.locator("#boom")).toHaveText("fine");
      bunExpect(await js("document.title")).toBe("svelte head title");
      bunExpect(await events()).toEqual(bunExpect.arrayContaining(["body:click", "document:click"]));
      await app.press("q");
      bunExpect(await until(`__sf.includes("window:q")`)).toBe(true);
    },
    T,
  );

  test(
    "stores, a Svelte 4 component (export let, $:, on:, slots), svelte/reactivity and class/style directives",
    async () => {
      await app.locator("#store-inc").click();
      await expect(app.locator("#store")).toHaveText("2 4");
      await expect(app.locator("#legacy")).toHaveText("5 10");
      await app.locator("#legacy-inc").click();
      await expect(app.locator("#legacy")).toHaveText("6 12");
      await expect(app.locator("#slot")).toHaveText("slot 6");
      bunExpect(await events()).toContain("bumped:6");
      await expect(app.locator("#reactivity")).toHaveText("none 0 true");
      await app.locator("#react").click();
      await expect(app.locator("#reactivity")).toHaveText("1 1 true");
      bunExpect(await js(`document.querySelector("#classy").className`)).toBe("base on active");
      bunExpect(await js(`getComputedStyle(document.querySelector("#classy")).color`)).toBe("rgb(0, 0, 255)");
    },
    T,
  );
});
