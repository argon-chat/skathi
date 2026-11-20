import { afterAll, expect, test } from "bun:test";
import { expect as ui, launch } from "../src/index";

const app = await launch({ app: "packages/test/fixtures/svelte-renderer/src/main.ts", label: "svelte-renderer" });
afterAll(() => app.close());

test("native Svelte moves keyed fragments, bubbles clicks and renders snippets", async () => {
  await ui(app.locator("h1")).toHaveText("Native renderer!");
  expect(await app.eval('document.querySelector("#gradient") instanceof SVGElement')).toBe(true);
  expect(await app.eval('document.querySelector("#disabled").hasAttribute("disabled")')).toBe(true);
  await ui(app.locator("sk-svelte-host .dom-value")).toHaveText("mixed");
  await app.locator(".dom-field").press("End");
  await app.locator(".dom-field").type(" input");
  await ui(app.locator(".dom-value")).toHaveText("mixed input");
  await app.eval('globalThis.original = document.querySelector(".item")');
  await app.locator("#add").click();
  await ui(app.locator("#clicks")).toHaveText("1");
  expect(await app.eval('document.querySelector("#disabled").hasAttribute("disabled")')).toBe(false);
  expect(await app.eval('Array.from(document.querySelectorAll(".item"), el => el.dataset.name).join("")')).toBe("BCA");
  expect(await app.eval('document.querySelectorAll(".item")[2] === original')).toBe(true);
  await app.locator("#toggle").click();
  await ui(app.locator(".detail")).toHaveText("Details open");
  await app.locator("#toggle").click();
  expect(await app.eval('document.querySelector(".detail")')).toBeNull();
  await app.locator("#window").click();
  await ui(app.locator(".detail")).toHaveText("In popout");
  await app.locator("#window").click();
  expect(await app.eval('document.querySelector(".detail")')).toBeNull();
}, 30_000);
