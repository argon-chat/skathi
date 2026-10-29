// One platform event, two JS events, and the microtask checkpoint between them
// (m26 client sweep #8, docs/RUNTIME.md §5.8).
//
//   bun test packages/vn-test/specs/event-order.spec.ts
//
// A dispatch made under a gpui listener leaves its checkpoint owed, and the
// host settles it after gpui's dispatch, where a layout read may lay out. When
// the same platform event produces a SECOND dispatch first, that dispatch
// settles the owed checkpoint before it runs. So each event's microtasks still
// run before the next event's handlers, as in a browser and as before #8.
// These are the seven pairs where one platform event produces two dispatches.
// Every handler in `fixtures/event-order` logs its event and then, from a
// microtask, `<event>:mt`.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/event-order/src/main.ts",
  window: [480, 420],
  label: "event-order",
});
afterAll(() => app.close());

const T = 30_000;

async function readLog(): Promise<string[]> {
  const r = await app.eval("JSON.stringify(__VN_TEST_ORDER__.log)");
  return JSON.parse(String(r?.value ?? r)) as string[];
}

/** Clears the log, acts, and returns the log once every `want` event and its microtask are in. */
async function logOf(action: () => Promise<unknown>, want: string[]): Promise<string[]> {
  await app.eval("__VN_TEST_ORDER__.clear()");
  await action();
  const deadline = Date.now() + 5_000;
  for (;;) {
    const log = await readLog();
    if (want.every((w) => log.includes(w) && log.includes(`${w}:mt`)) || Date.now() > deadline) return log;
    await Bun.sleep(50);
  }
}

/** The events in order, each followed at once by its own microtask. */
function expectCheckpointed(log: string[], events: string[]) {
  expect(log).toEqual(events.flatMap((e) => [e, `${e}:mt`]));
}

test("one move: mouseleave, then mouseenter", async () => {
  await app.locator("#h1").hover();
  const log = await logOf(() => app.locator("#h2").hover(), ["leave-h1", "enter-h2"]);
  // gpui decides which of the two runs first; each has its checkpoint.
  const events = log.filter((e) => !e.endsWith(":mt"));
  expect([...events].sort()).toEqual(["enter-h2", "leave-h1"]);
  expectCheckpointed(log, events);
}, T);

test("one focus move: blur, then focus", async () => {
  await app.locator("#f1").focus();
  await Bun.sleep(100);
  const log = await logOf(() => app.locator("#f2").focus(), ["blur-f1", "focus-f2"]);
  expectCheckpointed(log, ["blur-f1", "focus-f2"]);
}, T);

test("Ctrl+C on a focusable box: keydown, then the document's copy", async () => {
  const log = await logOf(() => app.locator("#pad").press("Ctrl+C"), ["keydown", "copy", "keyup"]);
  expectCheckpointed(log, ["keydown", "copy", "keyup"]);
}, T);

test("Ctrl+C in a text field: its keydown, then its own copy", async () => {
  const log = await logOf(() => app.locator("#ta").press("Ctrl+C"), ["keydown-ta", "copy-ta"]);
  expectCheckpointed(log, ["keydown-ta", "copy-ta"]);
}, T);

test("the Menu key: keyup, then contextmenu", async () => {
  const log = await logOf(() => app.locator("#pad").press("Menu"), ["keydown", "keyup", "contextmenu"]);
  expectCheckpointed(log, ["keydown", "keyup", "contextmenu"]);
}, T);

test("Enter on a button: the keyboard click and keyup, from one key-up", async () => {
  // gpui's keyboard click runs before the element's own key-up listener, in
  // the same dispatch. Chromium clicks on Enter's keydown, so its order is the
  // same (click before keyup), only earlier.
  const log = await logOf(() => app.locator("#btn").press("Enter"), ["keyup-btn", "click-btn"]);
  expectCheckpointed(log, ["click-btn", "keyup-btn"]);
}, T);

test("one wheel over a scroll container: scroll, then wheel", async () => {
  // Today's order, which is not a browser's: the container scrolls in gpui's
  // capture phase and reports `scroll` before the bubble phase's `wheel`.
  const log = await logOf(() => app.send("wheel", { selector: "#scroller", dy: 40 }), ["scroll", "wheel"]);
  expectCheckpointed(log, ["scroll", "wheel"]);
}, T);

test("no errors on the way", async () => {
  await app.expectNoErrors();
}, T);
