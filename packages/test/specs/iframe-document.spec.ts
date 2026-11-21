import { afterAll, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/iframe/src/main.ts",
  window: [640, 480],
  label: "iframe-document",
});
let closed = false;
afterAll(async () => {
  if (!closed) await app.close();
});

const browserProcesses = (): number => {
  const out = spawnSync(
    "pwsh",
    [
      "-NoProfile",
      "-Command",
      `(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'sk-test-iframe' } | Measure-Object).Count`,
    ],
    { encoding: "utf8" },
  );
  return Number(out.stdout.trim());
};

const T = 60_000;
type State = { supported: boolean; log: string[]; frame: Record<string, unknown> | null };
const state = async (): Promise<State> => JSON.parse(String(await app.eval("__SK_TEST_IFRAME__.state()")));
const until = async (done: (s: State) => boolean, ms = 20_000) => {
  const end = Date.now() + ms;
  for (;;) {
    const s = await state();
    if (done(s) || Date.now() > end) return s;
    await new Promise((r) => setTimeout(r, 50));
  }
};
const messages = (s: State, type: string) =>
  s.log
    .filter((l) => l.startsWith("message "))
    .map((l) => {
      const [, who, origin, ...rest] = l.split(" ");
      return { who, origin, data: JSON.parse(rest.join(" ")) };
    })
    .filter((m) => m.data?.type === type);

await app.locator("#plain").waitFor();
const supported = (await state()).supported;
const it = supported ? test : test.skip;

