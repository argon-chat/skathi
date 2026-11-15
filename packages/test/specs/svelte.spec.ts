// A Svelte 5 app on Skathi's DOM: runes, `{#if}`, keyed `{#each}`, delegated clicks, scoped CSS, transitions, and
// a component hot update through `import.meta.hot`.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";
import { cacheDistFor, resolveEntry } from "../src/dist";

const ENTRY = "packages/test/fixtures/svelte/src/main.ts";
const APP_ID = "packages/test/fixtures/svelte/src/App.svelte";

const app = await launch({ app: ENTRY, window: [520, 420], label: "svelte" });
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Box = { x: number; y: number; w: number; h: number };
const box = async (selector: string): Promise<Box | null> =>
  JSON.parse(
    await app.eval(`(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return "null";
      const r = el.getBoundingClientRect();
      return JSON.stringify({ x: r.x, y: r.y, w: r.width, h: r.height });
    })()`),
  );
const ids = async (): Promise<string[]> =>
  JSON.parse(await app.eval(`JSON.stringify(Array.from(document.querySelectorAll(".item"), (li) => li.dataset.id))`));
async function until(cond: string, timeout = 5000): Promise<boolean> {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(50)) {
    if ((await app.eval(`String(!!(${cond}))`)) === "true") return true;
  }
  return false;
}

describe("svelte", () => {
  test(
    "mounts with props and scoped CSS; white space between blocks takes no room",
    async () => {
      await expect(app.locator("#title")).toHaveText("Svelte");
      await expect(app.locator("#count")).toHaveText("count 0, doubled 0");
      const title = (await box("#title"))!;
      bunExpect(title.h).toBe(30);
      bunExpect(await app.eval(`getComputedStyle(document.querySelector("#title")).color`)).toBe("rgb(200, 0, 0)");
      bunExpect((await app.locator("#title").classes()).some((c) => c.startsWith("svelte-"))).toBe(true);
      bunExpect((await app.locator(".bar").classes()).some((c) => c.startsWith("svelte-"))).toBe(false);
      // `<h1/> <p/>` and `<div/> <ul/>`: the " " Svelte keeps between them collapses away, as in a browser.
      bunExpect((await box("#count"))!.y).toBe(title.y + title.h);
      const bar = (await box(".bar"))!;
      bunExpect(bar.y).toBe(title.y + title.h + 20);
      bunExpect((await box("#list"))!.y).toBe(bar.y + bar.h);
      // Inside the row of buttons the " " is the word gap: they share a line.
      const inc = (await box("#inc"))!;
      const toggle = (await box("#toggle"))!;
      bunExpect(toggle.y).toBe(inc.y);
      bunExpect(toggle.x).toBeGreaterThan(inc.x + inc.w);
    },
    T,
  );

  test(
    "a click updates $state and $derived, and {#if} mounts its block with a fade",
    async () => {
      await app.locator("#inc").click();
      await expect(app.locator("#count")).toHaveText("count 1, doubled 2");
      bunExpect(await app.eval(`String(!!document.querySelector("#many"))`)).toBe("false");
      await app.locator("#inc").click();
      await expect(app.locator("#count")).toHaveText("count 2, doubled 4");
      await expect(app.locator("#many")).toHaveText("more than one");
    },
    T,
  );

  test(
    "keyed {#each} adds an item and moves the existing nodes on reorder",
    async () => {
      bunExpect(await ids()).toEqual(["1", "2"]);
      await app.locator("#add").click();
      bunExpect(await until(`document.querySelectorAll(".item").length === 3`)).toBe(true);
      bunExpect(await ids()).toEqual(["1", "2", "3"]);
      await expect(app.locator(".item").nth(2)).toHaveText("n2");
      await app.eval(`(globalThis.__SK_TEST_FIRST__ = document.querySelector(".item"), "kept")`);
      await app.locator("#rotate").click();
      bunExpect(await until(`document.querySelector(".item").dataset.id === "2"`)).toBe(true);
      bunExpect(await ids()).toEqual(["2", "3", "1"]);
      bunExpect(await app.eval(`String(document.querySelectorAll(".item")[2] === __SK_TEST_FIRST__)`)).toBe("true");
      bunExpect((await box(".item:last-child"))!.y).toBe((await box("#list"))!.y + 40);
    },
    T,
  );

  test(
    "slide measures the box through getComputedStyle and grows it to its full height",
    async () => {
      await app.locator("#toggle").click();
      bunExpect(await until(`!!document.querySelector("#panel")`)).toBe(true);
      bunExpect(await app.eval(`getComputedStyle(document.querySelector("#panel")).paddingTop`)).toBe("6px");
      bunExpect((await box("#panel"))!.h).toBeLessThan(40);
      bunExpect(
        await until(
          `document.querySelector("#panel").getAnimations().length === 0 && ` +
            `document.querySelector("#panel").getBoundingClientRect().height === 40`,
        ),
      ).toBe(true);
      await app.locator("#toggle").click();
      bunExpect(await until(`!document.querySelector("#panel")`)).toBe(true);
      // slide animates overflow and the border widths along with height and padding.
      const { entries } = await app.logs(0);
      bunExpect(entries.filter((e) => e.text.includes("is not animatable here")).map((e) => e.text)).toEqual([]);
    },
    T,
  );

  test(
    "an edited component hot-updates in place; module state survives",
    async () => {
      // What the host does with a rebuilt module: evaluate its script, then hand the change to the loader.
      const script = readFileSync(
        join(cacheDistFor(resolveEntry(ENTRY)), "modules", APP_ID.split("/").join("__") + ".js"),
        "utf8",
      );
      const edited = script.replace("add one", "add more").replace(/\}, "[0-9a-f]+"\);\s*$/, '}, "edited");');
      bunExpect(edited).toContain('"edited"');
      await app.eval(`${edited}\n"defined"`);
      await app.eval(`__SK__.hmrApply({ changed: [{ id: ${JSON.stringify(APP_ID)}, kind: "reload" }] }), "applied"`);
      await expect(app.locator("#inc")).toHaveText("add more");
      bunExpect(await ids()).toEqual(["2", "3", "1"]);
      bunExpect(await app.eval(`String(__SK__.mountCount())`)).toBe("1");
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes(`hot update ${APP_ID}`))).toBe(true);
      bunExpect(entries.some((e) => e.text.includes("hard restart"))).toBe(false);
      await app.locator("#inc").click();
      await expect(app.locator("#count")).toHaveText("count 1, doubled 2");
    },
    T,
  );

  test(
    "nothing threw",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
