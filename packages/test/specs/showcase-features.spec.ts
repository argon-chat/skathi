import { afterAll, expect, test } from "bun:test";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { create } from "../../cli/cli/create";
import { expect as ui, launch, readPng, REPO, SCREENSHOTS, WINDOWS, WORK_DIR } from "../src/index";

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
const choose = (label: string, value: string, instance = app) =>
  instance.eval(`{
  const el = document.querySelector(${JSON.stringify('select[aria-label="' + label + '"]')});
  el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event("change", { bubbles: true }));
}`);
async function waitFor(js: string, instance = app) {
  const deadline = Date.now() + 5000;
  while (!(await instance.eval(js))) {
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

test("Gamepad opens from the sidebar and shows both APIs before a controller gesture", async () => {
  await page("gamepad");
  await app.locator("#gamepad-page").waitFor();
  await app.locator('nav a[href="/gamepad"].router-link-exact-active').waitFor();

  for (const [api, devices, label, hint] of [
    ["native", "__sk_std.gamepads.list()", "skathi:gamepad", "No controller connected"],
    ["web", "navigator.getGamepads()", "navigator.getGamepads()", "press a controller button"],
  ] as const) {
    await ui(app.locator(`#gamepad-${api} .api`)).toHaveText(label);
    const count = await app.eval(`${devices}.filter(Boolean).length`);
    if (count === 0) {
      await ui(app.locator(`#gamepad-${api}-empty`)).toContainText(hint);
      expect(await app.eval(`document.querySelector("#gamepad-${api}-rumble").disabled`)).toBe(true);
    }
  }

  const actions = app.locator("#gamepad-confirm-count");
  await ui(actions).toHaveText("0");
  await app.press("Enter");
  await ui(actions).toHaveText("1");
}, 30_000);

async function launchGamepadFixture(label: string) {
  const fixture = join(dir, label);
  create(fixture, { name: "Gamepad", template: "showcase" });
  copyFileSync(
    join(import.meta.dir, "../fixtures/showcase-gamepads/src/controllers.ts"),
    join(fixture, "src/controllers.ts"),
  );
  // Built-in modules are registered before the entry runs. Substitute the controller provider in the disposable app.
  for (const name of ["pages/GamepadPage.vue", "components/GamepadDeadZone.vue"]) {
    const component = join(fixture, "src", name);
    writeFileSync(component, readFileSync(component, "utf8").replace('from "skathi:gamepad"', 'from "../controllers"'));
  }
  return launch({
    app: relative(REPO, join(fixture, "src/main.ts")).replaceAll("\\", "/"),
    label,
    window: [1040, 760],
    env: { SK_APP_ROOT: fixture },
  });
}

test("Gamepad renders live inputs and routes the motors to the selected controller", async () => {
  const probe = await launchGamepadFixture("showcase-gamepad-input");
  try {
    await probe.locator('nav a[href="/gamepad"]').click();
    await ui(probe.locator("#gamepad-native .facts")).toContainText("Connection 41");
    await ui(probe.locator("#gamepad-web .axis output").first()).toHaveText("0.25");
    await choose("Native controller", "92", probe);
    await choose("Web gamepad slot", "2", probe);
    await ui(probe.locator("#gamepad-native .facts")).toContainText("054C:09CC");
    await probe.locator("#gamepad-native-rumble").focus();
    await waitFor('document.activeElement?.id === "gamepad-native-rumble"', probe);
    await probe.eval("showcaseGamepadInput()");
    await ui(probe.locator("#gamepad-confirm-count")).toHaveText("1");
    await ui(probe.locator("#gamepad-native .history")).toContainText("Slot 2 · south down");
    for (const api of ["native", "web"]) {
      const card = "#gamepad-" + api;
      await ui(probe.locator(card + " .axis output").first()).toHaveText("-0.75");
      await ui(probe.locator(card + " .button-value.pressed output")).toHaveText("1.00");
      const track = await probe
        .locator(card + " .track")
        .first()
        .boundingBox();
      const level = await probe
        .locator(card + " .level")
        .first()
        .boundingBox();
      expect(track.w).toBeGreaterThan(20);
      expect(Math.abs(level.w - track.w * 0.375)).toBeLessThan(2);
      expect(Math.abs(level.x - track.x - track.w * 0.125)).toBeLessThan(2);
    }
    await probe.locator('#gamepad-native .pressed kbd[aria-label="Cross button"]').waitFor();
    await choose("Rumble motors", "weak", probe);
    await probe.locator("#gamepad-native-rumble").click();
    await ui(probe.locator("#gamepad-native .outputs output")).toHaveText("Sent an 800 ms pulse");
    await probe.locator("#gamepad-native .outputs button").nth(1).click();
    await ui(probe.locator("#gamepad-native .outputs output")).toHaveText("Stopped");
    await probe.locator("#gamepad-web-rumble").click();
    await ui(probe.locator("#gamepad-web .outputs output")).toHaveText("complete");
    await probe.locator("#gamepad-web .outputs button").nth(1).click();
    expect(JSON.parse(String(await probe.eval("JSON.stringify(showcaseGamepadCommands)")))).toEqual([
      ["native", 92, { strong: 0, weak: 0.65, durationMs: 800 }],
      ["native", 92, { strong: 0, weak: 0.65, durationMs: 0 }],
      ["web", 2, "dual-rumble", { duration: 250, strongMagnitude: 0.45, weakMagnitude: 0.2 }],
      ["web", 2, "reset"],
    ]);
    await probe.eval("showcaseGamepadInput(false); showcaseGamepadInput(); showcaseGamepadInput(false)");
    await ui(probe.locator("#gamepad-confirm-count")).toHaveText("2");
    await ui(probe.locator("#gamepad-native .history .mono").first()).toHaveText("Slot 2 · south up");
    await ui(probe.locator("#gamepad-native .history .mono").nth(1)).toHaveText("Slot 2 · south down");
    await probe.eval("showcaseGamepadAxis(0.284)");
    for (const api of ["native", "web"]) {
      await ui(probe.locator("#gamepad-" + api + " .axis output").first()).toHaveText("0.28");
      expect(await probe.eval(`document.querySelector("#gamepad-${api} .level").style.width`)).toBe("14%");
    }
    await choose("Native controller", "41", probe);
    await choose("Web gamepad slot", "0", probe);
    for (const api of ["native", "web"]) {
      await ui(probe.locator("#gamepad-" + api + " .axis output").first()).toHaveText("0.25");
      expect(await probe.locator("#gamepad-" + api + " .button-value.pressed").count()).toBe(0);
    }
    await probe.eval('window.dispatchEvent(new Event("blur"))');
    await probe.locator("#gamepad-web-empty").waitFor();
    const pausedPolls = await probe.eval("showcaseGamepadStats().polls");
    await Bun.sleep(150);
    expect(await probe.eval("showcaseGamepadStats().polls")).toBe(pausedPolls);
    await probe.eval('window.dispatchEvent(new Event("focus"))');
    await ui(probe.locator("#gamepad-web .axis output").first()).toHaveText("0.25");
    await probe.locator('nav a[href="/window"]').click();
    await waitFor('!document.querySelector("#gamepad-page") && showcaseGamepadStats().listeners === 0', probe);
    const polls = await probe.eval("showcaseGamepadStats().polls");
    await Bun.sleep(150);
    expect(await probe.eval("showcaseGamepadStats().polls")).toBe(polls);
  } finally {
    await probe.close();
  }
}, 60_000);

test("Gamepad dead zones follow the selected controller and inherit across device, model and global settings", async () => {
  const probe = await launchGamepadFixture("showcase-gamepad-dead-zones");
  try {
    await probe.locator('nav a[href="/gamepad"]').click();
    const group = '[role="group"][aria-label="Stick dead zone"]';
    const status = ui(probe.locator(group + " output"));
    const reset = probe.locator(group + " button");
    const scope = (name: string) => choose("Dead zone scope", name, probe);
    const radius = (percent: number) =>
      probe.eval(`{
      const input = document.querySelector('input[aria-label="Dead zone radius (%)"]');
      input.value = "${percent}"; input.dispatchEvent(new Event("change", { bubbles: true }));
    }`);

    await status.toHaveText("Inherited · Effective 5%");
    await scope("global");
    await radius(16);
    await status.toHaveText("Override 16% · Effective 16%");
    await scope("model");
    await status.toHaveText("Inherited · Effective 16%");
    await radius(12);
    await status.toHaveText("Override 12% · Effective 12%");
    await choose("Native controller", "92", probe);
    await status.toHaveText("Inherited · Effective 16%");
    await scope("device");
    await radius(0);
    await status.toHaveText("Override 0% · Effective 0%");
    const counts = "JSON.stringify([showcaseGamepadStats().lists, showcaseGamepadStats().deadZoneReads])";
    const before = await probe.eval(counts);
    await probe.eval("for (let i = 0; i < 40; i++) showcaseGamepadAxis(i / 100)");
    await ui(probe.locator("#gamepad-native .axis output").first()).toHaveText("0.39");
    expect(await probe.eval(counts)).toBe(before);
    await scope("global");
    await radius(25);
    await status.toHaveText("Override 25% · Effective 0%");
    await scope("device");
    await reset.click();
    await status.toHaveText("Inherited · Effective 25%");
    await choose("Native controller", "41", probe);
    await status.toHaveText("Inherited · Effective 12%");
    await radius(7);
    await status.toHaveText("Override 7% · Effective 7%");
    await reset.click();
    await scope("model");
    await reset.click();
    await status.toHaveText("Inherited · Effective 25%");
    await scope("global");
    await reset.click();
    await status.toHaveText("Inherited · Effective 5%");
    expect(await probe.eval(`document.querySelector('${group} button').disabled`)).toBe(true);
  } finally {
    await probe.close();
  }
}, 60_000);

test("shortcut previews distinguish sequences, alternatives and recorded mouse buttons", async () => {
  await page("tray");
  await app.locator('[aria-label="Shortcut platform"]').waitFor();
  await choose("Shortcut platform", "macos");
  await ui(app.locator(".shortcut").first()).toContainText("⇧⌘P");
  expect(await app.locator('kbd[aria-label="Command plus K, then Command plus C"]').count()).toBe(1);
  expect(await app.locator('kbd[aria-label="Command plus Plus, or Command plus Numpad Plus"]').count()).toBe(1);
  expect(await app.locator('kbd[aria-label="Right Control plus Mouse button 4"]').count()).toBe(1);
  await choose("Shortcut platform", "windows");
  await ui(app.locator(".shortcut").first()).toContainText("Ctrl+Shift+P");
}, 30_000);

test("mouse input and controller layouts update the device prompts", async () => {
  await page("tray");
  await waitFor("document.activeViewTransition === null");
  await choose("Shortcut platform", "macos");
  const devices = app.locator('[aria-label="Shortcut device"] button');
  await devices.nth(1).click({ real: true });
  const mouse = ".mouse-test";
  const liveLabel = (name: string) => app.locator(mouse + ' output kbd[aria-label="' + name + '"]').waitFor();
  await app.send("mouse", { selector: mouse, kind: "down", button: 1 });
  await app.send("mouse", { selector: mouse, kind: "up", button: 1 });
  await liveLabel("Middle mouse button");
  await app.send("wheel", { selector: mouse, dy: -60 });
  await liveLabel("Scroll up");
  const mark = () =>
    app.eval(
      'document.querySelector(\'kbd[aria-label="Primary mouse button"] path[fill="currentColor"]\').getAttribute("d")',
    );
  const left = await mark();
  await choose("Primary mouse button", "right");
  expect(await mark()).not.toBe(left);
  await devices.nth(2).click({ real: true });
  for (const [layout, confirm] of [
    ["xbox", "A"],
    ["playstation", "Cross"],
    ["nintendo", "B"],
  ] as const) {
    await choose("Controller layout", layout);
    const legend = app.locator('kbd[aria-label="Return, or ' + confirm + ' button"] svg');
    await legend.waitFor();
    expect((await legend.boundingBox()).w).toBeGreaterThanOrEqual(16);
  }
  await ui(app.locator('kbd[aria-label="ZL, left trigger plus ZR, right trigger"]')).toHaveText("ZL+ZR");
}, 30_000);

test.skipIf(!WINDOWS)(
  "the global keymap is enabled and disposed independently of its hints",
  async () => {
    await page("tray");
    await app.locator('[aria-label="Shortcut device"] button').first().click();
    const toggle = app.locator("#global-keymap-toggle");
    await toggle.click();
    try {
      await ui(toggle).toHaveText("Unbind");
      expect(await app.eval("__sk_appHotkeys.status().running")).toBe(true);
      expect(await app.locator('kbd[data-scope="app"]').count()).toBe(1);
    } finally {
      if (await app.eval('document.querySelector("#global-keymap-toggle").getAttribute("aria-pressed") === "true"'))
        await toggle.click();
    }
    await app.locator('#global-keymap-toggle[aria-pressed="false"] kbd[data-scope="global"]').waitFor();
  },
  30_000,
);

test("the gesture area recognizes a single click, double click, hold and pause", async () => {
  await page("tray");
  await waitFor("document.activeViewTransition === null");
  await app.locator('[aria-label="Shortcut device"] button').nth(1).click({ real: true });
  const status = app.locator('[aria-label="Recognized gesture"]');
  const pause = app.locator('[aria-label="Recognize gestures"]');
  const mouse = (kind: "down" | "up" | "click") => app.send("mouse", { selector: "#gesture-area", kind, button: 0 });
  await mouse("click");
  await ui(status).toHaveText("Single click");
  await mouse("click");
  await mouse("click");
  await ui(status).toHaveText("Double click");
  await mouse("down");
  try {
    await ui(status).toHaveText("Hold");
  } finally {
    await mouse("up");
  }
  await ui(status).toHaveText("Hold");
  await pause.click({ real: true });
  await mouse("click");
  await ui(status).toHaveText("Paused");
  await pause.click({ real: true });
  await ui(status).toHaveText("Ready");
  await app.eval('document.querySelector("#gesture-area").focus()');
  await app.press("Space");
  await ui(status).toHaveText("Single click");
}, 30_000);

test("Context menu pins a file and moves it to another folder from its submenu", async () => {
  await page("context-menu");
  await waitFor("document.activeViewTransition === null");
  const actions = app.locator('[aria-label="Actions for Cover artwork.png"]');
  await actions.click({ real: true });
  await app.locator('[role="menu"]').waitFor();
  await app.press("Enter");
  await ui(app.locator(".file").first()).toContainText("Cover artwork.png");
  await ui(app.locator('[role="status"]')).toHaveText("Pinned Cover artwork.png");

  await actions.press("Shift+F10");
  await app.locator('[role="menu"]').waitFor();
  await app.press("ArrowDown");
  await app.press("ArrowRight");
  await app.locator('[role="menu"][aria-label="Move to"]').waitFor();
  await app.press("Enter");
  await ui(app.locator(".file").first()).toContainText("Drafts");
  await ui(app.locator('[role="status"]')).toHaveText("Moved Cover artwork.png to Drafts");
}, 30_000);

test("Files accepts disk files and exports both the originals and the current editor text", async () => {
  const disk = join(dir, "drop-example.txt");
  writeFileSync(disk, "from disk");
  await page("files");
  const zone = { selector: ".drop-zone" };
  await app.locator(zone.selector).waitFor();
  await waitFor("document.activeViewTransition === null");
  expect((await app.send("drag-files", { kind: "enter", ...zone, paths: [disk] })).dropEffect).toBe("copy");
  await ui(app.locator(zone.selector)).toHaveText("Release to add files");
  expect((await app.send("drag-files", { kind: "drop", ...zone })).dropEffect).toBe("copy");
  await ui(app.locator(".dropped-export")).toHaveText("Drag drop-example.txt out");

  async function dragToZone(selector: string) {
    // A retry after a failure finds the earlier drag still pending.
    await app.send("drag-document", { kind: "cancel" }).catch(() => {});
    await app.eval(`globalThis.droppedNote = null; globalThis.dragStarted = false;
      document.addEventListener("dragstart", () => { globalThis.dragStarted = true; }, {once:true, capture:true});
      document.querySelector(".drop-zone").addEventListener("drop", async e => {
        const f = e.dataTransfer.files[0]; globalThis.droppedNote = [f.name, await f.text()];
      }, {once:true})`);
    await app.send("drag-document", { kind: "arm" });
    const down = await app.send("mouse", { kind: "down", selector });
    await app.send("mouse", { kind: "move", x: Number(down.x) + 9, y: Number(down.y), held: true });
    // The move is coalesced into the next frame, whose dispatch starts the drag.
    await waitFor("dragStarted");
    await app.send("drag-document", { kind: "enter", ...zone });
    expect((await app.send("drag-document", { kind: "over", ...zone })).dropEffect).toBe("copy");
    expect((await app.send("drag-document", { kind: "drop", ...zone })).dropEffect).toBe("copy");
    await waitFor("droppedNote !== null");
    return JSON.parse(String(await app.eval("JSON.stringify(droppedNote)")));
  }
  expect(await dragToZone(".dropped-export")).toEqual(["drop-example.txt", "from disk"]);
  await app.eval(`const editor = document.querySelector("textarea.editor");
    editor.value = "edited in the showcase"; editor.dispatchEvent(new Event("input", {bubbles:true}));`);
  expect(await dragToZone(".note-export")).toEqual(["notes.txt", "edited in the showcase"]);
  await ui(app.locator('[role="status"]')).toHaveText("Copy completed");
}, 30_000);

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

test("CSS feedback follows native activation, value changes, range limits and application snap points", async () => {
  await page("haptics");
  const show = async (selector: string) => {
    await app.eval(`document.querySelector("${selector}").scrollIntoView({block:"center",behavior:"instant"})`);
    await app.locator(selector).waitFor();
  };
  const next = async () => (await app.logs()).next;
  const gestures = async (since: number) =>
    (await app.logs(since)).entries.filter((e) => e.kind === "haptic").map((e) => e.text.split(" ")[0]);
  await show(".double");
  expect(await app.eval("getComputedStyle(document.querySelector('.double')).getPropertyValue('haptic-effect')")).toBe(
    "pattern(--double-tap)",
  );
  const property = () =>
    app.eval("getComputedStyle(document.querySelector('.press')).getPropertyValue('haptic-intensity')");
  expect(await property()).toBe("0.4");
  let since = await next();
  await app.locator(".press").click({ real: true });
  expect(await gestures(since)).toEqual(["press"]);
  since = await next();
  await app.locator(".activate").press("Enter");
  expect(await gestures(since)).toEqual(["activate"]);
  since = await next();
  await app.eval("document.querySelector('.activate').addEventListener('click', e => e.preventDefault(), {once:true})");
  await app.locator(".activate").click({ real: true });
  expect(await gestures(since)).toEqual([]);
  since = await next();
  await app.locator(".toggle").click({ real: true });
  expect(await gestures(since)).toEqual(["change"]);
  await waitFor("getComputedStyle(document.querySelector('.press')).getPropertyValue('haptic-intensity') === '0.15'");
  since = await next();
  await app.locator(".scale").press("ArrowRight");
  await app.locator(".scale").press("End");
  await app.locator(".scale").press("End");
  expect(await gestures(since)).toEqual(["step", "boundary"]);
  since = await next();
  await app.eval(`{ const g = document.querySelector('.guides'); g.style.setProperty('haptic-min-interval','10s'); getComputedStyle(g).getPropertyValue('haptic-min-interval');
    __sk_appHaptics.trigger(g, 'snap', 1); __sk_appHaptics.trigger(g, 'snap', 1); }`);
  expect(await gestures(since)).toEqual(["snap"]);
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
