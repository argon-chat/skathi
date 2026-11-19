import { afterAll, expect, test } from "bun:test";
import { launch, MACOS, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/css-components/src/main.ts",
  window: [480, 340],
  label: "css-components",
});
afterAll(() => app.close());

async function eventually(read: () => Promise<unknown>, expected: unknown) {
  const deadline = Date.now() + 6_000;
  let actual = await read();
  while (actual !== expected && Date.now() < deadline) {
    await Bun.sleep(30);
    actual = await read();
  }
  expect(actual).toBe(expected);
}

test("percentage comparisons follow the containing block's content box when it resizes", async () => {
  const sizes = async () => {
    const sheet = await app.locator("#sheet").boundingBox();
    const limits = await app.locator("#limits").boundingBox();
    return [sheet.w, sheet.h, limits.w].join(",");
  };
  await eventually(sizes, "200,40,140");
  await app.eval('document.querySelector("#bounds").style.cssText = "width: 150px; height: 40px"');
  await eventually(sizes, "130,12,80");
}, 20_000);

test("percentage comparisons use the grid area and the absolute containing block's padding box", async () => {
  const width = async (id: string) => (await app.locator(id).boundingBox()).w;
  await eventually(() => width("#gridcap"), 60);
  await eventually(() => width("#abscap"), 240);
  await app.eval('document.getElementById("grid").style.gridTemplateColumns = "230px 70px"');
  await eventually(() => width("#gridcap"), 200);
}, 20_000);

test.skipIf(!SCREENSHOTS)(
  "a percentage comparison on an SVG shape uses the SVG viewBox in user units",
  async () => {
    const box = await app.locator("#size-svg").boundingBox();
    await eventually(async () => {
      const shot = readPng(await app.screenshot());
      const scale = shot.width / 480;
      const colour = (x: number) => shot.at(Math.floor((box.x + x) * scale), Math.floor((box.y + 10) * scale));
      return [colour(30), colour(120)].join(";");
    }, "0,255,0;255,255,255");
  },
  20_000,
);

test("one transition rule picks up different local durations and runs both transitions", async () => {
  await app.locator("#fast").boundingBox();
  expect(
    await app.eval(
      `["fast", "slow"].map(id => getComputedStyle(document.getElementById(id)).transitionDuration).join(",")`,
    ),
  ).toBe("0.075s,0.35s");
  await app.eval(`
    for (const id of ["fast", "slow"]) {
      const el = document.getElementById(id);
      el.addEventListener("transitionend", e => el.setAttribute("data-ended", e.propertyName));
      el.className = "fade changed";
    }
  `);
  await eventually(
    () => app.eval('["fast", "slow"].map(id => document.getElementById(id).getAttribute("data-ended")).join(",")'),
    "opacity,opacity",
  );
}, 20_000);

const posterHeight = async () => (await app.query("#poster")).nodes[0]?.bounds?.h ?? 0;

test("hover creates and removes generated descendant boxes", async () => {
  await app.locator("#card").hover();
  await eventually(posterHeight, 13);
  await app.locator("#prior").hover();
  await eventually(posterHeight, 0);
}, 30_000);

test.skipIf(MACOS)(
  "keyboard focus creates a generated descendant box",
  async () => {
    await app.locator("#prior").focus();
    await app.locator("#prior").press("Tab");
    await eventually(posterHeight, 17);
  },
  30_000,
);

test("a relative box paints and hit-tests over a later overlapping in-flow sibling", async () => {
  const rel = await app.locator("#relative").boundingBox();
  const x = Math.floor(rel.x + rel.w / 2);
  const y = Math.floor(rel.y + rel.h - 5);
  for (const z of ["auto", "0"]) {
    await app.eval(`document.querySelector("#relative").style.zIndex = "${z}"`);
    await eventually(() => app.eval(`document.elementFromPoint(${x}, ${y})?.id`), "relative");
    if (SCREENSHOTS) {
      const shot = readPng(await app.screenshot());
      const scale = shot.width / 480;
      expect(shot.at(Math.floor(x * scale), Math.floor(y * scale))).toEqual([0, 255, 0]);
    }
  }
}, 20_000);
