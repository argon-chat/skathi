import { afterEach, expect, test } from "bun:test";

const globals = globalThis as Record<string, unknown>;
let sequence = 0;
const load = async (): Promise<typeof import("../src/hotkeys")> =>
  import(`../src/hotkeys.ts?hotkeys-case=${++sequence}`);
afterEach(() => {
  delete globals.__sk_appHotkeys;
});

test("a host without hotkeys reports it and still allows clearing", async () => {
  const { appHotkeys } = await load();
  expect(appHotkeys.available).toBe(false);
  expect(appHotkeys.setBindings([])).toBe(false);
  expect(appHotkeys.status().running).toBe(false);
  expect(appHotkeys.requestAccessibility()).toBe(false);
  expect(() => appHotkeys.cancelCapture()).not.toThrow();
  expect(typeof appHotkeys.onFired(() => {})).toBe("function");
  for (const call of [
    () => appHotkeys.setBindings([{ id: "x", chord: { buttons: [{ device: 0, code: 65 }] } }]),
    () => appHotkeys.restart(),
  ]) {
    try {
      call();
      throw new Error("expected unsupported");
    } catch (error) {
      expect((error as Error).name).toBe("NotSupportedError");
    }
  }
  const failure = await appHotkeys.captureOnce().catch((error: Error) => error);
  expect((failure as Error).name).toBe("NotSupportedError");
});
