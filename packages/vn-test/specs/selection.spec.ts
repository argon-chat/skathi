import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/selection/src/main.ts",
  window: [480, 360],
  label: "selection",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Box = { x: number; y: number; w: number; h: number };
async function box(sel: string): Promise<Box> {
  return (await app.query(sel)).nodes[0]!.bounds!;
}
async function mouse(x: number, y: number, kind: string, extra: Record<string, unknown> = {}) {
  await app.send("mouse", { x, y, kind, ...extra });
}
async function drag(from: [number, number], to: [number, number]) {
  await mouse(from[0], from[1], "down");
  for (let i = 1; i <= 4; i++) {
    const t = i / 4;
    await mouse(from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t, "move", { held: true });
    await sleep(30);
  }
  await mouse(to[0], to[1], "up");
  await sleep(120);
}
const selected = async () => String(await app.eval("String(getSelection())"));
const rangeCount = async () => Number(await app.eval("getSelection().rangeCount"));
const state = async () =>
  JSON.parse(String(await app.eval("JSON.stringify(__VN_TEST_SEL__)"))) as { changes: number; starts: number; copies: string[] };
const nowhere = async () => {
  const d = await box("#doc");
  return [d.x + d.w - 4, d.y + d.h - 4] as [number, number];
};
async function clear() {
  const [x, y] = await nowhere();
  await mouse(x, y, "click");
  await sleep(80);
}

function powershell(cmd: string): string {
  const r = spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", cmd], { encoding: "utf8" });
  return r.stdout ?? "";
}

