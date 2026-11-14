import { afterEach, expect, test } from "bun:test";
import type { HotkeyBinding, HotkeyEvent } from "../src/hotkeys";

const globals = globalThis as Record<string, unknown>;
let sequence = 0;
const load = async (): Promise<typeof import("../src/hotkeys")> =>
  import(`../src/hotkeys.ts?hotkeys-case=${++sequence}`);
afterEach(() => {
  delete globals.__sk_appHotkeys;
});

test("hotkeys pass bindings and listeners to the native object unchanged", async () => {
  const received: unknown[] = [];
  let fire!: (event: HotkeyEvent) => void;
  const off = () => received.push("off");
  const chord = {
    buttons: [
      { device: 0, code: 0xa2 },
      { device: 1, code: 4 },
    ],
  };
  const status = { running: true, platform: "windows", accessibilityGranted: true, error: null, restarts: 0 };
  globals.__sk_appHotkeys = {
    available: true,
    setBindings: (bindings: readonly HotkeyBinding[]) => {
      received.push(bindings);
      return true;
    },
    status: () => status,
    requestAccessibility: () => true,
    restart: () => true,
    captureOnce: () => Promise.resolve(chord),
    cancelCapture: () => received.push("cancel"),
    onFired: (listener: typeof fire) => {
      fire = listener;
      return off;
    },
  };
  const { appHotkeys } = await load();
  const bindings = [
    {
      id: "talk",
      chord,
      suppress: true,
      toJSON(): never {
        throw new Error("bindings must never be serialized");
      },
    },
  ];
  expect(appHotkeys.available).toBe(true);
  expect(appHotkeys.setBindings(bindings)).toBe(true);
  expect(received[0]).toBe(bindings);
  expect(appHotkeys.status()).toBe(status);
  expect(appHotkeys.requestAccessibility()).toBe(true);
  expect(appHotkeys.restart()).toBe(true);
  expect(await appHotkeys.captureOnce()).toBe(chord);
  appHotkeys.cancelCapture();
  const phases: string[] = [];
  const unsubscribe = appHotkeys.onFired((event) => phases.push(`${event.id}:${event.phase}`));
  fire({ id: "talk", phase: "down" });
  fire({ id: "talk", phase: "up" });
  expect(phases).toEqual(["talk:down", "talk:up"]);
  expect(unsubscribe).toBe(off);
  unsubscribe();
  expect(received.slice(1)).toEqual(["cancel", "off"]);
  expect(Object.isFrozen(appHotkeys)).toBe(true);
});

test("native capture rejections reach the caller as they are", async () => {
  const error = new Error("hotkey capture superseded");
  globals.__sk_appHotkeys = { available: true, captureOnce: () => Promise.reject(error) };
  const { appHotkeys } = await load();
  await expect(appHotkeys.captureOnce()).rejects.toBe(error);
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
