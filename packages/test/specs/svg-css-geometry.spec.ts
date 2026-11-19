import { afterAll, expect, test } from "bun:test";
import { launch, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/svg-css-geometry/src/main.ts",
  window: [580, 130],
  label: "svg-css-geometry",
});
afterAll(() => app.close());

test("computed rect dimensions follow cascaded geometry in SVG user units", async () => {
  await app.locator("#outline").boundingBox();
  const dimensions = () =>
    app.eval(`["#outline rect", "#outline-control rect", ".missing-width"].map(sel => {
    const style = getComputedStyle(document.querySelector(sel));
    return [style.width, style.getPropertyValue("height")].join(" ");
  }).join(",")`);
  expect(await dimensions()).toBe("119px 43px,119px 43px,0px 4px");
  try {
    await app.eval(`document.querySelector("#outline").setAttribute("viewBox", "0 0 240 88")`);
    expect(await dimensions()).toBe("239px 87px,119px 43px,0px 4px");
  } finally {
    await app.eval(`document.querySelector("#outline").removeAttribute("viewBox")`);
  }
});

test.skipIf(!SCREENSHOTS)(
  "CSS geometry and gradient stops match literal SVG attributes",
  async () => {
    const bounds = async (selector: string) => {
      const box = (await app.query(selector)).nodes[0]?.bounds;
      if (!box) throw new Error(`Missing ${selector}`);
      return box;
    };
    const [outline, edge, grad, control] = await Promise.all(
      ["#outline", "#outline-control", "#gradient", "#gradient-control"].map(bounds),
    );
    if (!outline || !edge || !grad || !control) throw new Error("Missing SVG controls");
    const shot = readPng(await app.screenshot());
    const scale = shot.width / 580;
    const at = (box: typeof outline, x: number, y: number) =>
      shot.at(Math.floor((box.x + x + 0.5) * scale), Math.floor((box.y + y + 0.5) * scale));
    for (const [x, y] of [
      [60, 0],
      [0, 22],
      [119, 22],
      [60, 43],
      [0, 0],
      [60, 22],
    ] as const) {
      expect(at(outline, x, y)).toEqual(at(edge, x, y));
    }
    expect(at(edge, 60, 0)[2]).toBeGreaterThan(at(edge, 60, 22)[2] + 60);
    for (const x of [5, 30, 60, 90, 115]) {
      const actual = at(grad, x, 22);
      const want = at(control, x, 22);
      for (const i of [0, 1, 2] as const) expect(Math.abs(actual[i] - want[i])).toBeLessThanOrEqual(2);
    }
    expect(at(grad, 5, 22)[2]).toBeGreaterThan(190);
    expect(at(grad, 115, 22)[0]).toBeGreaterThan(at(grad, 5, 22)[0] + 25);
  },
  30_000,
);

test("a skipped SVG shape is identified in the host log", async () => {
  const entries = JSON.stringify((await app.logs()).entries);
  expect(entries).toContain("missing-width");
  expect(entries).toContain("resvg skips this shape");
});
