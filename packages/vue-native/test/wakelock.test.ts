import { expect, test } from "bun:test";
import { installWakeLock } from "../src/wakelock";

class FakeDOMException extends Error {
  constructor(message?: string, name?: string) {
    super(message);
    this.name = name ?? "Error";
  }
}

function setup(refuse = false) {
  const holds: boolean[] = [];
  let hidden = false;
  let onHidden = (): void => {};
  const nav = {} as {
    wakeLock: {
      request(type?: unknown): Promise<EventTarget & { released: boolean; type: string; release(): Promise<void> }>;
    };
  };
  installWakeLock(nav, {
    host: {
      hold: (on) => {
        holds.push(on);
        return !(refuse && on);
      },
    },
    engine: { EventTarget, Event, DOMException: FakeDOMException },
    isHidden: () => hidden,
    onHidden: (fn) => void (onHidden = fn),
  });
  return {
    nav,
    holds,
    hide: () => {
      hidden = true;
      onHidden();
    },
  };
}

test("one system request while any sentinel is active", async () => {
  const { nav, holds } = setup();
  const a = await nav.wakeLock.request("screen");
  const b = await nav.wakeLock.request();
  expect([a.type, a.released]).toEqual(["screen", false]);
  expect(holds).toEqual([true]);
  const fired: string[] = [];
  a.addEventListener("release", (e) => fired.push(e.type));
  await a.release();
  await a.release();
  expect([a.released, fired.length, holds]).toEqual([true, 1, [true]]);
  await b.release();
  expect(holds).toEqual([true, false]);
});

test("hiding the page releases every sentinel; a hidden page cannot ask", async () => {
  const { nav, holds, hide } = setup();
  const a = await nav.wakeLock.request();
  hide();
  expect([a.released, holds]).toEqual([true, [true, false]]);
  await expect(nav.wakeLock.request()).rejects.toMatchObject({ name: "NotAllowedError" });
});

test("a wrong type is a TypeError and a refusal a NotAllowedError", async () => {
  await expect(setup().nav.wakeLock.request("system")).rejects.toBeInstanceOf(TypeError);
  await expect(setup(true).nav.wakeLock.request()).rejects.toMatchObject({ name: "NotAllowedError" });
  expect(() => new (globalThis as unknown as { WakeLockSentinel: new () => unknown }).WakeLockSentinel()).toThrow(
    TypeError,
  );
});
