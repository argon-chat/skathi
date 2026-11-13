import { expect, test } from "bun:test";
import { installBadge } from "../src/badge";

test("setAppBadge sends a flag, a count or a clear; bad counts reject", async () => {
  const sent: number[] = [];
  const nav = {} as { setAppBadge(c?: unknown): Promise<void>; clearAppBadge(): Promise<void> };
  installBadge(nav, { set: (c) => void sent.push(c) });
  await nav.setAppBadge();
  await nav.setAppBadge(3.7);
  await nav.setAppBadge(0);
  await nav.clearAppBadge();
  expect(sent).toEqual([-1, 3, 0, 0]);
  await expect(nav.setAppBadge(-1)).rejects.toBeInstanceOf(TypeError);
  await expect(nav.setAppBadge(NaN)).rejects.toBeInstanceOf(TypeError);
  await expect(nav.setAppBadge(Infinity)).rejects.toBeInstanceOf(TypeError);
  expect(sent.length).toBe(4);
});
