import { afterAll, expect, test } from "bun:test";
import { launch, readPng, SCREENSHOTS } from "../src/index";

const viewport = 620;
const app = await launch({
  app: "packages/test/fixtures/state-custom-properties/src/main.ts",
  window: [viewport, 160],
  label: "state-custom-properties",
});
afterAll(() => app.close());

const resetPointer = () => app.locator("#self").unhover();

async function eventually(read: () => Promise<string>, expected: string) {
  const deadline = Date.now() + 4_000;
  let actual = await read();
  while (actual !== expected && Date.now() < deadline) {
    await Bun.sleep(30);
    actual = await read();
  }
  expect(actual).toBe(expected);
}

test.skipIf(!SCREENSHOTS)(
  "hovered tokens repaint the box, child, SVG stops and geometry, and revert on pointer leave",
  async () => {
    await resetPointer();
    const boxes = ["self", "parent", "inline", "sheet", "color", "same", "geometry"];
    const centers = await Promise.all(
      boxes.map(async (id) => {
        const { x, y, w, h } = await app.locator(`#${id}`).boundingBox();
        return { x: x + w / 2, y: y + h / 2 };
      }),
    );
    const colors = async () => {
      const shot = readPng(await app.screenshot());
      const scale = shot.width / viewport;
      return centers.map(({ x, y }) => shot.at(Math.floor(x * scale), Math.floor(y * scale)).join(",")).join(" ");
    };
    const expected = (hovered?: string) => boxes.map((id) => (id === hovered ? "0,0,255" : "255,0,0")).join(" ");
    await eventually(colors, expected());
    for (const id of boxes) {
      await app.locator(`#${id}`).hover();
      await eventually(colors, expected(id));
    }
    await resetPointer();
    await eventually(colors, expected());
  },
  45_000,
);

test("computed custom properties follow hover on the owner and on descendants, including non-consumers", async () => {
  const tokens = () =>
    app.eval(
      `["parent", "child", "unpainted", "self"].map(id =>
      getComputedStyle(document.getElementById(id)).getPropertyValue("--c").trim()).join(" ")`,
    ) as Promise<string>;
  await resetPointer();
  await eventually(tokens, "red red red red");
  await app.locator("#child").hover();
  await eventually(tokens, "blue blue blue red");
  await resetPointer();
  await eventually(tokens, "red red red red");
}, 20_000);

test("a transition interpolates an inherited hover token in both directions", async () => {
  const width = () =>
    app
      .locator("#bar")
      .boundingBox()
      .then((box) => String(box.w));
  await resetPointer();
  await eventually(width, "20");
  await app.eval(`
    const bar = document.getElementById("bar");
    let ends = 0;
    bar.addEventListener("transitionend", e => {
      if (e.propertyName === "width") bar.setAttribute("data-result", [++ends, Math.round(e.elapsedTime * 1000)].join(","));
    });
  `);
  const result = () => app.eval('document.getElementById("bar").getAttribute("data-result") || ""') as Promise<string>;
  const frame = async () => {
    const w = (await app.locator("#bar").boundingBox()).w;
    return w > 20 && w < 90 ? "intermediate" : String(w);
  };
  await app.locator("#motion").hover();
  await eventually(frame, "intermediate");
  await eventually(width, "90");
  await eventually(result, "1,450");
  await resetPointer();
  await eventually(frame, "intermediate");
  await eventually(width, "20");
  await eventually(result, "2,450");
}, 25_000);

test("focus, focus-within and an active press update inherited tokens independently", async () => {
  const tokens = () =>
    app.eval(`(() => {
    const style = getComputedStyle(document.getElementById("label"));
    return ["--within","--focus","--press","--both"].map(key => style.getPropertyValue(key).trim()).join(" ");
  })()`) as Promise<string>;
  await resetPointer();
  await app.locator("#other").focus();
  await eventually(tokens, "outside off up no");
  await app.locator("#action").focus();
  await eventually(tokens, "inside on up no");
  await app.locator("#action").hover();
  await eventually(tokens, "inside on up yes");
  await app.send("mouse", { selector: "#action", index: 0, kind: "down" });
  try {
    await eventually(tokens, "inside on down yes");
  } finally {
    await app.send("mouse", { selector: "#action", index: 0, kind: "up" });
  }
  await eventually(tokens, "inside on up yes");
  await app.locator("#other").focus();
  await eventually(tokens, "outside off up no");
}, 25_000);
