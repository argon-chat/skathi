// `location.reload()` on the real host and engine (m26 client sweep #3,
// docs/RUNTIME.md §5.2): the app runs again in the SAME JavaScript realm, and
// the runtime that comes up must not run on its predecessor's classes.
//
//   bun test packages/vn-test/specs/reload.spec.ts
//
// test/runtime-reload.test.ts pins the bookkeeping in a fake global. This spec
// checks the same things against JavaScriptCore's real global object, where
// key enumeration and `delete` are the engine's: after a reload,
// ResizeObserver is a new class and fires, and a global the old app added is
// gone. Any fixture will do; this one borrows `events`.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "reload",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const js = async (src: string) => app.eval(src);

/** Polls `cond` (a JS expression) until it is true. */
async function until(cond: string): Promise<boolean> {
  for (let i = 0; i < 100; i++) {
    if ((await js(`String(!!(${cond}))`)) === "true") return true;
    await sleep(100);
  }
  return false;
}

test("after location.reload() the runtime's classes are new, ResizeObserver fires, and the old app's globals are gone", async () => {
  await until("document.querySelector('#trigger')");
  // The old app: a global, a symbol-keyed flag (Mediabunny's shape), and a
  // note of the classes it ran on. `__VN_*` names survive a reload by design.
  await js(`(() => {
    globalThis.oldAppGlobal = 1;
    globalThis[Symbol.for("vn-test loaded")] = true;
    globalThis.__VN_TEST_OLD__ = { RO: ResizeObserver, MO: MutationObserver, Window: Window };
    location.reload();
    return "asked";
  })()`);
  expect(await until("typeof oldAppGlobal === 'undefined' && document.querySelector('#trigger')")).toBe(true);
  expect(await js("String(globalThis[Symbol.for('vn-test loaded')])")).toBe("undefined");
  expect(
    await js("[ResizeObserver !== __VN_TEST_OLD__.RO, MutationObserver !== __VN_TEST_OLD__.MO, Window !== __VN_TEST_OLD__.Window, window.constructor === Window].join()"),
  ).toBe("true,true,true,true");
  // The observer that stopped firing after a reload, before the fix.
  await js(`(() => {
    globalThis.__VN_TEST_RO_SEEN__ = 0;
    new ResizeObserver(() => { __VN_TEST_RO_SEEN__++; }).observe(document.querySelector("#trigger"));
    return "observing";
  })()`);
  expect(await until("__VN_TEST_RO_SEEN__ > 0")).toBe(true);
  const { entries } = await app.logs(0);
  expect(entries.some((e) => e.text.includes("re-runs the app in the SAME JavaScript realm"))).toBe(true);
}, T);
