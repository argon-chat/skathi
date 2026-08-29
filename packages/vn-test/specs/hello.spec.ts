// The hello tour, driven end to end.
//
// One app, one window, five things a browser Vue app does that a native one has
// to do identically: route between tabs, keep a store, echo a v-model, run a
// native text editor, and hand a <KeepAlive>'d page back with its state intact.
//
//   bun test packages/vn-test/specs/hello.spec.ts
//
// The window opens. Nothing here focuses it, moves the cursor or sends global
// input — every gesture is a devtools command, which is why the suite can run
// while somebody is using the machine.

import { afterAll, describe, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({ app: "hello", window: [1100, 820], label: "hello" });
afterAll(() => app.close());

/** Generous: a cold `bun test` pays for the first frame of a real GPU app. */
const T = 30_000;

const tab = (i: number) => app.locator(".tab").nth(i);
const FORMS = 0;
const MOTION = 2;

describe("hello", () => {
  test("the shell mounted with all eleven tabs", async () => {
    await expect(app.locator(".tab")).toHaveCount(11);
    // The two tabs the rest of this file addresses by position — asserted by
    // NAME once, so a reordered nav fails here with a readable message instead
    // of three tests failing for a reason none of them mentions.
    await expect(tab(FORMS)).toHaveText("Forms");
    await expect(tab(MOTION)).toHaveText("Motion");
  }, T);

  test("clicking a tab moves the active class", async () => {
    await expect(tab(FORMS)).toHaveClass("router-link-active");
    await expect(tab(MOTION)).not.toHaveClass("router-link-active");

    await tab(MOTION).click();

    await expect(tab(MOTION)).toHaveClass("router-link-active");
    await expect(tab(FORMS)).not.toHaveClass("router-link-active");
    // …and the route actually rendered, not just the link restyled.
    await expect(app.locator(".card .title")).toContainText("Motion");

    await tab(FORMS).click();
    await expect(tab(FORMS)).toHaveClass("router-link-active");
  }, T);

  test("the pinia counter increments on click, and every tab sees it", async () => {
    const bump = app.locator(".store-row .btn").first();
    await expect(bump).toHaveText("visits++ (0)");

    await bump.click();
    await expect(bump).toHaveText("visits++ (1)");
    await bump.click();
    await bump.click();
    await expect(bump).toHaveText("visits++ (3)");

    // The store is read in the SHELL too, outside the routed page — the badge
    // is a computed inside the store, so this is one store instance or none.
    await expect(app.locator(".bar .store")).toHaveText("pinia · session · guest · 3");
  }, T);

  test("typing into the native editor echoes through v-model", async () => {
    const field = app.locator(".field").first();
    await field.type("yuuki");

    // `text` on an <input> reads the widget's live buffer, not a text node.
    await expect(field).toHaveText("yuuki");
    // …and the same characters came back out through Vue's reactivity.
    await expect(app.locator(".hint").first()).toHaveText('v-model echo: "yuuki" (5 chars)');
  }, T);

  test("preventDefault() on keydown still cancels a character", async () => {
    // FormsPage blocks digits from its own @keydown handler. A real keystroke
    // goes through the widget's interceptor, so the app's veto has to hold.
    const field = app.locator(".field").first();
    await field.type("7");
    await expect(field).toHaveText("yuuki");
    await expect(app.locator(".hint").nth(4)).toContainText("digits blocked by preventDefault: 1");
  }, T);

  test("KeepAlive hands the page back with its state", async () => {
    const field = app.locator(".field").first();
    const keep = app.locator(".hint.keep");
    // Read the count rather than assume it: the tests above already navigated,
    // and a spec that only passes as test #6 of 7 is a spec that will one day
    // be reordered into a lie.
    const before = Number(/fired (\d+)×/.exec(await keep.textContent())![1]);

    await tab(MOTION).click();
    // The cached subtree is DETACHED — it is not painted, so it has no bounds,
    // which is exactly how `toBeHidden` reads it.
    await expect(field).toBeHidden();

    await tab(FORMS).click();
    await expect(field).toBeVisible();
    await expect(field).toHaveText("yuuki"); // the editor buffer survived
    await expect(keep).toContainText(`onActivated fired ${before + 1}×`);
    // …and the store is untouched by the round trip.
    await expect(app.locator(".store-row .btn").first()).toHaveText("visits++ (3)");
  }, T);

  test("the app logged nothing broken", async () => {
    await app.expectNoErrors();
  }, T);
});
