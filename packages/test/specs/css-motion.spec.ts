import { afterAll, expect, setDefaultTimeout, test } from "bun:test";
import { launch } from "../src/index";

setDefaultTimeout(30_000);

const app = await launch({
  app: "packages/test/fixtures/css-motion/src/main.ts",
  window: [640, 440],
  label: "css-motion",
});
afterAll(() => app.close());

async function until<T>(read: () => Promise<T>, matches: (value: T) => boolean): Promise<T> {
  const deadline = Date.now() + 5000;
  let value = await read();
  while (!matches(value) && Date.now() < deadline) {
    await Bun.sleep(16);
    value = await read();
  }
  return value;
}

interface PanelFrame {
  opacity: number;
  translate: number;
  display: string;
  height: number;
}

const frame = `(() => {
  const panel = document.getElementById("panel");
  const css = getComputedStyle(panel);
  return {
    opacity: Number(css.opacity),
    translate: Number.parseFloat(css.translate.split(" ")[1]) || 0,
    display: css.display,
    height: panel.getBoundingClientRect().height,
  };
})()`;
const sample = async (): Promise<PanelFrame> => app.eval(frame);

test("starting-style fades and translates a newly inserted plain DOM panel, and display holds the exit open", async () => {
  const entry: PanelFrame = await app.eval(`(() => {
    const panel = document.createElement("div");
    panel.id = "panel";
    panel.className = "panel";
    panel.textContent = "Draft saved";
    document.getElementById("panel-slot").appendChild(panel);
    return ${frame};
  })()`);
  expect(entry.display).toBe("block");
  expect(entry.opacity).toBeLessThan(0.1);
  expect(entry.translate).toBeGreaterThan(15);

  const mid = await until(sample, (s) => s.opacity > 0.2 && s.height > 0);
  expect(mid.opacity).toBeLessThan(1);
  expect(mid.translate).toBeGreaterThan(0);
  expect((await until(sample, (s) => s.opacity === 1)).translate).toBe(0);

  const exit: PanelFrame = await app.eval(`(() => {
    document.getElementById("panel").setAttribute("hidden", "");
    return ${frame};
  })()`);
  expect(exit.display).toBe("block");
  const fading = await until(sample, (s) => s.opacity < 0.8);
  expect(fading.display).toBe("block");
  expect(fading.height).toBeGreaterThan(0);
  expect((await until(sample, (s) => s.display === "none")).opacity).toBe(0);

  const revealed: PanelFrame = await app.eval(`(() => {
    document.getElementById("panel").removeAttribute("hidden");
    return ${frame};
  })()`);
  expect(revealed.display).toBe("block");
  expect(revealed.opacity).toBeLessThan(0.1);
});

test("transition delays follow the element order and skip text siblings", async () => {
  const values = async (): Promise<[number, number, number][]> =>
    app.eval(`["a", "b", "c"].map((id) => {
      const style = getComputedStyle(document.getElementById(id));
      return [Number.parseFloat(style.transitionDelay), Number(style.zIndex), Math.round(Number.parseFloat(style.rotate))];
    })`);
  expect(await values()).toEqual([
    [0.15, 1, 120],
    [0.3, 2, 240],
    [0.45, 3, 360],
  ]);
  await app.eval(`(() => {
    const tiles = document.getElementById("tiles");
    tiles.insertBefore(document.getElementById("c"), document.getElementById("a"));
    tiles.insertBefore(document.createTextNode(" "), document.getElementById("a"));
    tiles.className = "active";
  })()`);
  expect(await values()).toEqual([
    [0.3, 2, 240],
    [0.45, 3, 360],
    [0.15, 1, 120],
  ]);

  const opacities = async (): Promise<number[]> =>
    app.eval(`["c", "a", "b"].map((id) => Number(getComputedStyle(document.getElementById(id)).opacity))`);
  const first = await until(opacities, (list) => (list[0] ?? 0) > 0.25);
  expect(first[0]).toBeGreaterThan(first[1] ?? 0);
  expect(first[2]).toBeCloseTo(0.2);
});
