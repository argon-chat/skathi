// Skathi's own elements from Svelte: Svelte sets every attribute of a hyphenated tag through its custom-element
// path, which reads `customElements`.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/svelte-elements/src/main.ts",
  window: [480, 240],
  label: "svelte-elements",
});
afterAll(() => app.close());

const T = 30_000;
const state = async <T>(expr: string): Promise<T> =>
  JSON.parse(String(await app.eval(`JSON.stringify(__SK_TEST_SVELTE_ELEMENTS__.${expr})`))) as T;

async function until(what: string, ok: () => Promise<boolean>, ms = 15_000) {
  const end = Date.now() + ms;
  while (!(await ok())) {
    if (Date.now() > end) {
      throw new Error(
        `timed out waiting for ${what}: ${JSON.stringify(await state("events"))}\n${await app.tailLogs()}`,
      );
    }
    await Bun.sleep(50);
  }
}

test(
  "sk-surface, sk-sticker and lottie mount with their attributes and fire their events",
  async () => {
    await until("the elements' events", async () => {
      const events = await state<string[]>("events");
      return ["surfaceresize:", "load:sticker", "complete:lottie"].every((e) => events.some((x) => x.startsWith(e)));
    });
    expect(await state<unknown>("attributes()")).toEqual({
      surface: "first",
      sticker: ["/src/steps.json", "off"],
      lottie: ["/src/steps.json", "3"],
    });
    await app.eval(`__SK_TEST_SVELTE_ELEMENTS__.rename("second"); 0`);
    await until("the surface's name to follow its state", async () => {
      return (await state<{ surface: string }>("attributes()")).surface === "second";
    });
    await app.expectNoErrors();
  },
  T,
);
