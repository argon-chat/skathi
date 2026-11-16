import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/focus-style/src/main.ts",
  window: [460, 340],
  label: "focus-style",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Out = { log: string[]; active: string[] };
async function out(): Promise<Out> {
  return JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_FOCUS_STYLE__)"))) as Out;
}
async function js<T>(source: string): Promise<T> {
  return JSON.parse(String(await app.eval(`JSON.stringify((() => { ${source} })())`))) as T;
}
const active = () => js<string>("return document.activeElement ? document.activeElement.id : null");

describe("focus() and the element's style", () => {
  test(
    "a popover that focuses its item while still visibility: hidden keeps the focus where it was",
    async () => {
      await app.eval("__SK_TEST_FOCUS_STYLE__.log.length = 0");
      await app.locator("#opener").click({ real: true });
      const deadline = Date.now() + 5_000;
      while ((await out()).active.length === 0 && Date.now() < deadline) await sleep(50);
      await sleep(400);
      const o = await out();
      bunExpect(o.active.length).toBe(1);
      bunExpect(o.active[0]).not.toBe("tone-1");
      bunExpect(o.log).not.toContain("focusin t=tone-1");
      bunExpect(await active()).toBe("opener");
      bunExpect(await js<string>('return getComputedStyle(document.getElementById("tone-1")).visibility')).toBe(
        "visible",
      );
      await app.eval('document.getElementById("tone-1").focus()');
      bunExpect(await active()).toBe("tone-1");
      await app.locator("#opener").click({ real: true });
      await sleep(200);
      bunExpect((await app.query("#tones")).count).toBe(0);
    },
    T,
  );

  test(
    "an element in a visibility: hidden box, or a display: none one, refuses focus",
    async () => {
      await app.eval('document.getElementById("other").focus()');
      bunExpect(await active()).toBe("other");
      await app.eval('document.getElementById("hchild").focus()');
      bunExpect(await active()).toBe("other");
      await app.eval('document.getElementById("nchild").focus()');
      bunExpect(await active()).toBe("other");
      await sleep(300);
      bunExpect(await active()).toBe("other");
      bunExpect(await js<string>('return getComputedStyle(document.getElementById("hchild")).visibility')).toBe(
        "hidden",
      );
    },
    T,
  );

  test(
    "a screen-reader-only field paints nothing but is visible to CSS, and takes focus",
    async () => {
      bunExpect(await js<string>('return getComputedStyle(document.getElementById("sr")).visibility')).toBe("visible");
      await app.eval('document.getElementById("sr").focus()');
      bunExpect(await active()).toBe("sr");
    },
    T,
  );

  test(
    "a style written on the line before focus() counts",
    async () => {
      const after = await js<string>(
        'document.getElementById("other").focus(); document.getElementById("hbox").style.visibility = "visible"; ' +
          'document.getElementById("hchild").focus(); return document.activeElement.id',
      );
      bunExpect(after).toBe("hchild");
    },
    T,
  );

  test(
    "a focus state on an ancestor compound paints its descendants as focus moves",
    async () => {
      await app.eval(`(() => {
        const style = document.createElement("style");
        style.textContent =
          "#anc { position: fixed; inset: 0; z-index: 10; background: white; display: flex; gap: 8px; padding: 8px } " +
          ".knob { width: 24px; height: 24px; background: rgb(255 0 0); display: flex } .knob i { width: 2px; height: 2px } " +
          ".slider:focus .knob, .fw:focus-within .knob, .fv:focus-visible .knob { background: rgb(0 0 255) }";
        document.head.appendChild(style);
        const host = document.createElement("div");
        host.id = "anc";
        for (const id of ["slider", "fw", "fv"]) {
          const box = document.createElement("div");
          box.id = id;
          box.className = id;
          box.tabIndex = 0;
          const wrap = document.createElement("div");
          const knob = document.createElement("div");
          knob.id = id + "-knob";
          knob.className = "knob";
          for (let i = 0; i < 6; i++) knob.appendChild(document.createElement("i"));
          wrap.appendChild(knob);
          box.appendChild(wrap);
          host.appendChild(box);
        }
        const inner = document.createElement("div");
        inner.id = "fw-inner";
        inner.tabIndex = 0;
        inner.style.cssText = "width: 8px; height: 8px";
        host.querySelector("#fw").appendChild(inner);
        document.body.appendChild(host);
      })()`);
      await sleep(300);
      const painted = async (): Promise<string> => {
        const shot = readPng(await app.screenshot());
        const k = shot.width / 460;
        let out = "";
        for (const id of ["slider", "fw", "fv"]) {
          const box = await app.locator(`#${id}-knob`).boundingBox();
          const [r, , b] = shot.at(Math.floor((box.x + 4) * k), Math.floor((box.y + 4) * k));
          out += r > 200 && b < 60 ? "r" : b > 200 && r < 60 ? "b" : "?";
        }
        return out;
      };
      bunExpect(await painted()).toBe("rrr");
      await app.locator("#slider").focus();
      await sleep(200);
      bunExpect(await painted()).toBe("brr");
      await app.locator("#fw-inner").focus();
      await sleep(200);
      bunExpect(await painted()).toBe("rbr");
      await app.locator("#fw-inner").press("Tab");
      await sleep(200);
      bunExpect(await active()).toBe("fv");
      bunExpect(await painted()).toBe("rrb");
      await app.eval('document.getElementById("anc").remove()');
    },
    T,
  );
});
