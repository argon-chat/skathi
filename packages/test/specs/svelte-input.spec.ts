// Svelte creates its template elements through document.createElement, `<input>` and `<a>` included.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/svelte-input/src/main.ts",
  window: [420, 240],
  label: "svelte-input",
});
afterAll(() => app.close());

const T = 30_000;

describe("svelte <input> and <a>", () => {
  test(
    "they mount into the tree and are painted",
    async () => {
      await expect(app.locator("#echo")).toHaveText("hello");
      bunExpect(await app.locator("#name").isVisible()).toBe(true);
      bunExpect(await app.locator("#link").isVisible()).toBe(true);
    },
    T,
  );

  test(
    "bind:value follows typing, and a state write reaches the field",
    async () => {
      await app.locator("#name").type("yuuki");
      await expect(app.locator("#echo")).toHaveText("hello yuuki");
      await app.locator("#reset").click();
      await expect(app.locator("#echo")).toHaveText("hello reset");
      bunExpect(await app.eval(`document.querySelector("#name").value`)).toBe("reset");
    },
    T,
  );

  test(
    "a delegated click on the <a> runs its handler",
    async () => {
      await app.locator("#link").click();
      await expect(app.locator("#link")).toHaveText("link 1");
    },
    T,
  );

  test(
    "nothing threw or was advised",
    async () => {
      const { entries } = await app.logs(0);
      bunExpect(entries.filter((e) => e.text.includes("[skathi compat]")).map((e) => e.text)).toEqual([]);
      await app.expectNoErrors();
    },
    T,
  );
});