describe("<iframe> with a document", () => {
  it(
    "the document loads from the app's files and says hello to its parent (R1.5, R2.2, R3.1)",
    async () => {
      await app.eval("__SK_TEST_IFRAME__.mount('/game.html')");
      const s = await until((s) => s.log.includes("load") && messages(s, "hello").length > 0);
      expect(s.log.filter((l) => l === "load")).toEqual(["load"]);
      expect(s.log).not.toContain("error");
      const hello = messages(s, "hello")[0]!;
      expect([hello.who, hello.origin]).toEqual(["mine", "null"]);
      expect([hello.data.framed, hello.data.origin]).toEqual([true, "null"]);
      expect(hello.data.size).toEqual([320, 240]);
    },
    T,
  );

  it(
    "messages go both ways, in order, with the types that cross (R2.1, R2.3, R2.4)",
    async () => {
      await app.eval(`(() => {
        const w = document.querySelector("#slot iframe").contentWindow;
        for (let n = 0; n < 20; n++) w.postMessage({ type: "ping", n }, "*");
        w.postMessage({ type: "rich", when: new Date(1700000000000), bytes: new Uint8Array([1, 2, 255]), set: new Set(["a"]), none: undefined, big: 12n }, "*");
      })()`);
      const s = await until((s) => messages(s, "echo").length >= 21);
      const echoes = messages(s, "echo");
      expect(echoes.slice(0, 20).map((m) => m.data.data.n)).toEqual([...Array(20).keys()]);
      expect(
        echoes.every((m) => m.who === "mine" && m.origin === "null" && m.data.sourceIsParent && m.data.originIsStable),
      ).toBe(true);
      expect(echoes[20]!.data.kinds).toEqual({
        type: "[object String]",
        when: "[object Date]",
        bytes: "[object Uint8Array]",
        set: "[object Set]",
        none: "[object Undefined]",
        big: "[object BigInt]",
      });
    },
    T,
  );

  it(
    "the sandbox is the browser's own: no way to the top document or the bridge (R4.2, R4.3, R4.4)",
    async () => {
      await app.eval(`document.querySelector("#slot iframe").contentWindow.postMessage({ type: "escape" }, "*")`);
      const s = await until((s) => messages(s, "escaped").length > 0);
      const { tried } = messages(s, "escaped")[0]!.data;
      expect(tried.top).toBe("SecurityError");
      expect(tried.parentDoc).toBe("SecurityError");
      await app.eval(`document.querySelector("#slot iframe").contentWindow.postMessage({ type: "ping", n: 99 }, "*")`);
      const after = await until((s) => messages(s, "echo").some((m) => m.data.data?.n === 99));
      expect(messages(after, "hello").length).toBe(1);
    },
    T,
  );

  it(
    "the document shows in the element's box, composed under the page (R5.1, R5.2)",
    async () => {
      const shot = await app.send("screenshot", {
        path: join(tmpdir(), `sk-iframe-composed-${process.pid}.png`),
        backend: "print_window",
      });
      const png = readPng(shot.path);
      const at = (x: number, y: number) => {
        const px = Math.round(x * shot.scale + shot.clientX);
        const py = Math.round(y * shot.scale + shot.clientY);
        return Array.from(png.data.slice((py * png.width + px) * 4, (py * png.width + px) * 4 + 3));
      };
      expect(at(20 + 160, 10 + 200)).toEqual([0, 160, 80]);
      expect(at(20 + 30, 10 + 20)).toEqual([200, 30, 30]);
      expect(at(20 + 2, 10 + 238)).toEqual([0, 160, 80]);
      expect(at(20 + 160, 10 + 240 + 6)).not.toEqual([0, 160, 80]);
      expect(at(20 - 6, 10 + 120)).not.toEqual([0, 160, 80]);
    },
    T,
  );

  it(
    "a press on the box is the document's, at the document's coordinates (R6.1)",
    async () => {
      await app.locator("#slot iframe").click({ real: true });
      const s = await until((s) => messages(s, "click").length > 0);
      const click = messages(s, "click")[0]!.data;
      expect(Math.abs(click.x - 160)).toBeLessThanOrEqual(1);
      expect(Math.abs(click.y - 120)).toBeLessThanOrEqual(1);
      expect(click.onBox).toBe(false);
    },
    T,
  );

  it(
    "the element is a focusable area: a press and focus() make it the active element (R6.3)",
    async () => {
      const active = () => app.eval(`document.activeElement === document.querySelector("#slot iframe")`);
      expect(await active()).toBe(true);
      await app.eval(`document.querySelector("#slot iframe").blur()`);
      expect(await active()).toBe(false);
      await app.eval(`document.querySelector("#slot iframe").focus({ preventScroll: true })`);
      expect(await active()).toBe(true);
      await app.eval(`document.querySelector("#slot iframe").contentWindow.postMessage({ type: "ping", n: 7 }, "*")`);
      await until((s) => messages(s, "echo").some((m) => m.data.data?.n === 7));
    },
    T,
  );

  it(
    "an element that is not painted hides its document; painted again, it is back (R5.5)",
    async () => {
      await app.eval(`document.querySelector("#slot iframe").style.display = "none"`);
      let s = await until((s) => messages(s, "visibility").at(-1)?.data.state === "hidden");
      await app.eval(`document.querySelector("#slot iframe").style.display = "block"`);
      s = await until((s) => messages(s, "visibility").at(-1)?.data.state === "visible");
      expect(messages(s, "visibility").map((m) => m.data.state)).toEqual(["hidden", "visible"]);
      expect(messages(s, "hello").length).toBe(1);
    },
    T,
  );

  it(
    "moving the element keeps the document (R1.8)",
    async () => {
      await app.eval("__SK_TEST_IFRAME__.move()");
      await app.eval(`document.querySelector("#slot iframe").contentWindow.postMessage({ type: "ping", n: 100 }, "*")`);
      const s = await until((s) => messages(s, "echo").some((m) => m.data.data?.n === 100));
      expect(messages(s, "hello").length).toBe(1);
      expect(s.log.filter((l) => l === "load")).toEqual(["load"]);
    },
    T,
  );

  it(
    "a removed element's document is gone: nothing arrives after it (R1.4, R2.5)",
    async () => {
      await app.eval("__SK_TEST_IFRAME__.unmount()");
      const before = (await state()).log.length;
      await new Promise((r) => setTimeout(r, 500));
      const s = await state();
      expect(s.log.length).toBe(before);
      expect(s.log.at(-1)).toBe("removed contentWindow null");
    },
    T,
  );

  (process.platform === "win32" ? it : test.skip)(
    "the app closing ends the browser: no process is left behind (R7.3)",
    async () => {
      await app.eval("__SK_TEST_IFRAME__.mount('/game.html')");
      await until((s) => messages(s, "hello").length === 2);
      expect(browserProcesses()).toBeGreaterThan(0);
      await app.close();
      closed = true;
      const end = Date.now() + 10_000;
      while (browserProcesses() > 0 && Date.now() < end) await new Promise((r) => setTimeout(r, 250));
      expect(browserProcesses()).toBe(0);
    },
    T,
  );
});
