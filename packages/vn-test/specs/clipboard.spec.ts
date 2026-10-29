// `copy` / `cut` / `paste` from real keystrokes (m26 fixes, docs/RUNTIME.md
// §5.4, crates/vn-tree/src/clipboard.rs).
//
//   bun test packages/vn-test/specs/clipboard.spec.ts
//
// The keys go through devtools `key`, which is gpui's own keystroke dispatch:
// the field's keydown interceptor, its key bindings (Ctrl+V, Shift+Insert…)
// and the root's key listener, exactly as a physical key. `fixtures/clipboard`
// never writes the system clipboard (see its header), so the spec is safe to
// run while somebody uses the machine; what the clipboard HOLDS is unknown,
// so the assertions are about the events and their stores, not the text.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/clipboard/src/main.ts",
  window: [420, 200],
  label: "clipboard",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The events recorded since the last call. */
let seen = 0;
async function fresh(): Promise<string[]> {
  await sleep(150);
  const all = JSON.parse((await app.eval("JSON.stringify(__VN_TEST_CLIP__.events)")) as string) as string[];
  const out = all.slice(seen);
  seen = all.length;
  return out;
}

const SHAPE = "event=true,dt=true,bubbles=true,cancelable=true";

describe("ClipboardEvent from the keyboard", () => {
  test("Ctrl+V in a field: a read-only paste at the field, then document; cancelling it inserts nothing", async () => {
    await expect(app.locator("#field")).toHaveCount(1);
    await fresh();
    await app.locator("#field").press("Ctrl+V");
    const got = await fresh();
    bunExpect(got).toEqual([`paste@field|${SHAPE},writable=false`, "paste@document|target=field", "after:paste|types=0"]);
    await expect(app.locator("#field")).toHaveText("");
  }, T);

  test("Shift+Insert is a paste too", async () => {
    await app.locator("#field").press("Shift+Insert");
    bunExpect((await fresh())[0]).toBe(`paste@field|${SHAPE},writable=false`);
  }, T);

  test("Ctrl+C and Ctrl+Insert: a writable copy at the field", async () => {
    await app.locator("#field").press("Ctrl+C");
    await app.locator("#field").press("Ctrl+Insert");
    bunExpect(await fresh()).toEqual([
      `copy@field|${SHAPE},writable=true`,
      "after:copy|types=0",
      `copy@field|${SHAPE},writable=true`,
      "after:copy|types=0",
    ]);
  }, T);

  test("a cancelled keydown has no cut; Shift+Delete is a cut", async () => {
    await app.locator("#field").press("Ctrl+X");
    bunExpect(await fresh()).toEqual(["keydown@field|cancelled ctrl+x"]);
    await app.locator("#field").press("Shift+Delete");
    bunExpect(await fresh()).toEqual([`cut@field|${SHAPE},writable=true`, "after:cut|types=0"]);
  }, T);

  test("outside a text field the keys fire at the focused element", async () => {
    await app.locator("#pad").press("Ctrl+V");
    bunExpect(await fresh()).toEqual([`paste@pad|${SHAPE},writable=false`, "paste@document|target=pad", "after:paste|types=0"]);
    await app.locator("#pad").press("Ctrl+C");
    bunExpect(await fresh()).toEqual([`copy@pad|${SHAPE},writable=true`, "after:copy|types=0"]);
  }, T);
});
