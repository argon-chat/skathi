// When the same platform event produces a SECOND dispatch first, that dispatch settles the owed
// checkpoint before it runs.

import { afterAll, expect, test } from "bun:test";
import { launch, MACOS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/event-order/src/main.ts",
  window: [480, 420],
  label: "event-order",
});
afterAll(() => app.close());

const T = 30_000;

async function readLog(): Promise<string[]> {
  const r = await app.eval("JSON.stringify(__SK_TEST_ORDER__.log)");
  return JSON.parse(String(r?.value ?? r)) as string[];
}

async function logOf(action: () => Promise<unknown>, want: string[]): Promise<string[]> {
  await app.eval("__SK_TEST_ORDER__.clear()");
  await action();
  const deadline = Date.now() + 5_000;
  for (;;) {
    const log = await readLog();
    if (want.every((w) => log.includes(w) && log.includes(`${w}:mt`)) || Date.now() > deadline) return log;
    await Bun.sleep(50);
  }
}

function expectCheckpointed(log: string[], events: string[]) {
  expect(log).toEqual(events.flatMap((e) => [e, `${e}:mt`]));
}

// macOS CI: hover, focus and popups do not follow the harness's synthetic input.
test.skipIf(MACOS)(
  "one move: mouseleave, then mouseenter",
  async () => {
    await app.locator("#h1").hover();
    const log = await logOf(() => app.locator("#h2").hover(), ["leave-h1", "enter-h2"]);
    // gpui decides which of the two runs first; each has its checkpoint.
    const events = log.filter((e) => !e.endsWith(":mt"));
    expect([...events].sort()).toEqual(["enter-h2", "leave-h1"]);
    expectCheckpointed(log, events);
  },
  T,
);

test.skipIf(MACOS)(
  "one focus move: blur, then focus",
  async () => {
    await app.locator("#f1").focus();
    await Bun.sleep(100);
    const log = await logOf(() => app.locator("#f2").focus(), ["blur-f1", "focus-f2"]);
    expectCheckpointed(log, ["blur-f1", "focus-f2"]);
  },
  T,
);

test.skipIf(MACOS)(
  "Ctrl+C on a focusable box: keydown, then the document's copy",
  async () => {
    const log = await logOf(() => app.locator("#pad").press("Ctrl+C"), ["keydown", "copy", "keyup"]);
    expectCheckpointed(log, ["keydown", "copy", "keyup"]);
  },
  T,
);

// macOS: copy is Cmd+C, not Ctrl+C.
test.skipIf(MACOS)(
  "Ctrl+C in a text field: its keydown, then its own copy",
  async () => {
    const log = await logOf(() => app.locator("#ta").press("Ctrl+C"), ["keydown-ta", "copy-ta"]);
    expectCheckpointed(log, ["keydown-ta", "copy-ta"]);
  },
  T,
);

test.skipIf(MACOS)(
  "the Menu key: keyup, then contextmenu",
  async () => {
    const log = await logOf(() => app.locator("#pad").press("Menu"), ["keydown", "keyup", "contextmenu"]);
    expectCheckpointed(log, ["keydown", "keyup", "contextmenu"]);
  },
  T,
);

test(
  "Enter on a button clicks on the press, Space on the release",
  async () => {
    // Chromium: Enter's click comes from its keypress, before keyup; Space's is the keyup's default
    // action, after its handlers.
    const enter = await logOf(() => app.locator("#btn").press("Enter"), ["keyup-btn", "click-btn"]);
    expectCheckpointed(enter, ["click-btn", "keyup-btn"]);
    const space = await logOf(() => app.locator("#btn").press("Space"), ["keyup-btn", "click-btn"]);
    expectCheckpointed(space, ["keyup-btn", "click-btn"]);
  },
  T,
);

test(
  "a div role=button that handles its own keys acts once, with no click",
  async () => {
    // A browser clicks only an element with an activation behaviour; a click here as well would run
    // a menu item's action twice.
    const enter = await logOf(() => app.locator("#role-btn").press("Enter"), ["act-role"]);
    const space = await logOf(() => app.locator("#role-btn").press("Space"), ["act-role"]);
    await Bun.sleep(150);
    expect([enter, space].map((l) => l.filter((e) => !e.endsWith(":mt")))).toEqual([["act-role"], ["act-role"]]);
    expect((await readLog()).filter((e) => e.startsWith("click"))).toEqual([]);
  },
  T,
);

test(
  "a cancelled Enter keydown or Space keyup clicks nothing",
  async () => {
    const enter = await logOf(() => app.locator("#btn-x").press("Enter"), ["keydown-x"]);
    const space = await logOf(() => app.locator("#btn-x").press("Space"), ["keyup-x"]);
    await Bun.sleep(150);
    expect([enter, space, await readLog()].flat().filter((e) => e.startsWith("click"))).toEqual([]);
  },
  T,
);

test(
  "one wheel over a scroll container: wheel, then scroll",
  async () => {
    const log = await logOf(() => app.send("wheel", { selector: "#scroller", dy: 40 }), ["wheel", "scroll"]);
    expectCheckpointed(log, ["wheel", "scroll"]);
  },
  T,
);

test(
  "a cancelled wheel scrolls nothing, on the container or on a box around it",
  async () => {
    const TOP = (sel: string) => `String(document.querySelector('${sel}').scrollTop)`;
    for (const [sel, wheel] of [
      ["#scroller-x", "wheel-x"],
      ["#scroller-in", "wheel-guard"],
    ] as const) {
      const log = await logOf(() => app.send("wheel", { selector: sel, dy: 40 }), [wheel]);
      await Bun.sleep(150);
      const after = await readLog();
      expect({ sel, log: after.filter((e) => !e.endsWith(":mt")) }).toEqual({ sel, log: [wheel] });
      expect(log[0]).toBe(wheel);
      const top = await app.eval(TOP(sel));
      expect(`${sel}: ${String(top?.value ?? top)}`).toBe(`${sel}: 0`);
    }
  },
  T,
);

test(
  "no errors on the way",
  async () => {
    await app.expectNoErrors();
  },
  T,
);
