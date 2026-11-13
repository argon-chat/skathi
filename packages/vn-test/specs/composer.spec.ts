import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/composer/src/main.ts",
  window: [480, 320],
  label: "composer",
});
afterAll(() => app.close());

const T = 30_000;

async function read(js: string): Promise<string> {
  const r = await app.eval(js);
  return String(r?.value ?? r);
}

async function until(js: string, want: string): Promise<string> {
  const deadline = Date.now() + 3_000;
  for (;;) {
    const got = await read(js);
    if (got === want || Date.now() > deadline) return got;
    await Bun.sleep(50);
  }
}

const VALUE = "JSON.stringify(document.querySelector('#composer').value)";

test(
  "Enter-to-send: preventDefault, send, clear — no newline is left",
  async () => {
    await app.eval("__VN_TEST_COMPOSER__.reset('send')");
    await app.locator("#composer").type("abc");
    await app.locator("#composer").press("Enter");
    expect(await until("JSON.stringify(__VN_TEST_COMPOSER__.state.sent)", '["abc"]')).toBe('["abc"]');
    expect(await until(VALUE, '""')).toBe('""');
  },
  T,
);

test(
  "Enter that clears WITHOUT preventDefault: cleared first, then the newline",
  async () => {
    await app.eval("__VN_TEST_COMPOSER__.reset('clear')");
    await app.locator("#composer").type("abc");
    await app.locator("#composer").press("Enter");
    expect(await until(VALUE, JSON.stringify("\n"))).toBe(JSON.stringify("\n"));
  },
  T,
);

test(
  "a keydown that rewrites the value: the key lands in what it wrote",
  async () => {
    await app.eval("__VN_TEST_COMPOSER__.reset('upper')");
    await app.locator("#composer").type("abc");
    await app.locator("#composer").press("x");
    expect(await until(VALUE, '"ABCx"')).toBe('"ABCx"');
  },
  T,
);

test(
  "a press whose handler hides its own target from a microtask: nothing is left focused",
  async () => {
    await app.eval("__VN_TEST_COMPOSER__.reset('send')");
    await app.locator("#other").focus();
    await Bun.sleep(100);
    await app.eval("__VN_TEST_COMPOSER__.state.focusLog.length = 0");
    await app.locator("#victim").click({ real: true });
    expect(await until("String(!!document.querySelector('#victim'))", "false")).toBe("false");
    await Bun.sleep(150);
    // A browser runs the checkpoint before the press's default focus, finds the target gone and
    // focuses nothing: #other loses focus either way.
    const active = await read("String(document.activeElement && document.activeElement.id)");
    const log = await read("JSON.stringify(__VN_TEST_COMPOSER__.state.focusLog)");
    console.log("victim:", active, log);
    expect(["other", "victim"]).not.toContain(active);
  },
  T,
);

test(
  "a press cancelled with @mousedown.prevent keeps the caret in the composer",
  async () => {
    await app.eval("__VN_TEST_COMPOSER__.reset('send')");
    await app.locator("#composer").click({ real: true });
    await app.locator("#composer").type("ab");
    const ACTIVE = "String(document.activeElement && document.activeElement.id)";
    expect(await until(ACTIVE, "composer")).toBe("composer");
    for (const sel of ["#keep", "#keep-plain"]) {
      await app.locator(sel).click({ real: true });
      await Bun.sleep(150);
      expect(`${sel}: ${await read(ACTIVE)}`).toBe(`${sel}: composer`);
    }
    await app.locator("#composer").press("c");
    expect(await until(VALUE, '"abc"')).toBe('"abc"');
  },
  T,
);

test(
  "no errors on the way",
  async () => {
    await app.expectNoErrors();
  },
  T,
);
