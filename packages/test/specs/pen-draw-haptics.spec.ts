import { expect, test } from "bun:test";
import { launch } from "../src/index";

test("the drawing app requests one detent per snap and feedback at brush-width stops", async () => {
  const app = await launch({ app: "pen-draw", label: "pen-draw-haptics", window: [960, 680] });
  try {
    await app.eval(`globalThis.cues = [];
      __sk_appHaptics.perform = (pattern, {pointerId, when} = {}) => { cues.push([pattern, pointerId, when]); return true; }`);
    const paper = await app.locator("canvas.paper").boundingBox();
    const mouse = async (kind: string, x: number, y: number, held = false) => {
      await app.send("mouse", { kind, x, y, held });
      await Bun.sleep(100);
    };
    const cues = async () => JSON.parse(String(await app.eval("JSON.stringify(cues.splice(0))"))) as unknown[];
    const x = Math.round(paper.x) + 64;
    const y = Math.round(paper.y) + 64;
    await mouse("down", x + 3, y);
    await mouse("move", x + 2, y, true);
    await app.send("mouse_pressure", { stage: 2, pressure: 0.6, x: x + 2, y });
    await Bun.sleep(100);
    expect(await app.eval("document.querySelector('.hud').textContent.includes('pressure 0.80')")).toBe(true);
    await mouse("move", x + 16, y, true);
    await mouse("move", x + 31, y, true);
    await mouse("up", x + 31, y);
    expect(await cues()).toEqual([
      ["alignment", 1, "next-frame"],
      ["alignment", 1, "next-frame"],
    ]);
    const slider = await app.locator("input[type=range]").boundingBox();
    const sy = Math.round(slider.y + slider.h / 2);
    await mouse("down", Math.round(slider.x + slider.w / 2), sy);
    await mouse("move", Math.round(slider.x + slider.w * 0.7), sy, true);
    await mouse("move", Math.round(slider.x + slider.w - 1), sy, true);
    await mouse("up", Math.round(slider.x + slider.w - 1), sy);
    const ticks = await cues();
    expect(ticks).toContainEqual(["level-change", 1, "next-frame"]);
    expect(ticks).toContainEqual(["generic", 1, "next-frame"]);
    await app.expectNoErrors();
  } finally {
    await app.close();
  }
}, 90_000);
