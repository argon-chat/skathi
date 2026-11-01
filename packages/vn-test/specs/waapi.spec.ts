// `el.animate()` paints: Web Animations on the host's animator.
//
// Argon's counter (apps/client/src/components/motionCounter/Digit.vue) rolls
// each digit with `el.animate()` on a back-out curve and cancels a roll that is
// still running when the next tick comes. Before this the call was a
// TypeError. The object model is src/waapi.ts (test/waapi.test.ts); the
// painting half is crates/vn-css/src/animator/script.rs. This spec proves the
// two meet on screen: the style in effect (`styles().computed.live`) and the
// pixels.
//
//   bun test packages/vn-test/specs/waapi.spec.ts

import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/waapi/src/main.ts",
  window: [400, 160],
  label: "waapi",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The style in effect this frame, as one text. */
const live = async (selector: string): Promise<string> =>
  ((await app.locator(selector).styles()).computed.live ?? []).join("\n");
const opacityOf = (text: string): number => {
  const m = /opacity:\s*Some\(\s*([-\d.eE]+)/.exec(text);
  return m ? Number(m[1]) : 1;
};
const translateYOf = (text: string): number => {
  // The dump is pretty-printed Debug: `Pixels(\n 0.0,\n ),` per axis.
  const m = /vn_transform:[\s\S]*?translate:\s*\(\s*Pixels\(\s*([-\d.eE]+),?\s*\),?\s*Pixels\(\s*([-\d.eE]+)/.exec(text);
  return m ? Number(m[2]) : NaN;
};
/** The colour at the digit window's centre: red is an even cell, blue an odd one. */
const digitColour = async (): Promise<string> => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 400;
  const [r, , b] = shot.at(Math.floor(60 * k), Math.floor(60 * k));
  return r > 200 && b < 60 ? "red" : b > 200 && r < 60 ? "blue" : `rgb(${r},?,${b})`;
};

describe("el.animate()", () => {
  test(
    "Digit's roll moves the strip past its target and settles back on its own transform",
    async () => {
      await sleep(500);
      expect(translateYOf(await live("#strip"))).toBe(-400);
      expect(await digitColour()).toBe("red");
      await app.eval(`(() => {
        const strip = document.getElementById("strip");
        window.__roll = strip.animate([{ transform: "translateY(-400px)" }, { transform: "translateY(-440px)" }], {
          duration: 900,
          easing: "cubic-bezier(0.34, 1.56, 0.64, 1)",
        });
        window.__rollDone = false;
        window.__roll.finished.then(() => (window.__rollDone = true));
      })()`);
      const seen: number[] = [];
      let midColour = "";
      const started = Date.now();
      while (Date.now() - started < 1300) {
        seen.push(translateYOf(await live("#strip")));
        if (!midColour && Date.now() - started > 350) midColour = await digitColour();
        await sleep(25);
      }
      // It moved, through the middle…
      expect(seen.some((y) => y < -405 && y > -435)).toBe(true);
      // …overshot the target, as a back-out curve does…
      expect(Math.min(...seen.filter(Number.isFinite))).toBeLessThan(-441);
      // …and painted the next cell while it was there.
      expect(midColour).toBe("blue");
      // Finished, without a fill: back on the strip's own transform.
      expect(await app.eval(`window.__rollDone && window.__roll.playState`)).toBe("finished");
      expect(translateYOf(await live("#strip"))).toBe(-400);
      expect(await digitColour()).toBe("red");
      expect(await app.eval(`document.getElementById("strip").getAnimations().length`)).toBe(0);
    },
    T,
  );

  test(
    "cancel mid-flight drops the effect at once and rejects `finished` with AbortError",
    async () => {
      await app.eval(`(() => {
        const box = document.getElementById("cancel");
        window.__cancel = box.animate({ opacity: [1, 0] }, 2000);
        window.__cancelWhy = "";
        window.__cancel.finished.catch((e) => (window.__cancelWhy = e.name));
      })()`);
      await sleep(400);
      const moving = opacityOf(await live("#cancel"));
      expect(moving).toBeLessThan(0.95);
      expect(moving).toBeGreaterThan(0.5);
      await app.eval(`window.__cancel.cancel()`);
      await sleep(100);
      expect(opacityOf(await live("#cancel"))).toBe(1);
      expect(await app.eval(`[window.__cancelWhy, window.__cancel.playState]`)).toEqual(["AbortError", "idle"]);
    },
    T,
  );

  test(
    "fill: forwards holds the end; commitStyles and cancel keep it",
    async () => {
      await app.eval(`(() => {
        const box = document.getElementById("fade");
        window.__fade = box.animate({ opacity: [1, 0.25] }, { duration: 200, fill: "forwards" });
      })()`);
      await sleep(500);
      expect(await app.eval(`window.__fade.playState`)).toBe("finished");
      expect(opacityOf(await live("#fade"))).toBeCloseTo(0.25, 3);
      expect(await app.eval(`document.getElementById("fade").getAnimations().length`)).toBe(1);
      await app.eval(`(() => { window.__fade.commitStyles(); window.__fade.cancel(); })()`);
      await sleep(100);
      expect(await app.eval(`document.getElementById("fade").style.opacity`)).toBe("0.25");
      expect(opacityOf(await live("#fade"))).toBeCloseTo(0.25, 3);
    },
    T,
  );

  test(
    "Digit's probe: CSS.supports says no to linear(), which animate() refuses, so the fallback rolls",
    async () => {
      expect(
        await app.eval(`(() => {
          let thrown = "";
          try {
            document.createElement("div").animate({ opacity: [0, 1] }, { duration: 10, easing: "linear(0, 1)" });
          } catch (e) {
            thrown = e.name;
          }
          return [CSS.supports("animation-timing-function", "linear(0, 1)"), thrown,
            CSS.supports("animation-timing-function", "cubic-bezier(0.34, 1.56, 0.64, 1)")];
        })()`),
      ).toEqual([false, "TypeError", true]);
    },
    T,
  );

  test(
    "motion's finish: the end value written from a rAF in onfinish, then cancel; no frame paints the base",
    async () => {
      // motion-dom's NativeAnimation, minus motion: fill "both", and onfinish
      // sets the end value through a motion value (a rAF render) and cancels.
      await app.eval(`(() => {
        const box = document.getElementById("settle");
        const a = box.animate({ opacity: [1, 0.2] }, { duration: 300, fill: "both" });
        window.__settled = false;
        a.onfinish = () => {
          requestAnimationFrame(() => (box.style.opacity = "0.2"));
          a.cancel();
          window.__settled = true;
        };
      })()`);
      // Red at the box's centre: 0 at opacity 1, 204 at 0.2 on white. A
      // screenshot is a frame that was drawn, and one takes a few ms.
      const reds: number[] = [];
      const started = Date.now();
      while (Date.now() - started < 700) {
        const shot = readPng(await app.screenshot());
        const k = shot.width / 400;
        reds.push(shot.at(Math.floor(360 * k), Math.floor(60 * k))[0]);
      }
      const near = reds.findIndex((r) => r > 180);
      expect(near).toBeGreaterThan(0);
      // Once near the end, it never goes back to the base value.
      expect(Math.min(...reds.slice(near))).toBeGreaterThan(180);
      expect(reds[reds.length - 1]).toBeGreaterThan(200);
      expect(await app.eval(`[window.__settled, document.getElementById("settle").getAnimations().length, document.getElementById("settle").style.opacity]`)).toEqual([true, 0, "0.2"]);
    },
    T,
  );

  test(
    "pause and a seek paint the held time, and it stays there",
    async () => {
      await app.eval(`(() => {
        const box = document.getElementById("seek");
        window.__seek = box.animate({ opacity: [0, 1] }, 1000);
        window.__seek.pause();
        window.__seek.currentTime = 500;
      })()`);
      await sleep(150);
      expect(opacityOf(await live("#seek"))).toBeCloseTo(0.5, 3);
      await sleep(400);
      expect(opacityOf(await live("#seek"))).toBeCloseTo(0.5, 3);
      expect(await app.eval(`[window.__seek.playState, window.__seek.currentTime]`)).toEqual(["paused", 500]);
    },
    T,
  );
});
