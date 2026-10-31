// A real click that opens a reka-style popover, which focuses itself and
// measures on mount (fixtures/popover-click): the Argon member click that
// aborted the host.
//
//   bun test packages/vn-test/specs/popover-click.spec.ts
//
// The press focuses the trigger, and gpui reports that from the next draw. The
// popover's onMounted runs in JSC's own drain at the end of the click's call,
// and its layout reads force that draw. So the trigger's `focus` was
// dispatched inside the running script. It owed a checkpoint there, kept the
// engine lock past the drain's release of the heap access, and the next
// release aborted the process: "FATAL: Attempting to release access but the
// mutator does not have access" (crates/vn-runtime/src/bridge.rs `owe_drain`).

import { afterAll, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/popover-click/src/main.ts",
  window: [480, 360],
  label: "popover-click",
});
afterAll(() => app.close());

const T = 30_000;

type State = {
  opened: number;
  measured: { trigger: number[]; content: number[] }[];
  focusLog: string[];
  active: string[];
};

async function state(): Promise<State> {
  const r = await app.eval("JSON.stringify(__VN_TEST_POPOVER__)");
  return JSON.parse(String(r?.value ?? r)) as State;
}

async function opened(n: number): Promise<State> {
  const deadline = Date.now() + 5_000;
  for (;;) {
    const s = await state();
    if (s.opened >= n || Date.now() > deadline) return s;
    await Bun.sleep(50);
  }
}

/** The host is still running, and its log has no engine abort. */
async function alive() {
  // The abort came from the settle deferred after gpui's dispatch, well
  // inside a second.
  await Bun.sleep(1_500);
  expect(readFileSync(app.hostLog, "utf8")).not.toContain("FATAL");
  expect(app.child.exitCode).toBeNull();
  expect(await app.eval("1 + 1")).toBeTruthy();
}

test("a real click opens the popover, which focuses and measures itself, and the host lives", async () => {
  await app.locator("#member").click({ real: true });
  const s = await opened(1);
  expect(s.opened).toBe(1);
  await alive();
  // The press focused the trigger, and the popover then took focus.
  expect(s.active).toEqual(["content"]);
  const after = await state();
  expect(after.focusLog).toEqual(["focus-member", "blur-member", "focus-content"]);
  // Measured on mount: the trigger's real box, and the content's.
  const { trigger, content } = s.measured[0]!;
  expect(trigger[2]).toBe(160);
  expect(trigger[3]).toBe(36);
  expect(content[2]).toBe(200);
  expect(content[3]).toBeGreaterThan(0);
}, T);

test("closing and opening it again from real clicks", async () => {
  await app.locator("#member").click({ real: true });
  await app.locator("#member").click({ real: true });
  const s = await opened(2);
  expect(s.opened).toBe(2);
  await alive();
  expect(s.measured[1]!.trigger[2]).toBe(160);
}, T);
