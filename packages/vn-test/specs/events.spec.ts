import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "events",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The events recorded since the last call. */
let seen = 0;
async function fresh(): Promise<string[]> {
  await sleep(200);
  const all = JSON.parse((await app.eval("JSON.stringify(__VN_TEST_EVENTS__.events)")) as string) as string[];
  const out = all.slice(seen);
  seen = all.length;
  return out;
}

async function centre(sel: string): Promise<string> {
  const b = await app.locator(sel).boundingBox();
  return `x=${Math.round(b.x + b.w / 2)} y=${Math.round(b.y + b.h / 2)}`;
}

describe("<img> load and error", () => {
  test("every src is answered: a data: PNG and a blob: URL load with their size, a missing file errors", async () => {
    await expect(app.locator("#ok")).toHaveCount(1);
    let got: string[] = [];
    for (let i = 0; i < 20 && got.length < 4; i++) got = got.concat(await fresh());
    bunExpect(got.sort()).toEqual(
      [
        "load@ok|bubbles=false complete=true w=8 h=6",
        "onload@ok|w=8",
        "error@bad|bubbles=false complete=true w=0 h=0",
        "load@blob|bubbles=false complete=true w=8 h=6",
      ].sort(),
    );
  }, T);
});

describe("contextmenu from the mouse", () => {
  test("a right click: one mousedown and one mouseup per listener, then contextmenu — cancelled, it opens the page's menu", async () => {
    await fresh();
    await app.locator("#inner").click({ real: true, button: 2 });
    const at = await centre("#inner");
    const got = await fresh();
    bunExpect(got).toEqual([
      `mousedown@inner|t=inner b=2 bs=2 d=1 ${at} c=true`,
      `mousedown@trigger|t=inner b=2 bs=2 d=1 ${at} c=true`,
      `mouseup@inner|t=inner b=2 bs=0 d=1 ${at} c=true`,
      `mouseup@trigger|t=inner b=2 bs=0 d=1 ${at} c=true`,
      // The right button's click, before the menu: Chromium on Windows finishes the release before
      // it shows one.
      `auxclick@trigger|t=inner b=2 bs=0 d=1 ${at} c=true`,
      `contextmenu@trigger|t=inner b=2 bs=0 d=0 ${at} c=true`,
    ]);
    await expect(app.locator("#menu")).toHaveCount(1);
  }, T);

  test("a middle click: mousedown, mouseup, then auxclick (button 1), and no click", async () => {
    await fresh();
    await app.locator("#inner").click({ real: true, button: 1 });
    const at = await centre("#inner");
    bunExpect(await fresh()).toEqual([
      `mousedown@inner|t=inner b=1 bs=4 d=1 ${at} c=true`,
      `mousedown@trigger|t=inner b=1 bs=4 d=1 ${at} c=true`,
      `mouseup@inner|t=inner b=1 bs=0 d=1 ${at} c=true`,
      `mouseup@trigger|t=inner b=1 bs=0 d=1 ${at} c=true`,
      `auxclick@trigger|t=inner b=1 bs=0 d=1 ${at} c=true`,
    ]);
  }, T);

  test("a left click: mouseup BEFORE click, each once, and click's detail is the count", async () => {
    await fresh();
    await app.locator("#inner").click({ real: true });
    const at = await centre("#inner");
    bunExpect(await fresh()).toEqual([
      `mousedown@inner|t=inner b=0 bs=1 d=1 ${at} c=true`,
      `mousedown@trigger|t=inner b=0 bs=1 d=1 ${at} c=true`,
      `mouseup@inner|t=inner b=0 bs=0 d=1 ${at} c=true`,
      `mouseup@trigger|t=inner b=0 bs=0 d=1 ${at} c=true`,
      `click@trigger|t=inner b=0 bs=0 d=1 ${at} c=true`,
    ]);
  }, T);

  test("an uncancelled one says, once, that no native menu opens", async () => {
    await fresh();
    await app.locator("#plain").click({ real: true, button: 2 });
    await app.locator("#plain").click({ real: true, button: 2 });
    bunExpect((await fresh()).map((l) => l.split("|")[0])).toEqual(["contextmenu@plain", "contextmenu@plain"]);
    const { entries } = await app.logs(0);
    const advisories = entries.filter((e) => e.text.includes("an uncancelled `contextmenu`"));
    bunExpect(advisories.length).toBe(1);
  }, T);
});

describe("contextmenu from the keyboard", () => {
  test("the Menu key (on its keyup) and Shift+F10 fire it at the focused element, at its centre, with no button", async () => {
    await app.locator("#pad").focus();
    await fresh();
    await app.locator("#pad").press("Menu");
    const at = await centre("#pad");
    bunExpect(await fresh()).toEqual(["keydown@pad|k=ContextMenu", `contextmenu@pad|t=pad b=0 bs=0 d=0 ${at} c=true`]);
    await app.locator("#pad").press("Shift+F10");
    bunExpect(await fresh()).toEqual(["keydown@pad|k=F10", `contextmenu@pad|t=pad b=0 bs=0 d=0 ${at} c=true`]);
  }, T);

  test("a cancelled Menu-key keyup has no contextmenu", async () => {
    await app.locator("#pad2").focus();
    await fresh();
    await app.locator("#pad2").press("Menu");
    bunExpect(await fresh()).toEqual(["keyup@pad2|cancelled"]);
  }, T);

  test("a keyboard click is detail 0", async () => {
    await app.locator("#key").focus();
    await fresh();
    await app.locator("#key").press("Enter");
    const got = await fresh();
    bunExpect(got.length).toBe(1);
    bunExpect(got[0]).toMatch(/^click@key\|t=key b=0 bs=0 d=0 /);
  }, T);
});
