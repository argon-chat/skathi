import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { create } from "../../cli/cli/create";
import { expect as ui, launch, readPng, REPO, SCREENSHOTS, WORK_DIR } from "../src/index";

mkdirSync(WORK_DIR, { recursive: true });
const dir = mkdtempSync(join(WORK_DIR, "showcase-features-"));
create(dir, { name: "Showcase", template: "showcase" });
const app = await launch({
  app: relative(REPO, join(dir, "src/main.ts")).replaceAll("\\", "/"),
  window: [1040, 760],
  label: "showcase-features",
  env: { SK_APP_ROOT: dir },
});
afterAll(async () => {
  await app.close();
  rmSync(dir, { recursive: true, force: true });
});

const page = (name: string) => app.locator(`nav a[href="/${name}"]`).click();
async function waitFor(js: string) {
  const deadline = Date.now() + 5000;
  while (!(await app.eval(js))) {
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${js}`);
    await Bun.sleep(50);
  }
}

async function recordHaptics() {
  await app.eval(`globalThis.cues = [];
    __sk_appHaptics.perform = (pattern, {pointerId, when} = {}) => {
      cues.push([pattern, pointerId, when]); return true;
    };
    __sk_appHaptics.stop = id => { cues.push(["stop", id]); return true; };`);
}
const cues = async () => JSON.parse(String(await app.eval("JSON.stringify(cues)")));

test("haptics binds individual gestures and lets the app choose the fallback", async () => {
  await app.eval(`globalThis.cues = [];
    const supported = ["generic", "hover", "success", "level-change"];
    __sk_appHaptics.getSupportedPatterns = () => supported.slice();
    __sk_appHaptics.perform = (pattern, options) => {
      cues.push([pattern, options.pointerId, options.when, options.fallback]);
      return supported.includes(pattern) || supported.includes(options.fallback);
    };`);
  const change = (name: string, value: string, event = "change") =>
    app.eval(`{ const el = document.querySelector('[aria-label="${name}"]'); el.value = ${JSON.stringify(value)};
      el.dispatchEvent(new Event(${JSON.stringify(event)}, {bubbles: true})); }`);
  try {
    await page("haptics");
    const status = ui(app.locator(".receipt"));
    const button = app.locator('[aria-label="Try the button gesture"]');
    await ui(app.locator(".device")).toContainText("4 available");
    await button.click();
    await status.toContainText("Accepted · success");
    await change("Button activation effect", "click");
    await change("Fallback effect", "off");
    await button.click();
    await status.toContainText("click unavailable");
    await change("Fallback effect", "generic");
    await change("Feedback timing", "next-frame");
    await button.click();
    await status.toContainText("Queued for next frame · generic");

    const hover = await app.locator('[aria-label="Try the hover gesture"]').boundingBox();
    const x = Math.round(hover.x + hover.w / 2),
      y = Math.round(hover.y + hover.h / 2);
    await app.send("mouse", { kind: "move", x, y });
    await status.toContainText("Pointer entry");
    await app.send("mouse", { kind: "move", x: x + 1, y });
    await change("Haptic sample level", "4", "input");
    await change("Haptic sample level", "4", "input");
    await status.toContainText("Slider step");
    await change("Button activation effect", "off");
    await button.click();
    await status.toContainText("Feedback off");
    await app.locator('[aria-label="Play hover"]').click();
    await status.toContainText("Device effect · Accepted · hover");
    expect(await cues()).toEqual([
      ["success", null, "now", "generic"],
      ["click", null, "now", false],
      ["click", null, "next-frame", "generic"],
      ["hover", 1, "next-frame", "generic"],
      ["level-change", null, "next-frame", "generic"],
      ["hover", 1, null, false],
    ]);
  } finally {
    await app.eval("delete __sk_appHaptics.getSupportedPatterns; delete __sk_appHaptics.perform;");
  }
}, 30_000);

test("the device palette follows a pen and stops its texture on lift", async () => {
  await app.eval(`globalThis.cues = [];
    __sk_appHaptics.getSupportedPatterns = id => id === 2 ? ["ink-continuous"] : ["hover"];
    __sk_appHaptics.perform = (pattern, opts) => { cues.push([pattern, opts.pointerId, opts.fallback]); return true; };
    __sk_appHaptics.stop = id => { cues.push(["stop", id]); return true; };`);
  try {
    await page("haptics");
    const device = app.locator(".device");
    const status = ui(app.locator(".receipt"));
    const label = await app.locator(".device span").boundingBox();
    const location = { x: Math.round(label.x + 8), y: Math.round(label.y + label.h / 2) };
    await app.send("pen", { kind: "down", ...location, pressure: 0.5 });
    await ui(device).toContainText("Pen · 1 available");
    await app.send("pen", { kind: "up", ...location });
    const texture = await app.locator('[aria-label="Hold ink-continuous"]').boundingBox();
    const tip = { x: Math.round(texture.x + texture.w / 2), y: Math.round(texture.y + texture.h / 2) };
    await app.send("pen", { kind: "down", ...tip, pressure: 0.5 });
    await status.toContainText("Accepted · ink-continuous");
    await app.send("pen", { kind: "up", ...location });
    await status.toContainText("Texture stopped");
    expect(await cues()).toEqual([
      ["ink-continuous", 2, false],
      ["stop", 2],
    ]);

    await app.send("mouse", { kind: "click", ...location });
    await ui(device).toContainText("Mouse or trackpad · 1 available");
    await app.locator('[aria-label="Play hover"]').waitFor();
    expect(await app.locator('[aria-label="Hold ink-continuous"]').count()).toBe(0);
  } finally {
    await app.eval(
      "delete __sk_appHaptics.getSupportedPatterns; delete __sk_appHaptics.perform; delete __sk_appHaptics.stop;",
    );
  }
}, 30_000);

test("the haptics button requests a pulse from the current device without a pointer ID", async () => {
  await page("graphics");
  await recordHaptics();
  const button = app.locator('[aria-label="Test haptics on the current device"]');
  await button.click();
  await ui(app.locator('p[role="status"]')).toHaveText("The system accepted the pulse.");
  expect(await cues()).toEqual([["generic", null, null]]);
  await app.eval("__sk_appHaptics.perform = () => false");
  await button.click();
  await ui(app.locator('p[role="status"]')).toContainText("device");
}, 30_000);

test("pressure, guide alignment and pen feedback use the active pointer", async () => {
  await page("graphics");
  const pad = await app.locator("canvas.pressure-pad").boundingBox();
  await recordHaptics();
  const x = Math.round(pad.x + pad.w / 3);
  const y = Math.round(pad.y + (80 * pad.h) / 200);
  await app.send("mouse", { kind: "down", x, y });
  await app.send("mouse_pressure", { stage: 2, pressure: 0.6, x, y });
  await ui(app.locator(".readout")).toContainText("pressure 0.80");
  await app.send("mouse", { kind: "move", x: x + 6, y, held: true });
  await app.send("mouse", { kind: "up", x: x + 6, y });
  await ui(app.locator(".readout")).toContainText("pressure 0.00");
  expect(await cues()).toEqual([["alignment", 1, "next-frame"]]);

  // Between guides, a pen stroke should only start and stop the selected feedback.
  const penY = Math.round(pad.y + pad.h / 2);
  await app.send("pen", { kind: "down", x, y: penY, pressure: 0.7 });
  await ui(app.locator(".readout")).toContainText("pen · pressure 0.70");
  await app.send("pen", { kind: "up", x, y: penY });
  await waitFor("cues.some(c => c[0] === 'stop')");
  const [, start, stop] = await cues();
  expect(start[0]).toBe("ink-continuous");
  expect(stop).toEqual(["stop", start[1]]);
}, 30_000);

test("the carousel requests one haptic pulse per settled destination", async () => {
  await page("layout");
  await app.locator(".strip").waitFor();
  await recordHaptics();
  const dune = app.locator('[aria-label="Go to Dune"]');
  await dune.click();
  await waitFor("cues.length > 0");
  expect(await app.eval(`document.querySelector('.dot[aria-pressed="true"]').getAttribute("aria-label")`)).toBe(
    "Go to Dune",
  );
  await dune.click();
  await Bun.sleep(350);
  expect(await cues()).toEqual([["alignment", null, "next-frame"]]);
  await app.locator('[aria-label="Go to Night"]').click();
  await waitFor("cues.length === 2");
  expect(await app.eval(`document.querySelector('.dot[aria-pressed="true"]').getAttribute("aria-label")`)).toBe(
    "Go to Night",
  );
}, 30_000);

test("the focused video handles Space once and exposes held and shifted keys", async () => {
  await page("media");
  const player = app.locator("video.player");
  await player.focus();
  // Count actions independently of the machine's decoder and the bundled clip's playback state.
  await app.eval(`globalThis.videoActions = 0;
    const v = document.querySelector("video.player");
    v.play = () => { videoActions++; return Promise.resolve(); };
    v.pause = () => { videoActions++; };`);
  const key = (options = "") =>
    app.eval(`document.activeElement.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", bubbles: true, cancelable: true, ${options}
    }))`);
  expect(await key()).toBe(false);
  await waitFor("videoActions === 1");
  expect(await key("repeat: true")).toBe(false);
  await ui(app.locator(".tile p.mono")).toHaveText("Space · held");
  expect(await key("shiftKey: true")).toBe(true);
  await ui(app.locator(".tile p.mono")).toHaveText("Space · Shift");
  expect(await app.eval("videoActions")).toBe(1);
}, 30_000);

test("inline SVG follows the width and gradient controls", async () => {
  await page("styles");
  await app.locator("svg.band-preview").waitFor();
  const width = 'getComputedStyle(document.querySelector("rect.band")).width';
  await waitFor(`${width} === "180px"`);
  await app.eval(`const slider = document.querySelector('[aria-label="SVG width"]');
    slider.value = "96"; slider.dispatchEvent(new Event("input", { bubbles: true }));`);
  await waitFor(`${width} === "96px"`);
  if (SCREENSHOTS) {
    const box = await app.locator("svg.band-preview").boundingBox();
    const viewportWidth = Number(await app.eval("innerWidth"));
    // A Vue update can reach computed style before the changed SVG reaches the screen.
    const waitForShade = async (warm: boolean) => {
      const deadline = Date.now() + 5000;
      for (;;) {
        const png = readPng(await app.screenshot(join(dir, "gradient.png")));
        const scale = png.width / viewportWidth;
        const [r, g, b] = png.at(Math.round((box.x + 88) * scale), Math.round((box.y + box.h / 2) * scale));
        if (warm ? r > g + 25 && g > b : g > r + 40 && b > r) return;
        if (Date.now() > deadline)
          throw new Error(`Expected a ${warm ? "warm" : "cool"} gradient end, got ${[r, g, b]}`);
        await Bun.sleep(50);
      }
    };
    await waitForShade(false);
    await app.locator('[aria-label="Warm end color"]').click();
    await waitForShade(true);
  }
}, 30_000);

test("the counter is a second native window backed by the same store", async () => {
  await page("window");
  const open = app.locator(".row > button.primary");
  await open.click();
  await waitFor("JSON.parse(__sk_window_popouts()).length === 1");
  expect((await app.send("windows")).windows).toContainEqual(
    expect.objectContaining({
      kind: "popout",
      viewport: expect.objectContaining({ w: 352, h: 198 }),
    }),
  );
  await app.locator(".mini button").click();
  await ui(app.locator(".mini-count")).toHaveText("1");
  await ui(app.locator(".card span.muted")).toContainText("count = 1");
  await open.click();
  await waitFor("JSON.parse(__sk_window_popouts()).length === 0");
}, 30_000);
