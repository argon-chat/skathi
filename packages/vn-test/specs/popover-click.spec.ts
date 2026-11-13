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

async function alive() {
  // The abort came from the settle deferred after gpui's dispatch, well inside a second.
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
  expect(s.active).toEqual(["content"]);
  const after = await state();
  expect(after.focusLog).toEqual(["focus-member", "blur-member", "focus-content"]);
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