describe("text selection by mouse", () => {
  test("a drag selects across inline runs and paragraphs, and getSelection() reads it", async () => {
    const p1 = await box("#p1");
    const p2 = await box("#p2");
    const before = await state();
    await drag([p1.x + 1, p1.y + p1.h / 2], [p2.x + p2.w - 2, p2.y + p2.h / 2]);
    bunExpect(await selected()).toBe("Hello big world\n\nSecond paragraph here");
    const after = await state();
    bunExpect(after.starts).toBe(before.starts + 1);
    bunExpect(after.changes).toBeGreaterThan(before.changes);
    const ends = JSON.parse(
      String(
        await app.eval(
          "(() => { const s = getSelection(); return JSON.stringify([s.anchorNode.nodeType, s.anchorNode.data, s.anchorOffset, " +
            "s.focusNode.nodeType, s.focusNode.data, s.focusOffset, s.type, s.direction]) })()",
        ),
      ),
    );
    bunExpect(ends).toEqual([3, "Hello ", 0, 3, "Second paragraph here", 21, "Range", "forward"]);
  }, T);

  test("the selection is painted: the highlight is behind the selected text only", async () => {
    const p1 = await box("#p1");
    const p3 = await box("#p3");
    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const tinted = (b: Box) =>
      shot.count({ x: b.x * scale, y: b.y * scale, w: b.w * scale, h: b.h * scale }, (_l, [r, g, b]) => b > r + 25 && b > g + 10);
    bunExpect(tinted(p1)).toBeGreaterThan(50);
    bunExpect(tinted(p3)).toBe(0);
  }, T);

  test("Ctrl+C copies the selected text, with the copy event seeing it", async () => {
    const saved = powershell("Get-Clipboard -Raw");
    try {
      await app.send("key", { key: "Ctrl+C" });
      await sleep(250);
      const got = powershell("Get-Clipboard -Raw").replace(/\r\n/g, "\n").replace(/\n$/, "");
      bunExpect(got).toBe("Hello big world\n\nSecond paragraph here");
      bunExpect((await state()).copies.at(-1)).toBe("Hello big world\n\nSecond paragraph here");
    } finally {
      const escaped = saved.replace(/\r?\n$/, "").replace(/'/g, "''");
      powershell(saved ? `Set-Clipboard -Value '${escaped}'` : "Set-Clipboard -Value $null");
    }
  }, T);

  test("a click without a drag clears it", async () => {
    const p3 = await box("#p3");
    await mouse(p3.x + 10, p3.y + p3.h / 2, "click");
    await sleep(100);
    bunExpect(await rangeCount()).toBe(0);
    bunExpect(await selected()).toBe("");
  }, T);

  test("a double click takes the word, a triple click the paragraph", async () => {
    const big = await box("#big");
    await mouse(big.x + big.w / 2, big.y + big.h / 2, "down", { clicks: 2 });
    await mouse(big.x + big.w / 2, big.y + big.h / 2, "up", { clicks: 2 });
    await sleep(120);
    bunExpect(await selected()).toBe("big");
    const p2 = await box("#p2");
    await mouse(p2.x + 30, p2.y + p2.h / 2, "down", { clicks: 3 });
    await mouse(p2.x + 30, p2.y + p2.h / 2, "up", { clicks: 3 });
    await sleep(120);
    bunExpect(await selected()).toBe("Second paragraph here");
  }, T);

  test("Shift+click extends the selection", async () => {
    const p3 = await box("#p3");
    await mouse(p3.x + p3.w - 2, p3.y + p3.h / 2, "down", { shift: true });
    await mouse(p3.x + p3.w - 2, p3.y + p3.h / 2, "up", { shift: true });
    await sleep(120);
    const text = await selected();
    bunExpect(text.startsWith("Second paragraph here")).toBe(true);
    bunExpect(text.endsWith("Third one")).toBe(true);
    bunExpect(text).not.toContain("Not selectable");
  }, T);

  test("user-select: none starts no selection, and a press there keeps the current one", async () => {
    const none = await box("#none");
    const kept = await selected();
    await mouse(none.x + 10, none.y + none.h / 2, "click");
    await sleep(100);
    bunExpect(await selected()).toBe(kept);
    await clear();
    const p3 = await box("#p3");
    await drag([none.x + 4, none.y + none.h / 2], [p3.x + p3.w - 2, p3.y + p3.h / 2]);
    bunExpect(await rangeCount()).toBe(0);
  }, T);

  test("a drag that starts on a button selects nothing", async () => {
    const btn = await box("#btn");
    const p3 = await box("#p3");
    await drag([btn.x + btn.w / 2, btn.y + btn.h / 2], [p3.x + 2, p3.y + p3.h / 2]);
    bunExpect(await rangeCount()).toBe(0);
  }, T);

  test("Ctrl+A outside a field selects the document; a press into the field clears it", async () => {
    await clear();
    await app.send("key", { key: "Ctrl+A" });
    await sleep(120);
    const all = await selected();
    bunExpect(all).toContain("Hello big world");
    bunExpect(all).toContain("Third one");
    bunExpect(all).not.toContain("Not selectable");
    const field = await box("#field");
    await mouse(field.x + 10, field.y + field.h / 2, "click");
    await sleep(120);
    bunExpect(await rangeCount()).toBe(0);
  }, T);

  test("a drag held below a scroll box scrolls it, and the selection follows", async () => {
    await clear();
    const sc = await box("#scroller");
    const below: [number, number] = [sc.x + 40, sc.y + sc.h + 30];
    await mouse(sc.x + 1, sc.y + 8, "down");
    for (let i = 1; i <= 3; i++) {
      await mouse(sc.x + 20, sc.y + 8 + (sc.h + 22) * (i / 3), "move", { held: true });
      await sleep(30);
    }
    await sleep(600);
    await mouse(below[0], below[1], "move", { held: true });
    await mouse(below[0], below[1], "up");
    await sleep(120);
    const top = Number(await app.eval("document.querySelector('#scroller').scrollTop"));
    bunExpect(top).toBeGreaterThan(0);
    const text = await selected();
    bunExpect(text.startsWith("Line 1\n")).toBe(true);
    bunExpect(text).toContain("Line 4");
  }, T);

  test("a selection set from script paints and reads back", async () => {
    await clear();
    await app.eval("getSelection().selectAllChildren(document.querySelector('#p1'))");
    await sleep(120);
    bunExpect(await selected()).toBe("Hello big world");
    const p1 = await box("#p1");
    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const n = shot.count({ x: p1.x * scale, y: p1.y * scale, w: p1.w * scale, h: p1.h * scale }, (_l, [r, g, b]) => b > r + 25);
    bunExpect(n).toBeGreaterThan(50);
    await app.eval("getSelection().removeAllRanges()");
    bunExpect(await rangeCount()).toBe(0);
  }, T);
});
