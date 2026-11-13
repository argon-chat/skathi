import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/iframe/src/main.ts",
  window: [640, 480],
  label: "iframe",
  env: { VN_IFRAME: "off" },
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const state = async () => JSON.parse(String(await app.eval("__VN_TEST_IFRAME__.state()")));
const until = async (done: (s: Awaited<ReturnType<typeof state>>) => boolean) => {
  const end = Date.now() + 10_000;
  for (;;) {
    const s = await state();
    if (done(s) || Date.now() > end) return s;
    await new Promise((r) => setTimeout(r, 50));
  }
};

describe("<iframe> without a webview", () => {
  test(
    "the element exists and lays out as a replaced box, 300 × 150 by default (R1.1, R8.1)",
    async () => {
      await app.locator("#plain").waitFor();
      const s = await state();
      expect(s.supported).toBe(false);
      expect(s.plain).toEqual({ isIframe: true, tagName: "IFRAME", size: [300, 150], src: "", sandbox: "", contentDocument: null });
    },
    T,
  );

  test(
    "the host's own sequence: attributes reflect, the box is where CSS puts it, contentWindow is one object (R1.2, R1.3)",
    async () => {
      await app.eval("__VN_TEST_IFRAME__.mount('/games/none/index.html')");
      const s = await state();
      expect(s.frame.sandbox).toEqual(["allow-scripts allow-pointer-lock", 2, true, false, "allow-scripts allow-pointer-lock"]);
      expect(s.frame.sandboxAssign).toEqual(["allow-scripts", true, false]);
      expect(s.frame.attrs).toEqual(["fullscreen 'none'", "fullscreen 'none'", "origin", "origin", "eager", "/games/none/index.html"]);
      expect(s.frame.box).toEqual([20, 10, 320, 240]);
      expect(s.frame.sameWindow).toBe(true);
      expect(s.frame.windowKeys).toEqual(["function", "undefined", null]);
      expect(s.log[0]).toBe("detached contentWindow null");
    },
    T,
  );

  test(
    "a src is answered with one error, asynchronously, and never with load (R1.5, R8.1)",
    async () => {
      const s = await until((s) => s.log.includes("error"));
      expect(s.log.filter((l: string) => l === "error")).toEqual(["error"]);
      expect(s.log).not.toContain("load");
      await app.eval("__VN_TEST_IFRAME__.move()");
      await new Promise((r) => setTimeout(r, 300));
      expect((await state()).log.filter((l: string) => l === "error")).toEqual(["error"]);
    },
    T,
  );

  test(
    "postMessage to a frame with no document is dropped; what cannot be cloned still throws in the sender (R2.3)",
    async () => {
      expect(String(await app.eval("__VN_TEST_IFRAME__.post()"))).toBe(
        "star ok; options ok; function DataCloneError; node DataCloneError; transfer DataCloneError",
      );
      const s = await state();
      expect(s.log.some((l: string) => l.startsWith("message"))).toBe(false);
    },
    T,
  );

  test(
    "a removed element has no contentWindow (R1.3)",
    async () => {
      await app.eval("__VN_TEST_IFRAME__.unmount()");
      const s = await state();
      expect(s.log.at(-1)).toBe("removed contentWindow null");
    },
    T,
  );
});
