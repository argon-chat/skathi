// The layout and paint shapes the m26 client sweep found broken in the Argon
// client (research/reports/m26/client-sweep-1.md; the fixes and their reasons
// are in research/reports/m26/fixes-layout-1.md).
//
//   bun test packages/vn-test/specs/layout.spec.ts
//
// `fixtures/layout` holds one section per finding, each spelled the way the
// client spells it.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, type Bounds } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/layout/src/main.ts",
  window: [720, 520],
  label: "layout",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** A node's laid-out box — `null` when it has none (`display: none`). */
const box = async (selector: string): Promise<Bounds | null> => {
  const { nodes } = await app.query(selector);
  return nodes[0]?.bounds ?? null;
};

/** Polls `read` until `ok` holds, and returns the last answer either way. */
async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const shown = (b: Bounds | null) => !!b && b.w > 0 && b.h > 0;

describe("#1 group-hover that changes `display`", () => {
  test(
    "a scroll under a still pointer neither crashes the host nor leaves the wrong row's timestamp",
    async () => {
      // `.group-msg:hover .group-msg-hover-flex { display: flex }` over a
      // `display: none` box — Tailwind's `hidden group-hover/msg:flex`, the
      // chat row's timestamp (MessageItem.vue:64-69).
      await app.locator("#msg-2").hover();
      bunExpect(shown(await until(() => box("#ts-2"), shown))).toBe(true);

      // The content moves under the pointer with no mouse move. The host used
      // to panic in paint ("must call prepaint before paint", gpui
      // element.rs:557): prepaint styled the newly hovered row from the stored
      // hover flag (hidden), paint from the live hit test (shown).
      for (let step = 1; step <= 6; step++) {
        await app.locator("#msgs").scroll(40);
        await sleep(120);
        bunExpect(await app.eval("1 + 1")).toBe(2);
      }

      // The row under the pointer now is #8 (two rows down, then six more at
      // 40 px each) — its timestamp is laid out, and the first one's is not.
      const under = await until(() => box("#ts-8"), shown);
      bunExpect(shown(under)).toBe(true);
      bunExpect(under?.w).toBe(36);
      bunExpect(shown(await until(() => box("#ts-2"), (b) => !shown(b)))).toBe(false);
    },
    T,
  );
});
