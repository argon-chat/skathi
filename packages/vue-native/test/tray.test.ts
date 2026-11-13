import { afterEach, expect, test } from "bun:test";
import type { TrayOptions } from "../src/tray";

const globals = globalThis as Record<string, unknown>;
let sequence = 0;
const load = async (): Promise<typeof import("../src/tray")> => import(`../src/tray.ts?tray-case=${++sequence}`);
afterEach(() => {
  delete globals.__vn_appTray;
});

test("tray passes functions and typed pixels directly without serialization", async () => {
  const received: Array<TrayOptions | null> = [];
  globals.__vn_appTray = { available: true, set: (options: TrayOptions | null) => received.push(options) };
  const { appTray } = await load();
  let selected = 0;
  const onSelect = () => selected++;
  const options = {
    tooltip: "A & B 😀",
    icon: { width: 1, height: 1, data: new Uint8Array([255, 0, 0, 128]) },
    menu: [{ label: "Action", onSelect }],
    toJSON(): never {
      throw new Error("tray options must never be serialized");
    },
  };
  expect(appTray.available).toBe(true);
  expect(appTray.set(options)).toBeUndefined();
  expect(received[0]).toBe(options);
  expect(received[0]?.icon).toBe(options.icon);
  const item = received[0]?.menu?.[0];
  if (item && "onSelect" in item) item.onSelect?.();
  expect(selected).toBe(1);
  appTray.set(null);
  expect(received).toHaveLength(2);
  expect(received[1]).toBeNull();
});

test("a host without tray support reports it and still allows cleanup", async () => {
  const { appTray } = await load();
  expect(appTray.available).toBe(false);
  expect(() => appTray.set(null)).not.toThrow();
  try {
    appTray.set({ tooltip: "Unavailable" });
    throw new Error("expected unsupported");
  } catch (error) {
    expect((error as Error).name).toBe("NotSupportedError");
  }
});

test("native schema errors remain synchronous and preserve the native exception", async () => {
  const error = new TypeError("tray tooltip must be at most 127 UTF-16 units");
  globals.__vn_appTray = {
    available: true,
    set: () => {
      throw error;
    },
  };
  const { appTray } = await load();
  expect(() => appTray.set({ tooltip: "x".repeat(128) })).toThrow(error);
});
