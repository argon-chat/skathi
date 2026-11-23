import { afterAll, beforeEach, describe, expect, test } from "bun:test";
import {
  CF_UNICODETEXT,
  clipboardFormat,
  clipboardRestore,
  clipboardSnapshot,
  launch,
  MACOS,
  readPng,
  SCREENSHOTS,
  WINDOWS,
} from "../src/index";

/** The system clipboard's text, read in this process; `null` when it holds none. */
function clipboardText(): string | null {
  const bytes = clipboardFormat(CF_UNICODETEXT);
  if (!bytes) return null;
  const units = new Uint16Array(bytes.buffer, bytes.byteOffset, bytes.byteLength >> 1);
  const end = units.indexOf(0);
  return String.fromCharCode(...(end < 0 ? units : units.subarray(0, end)));
}

const app = await launch({
  app: "packages/test/fixtures/editor/src/main.ts",
  window: [480, 360],
  label: "editor",
});
afterAll(() => app.close());

const T = 30_000;
const MOD = MACOS ? "Cmd" : "Ctrl";
const REDO = MACOS ? "Cmd+Shift+Z" : "Ctrl+Y";
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Json = string | number | boolean | null | Json[] | { [key: string]: Json };

async function js(src: string): Promise<Json> {
  const r = await app.eval(`JSON.stringify((() => { const E = __SK_TEST_EDITOR__; return ${src}; })())`);
  return JSON.parse(String(r)) as Json;
}

async function read<V>(src: string): Promise<V> {
  return (await js(src)) as V;
}

async function until<V>(src: string, ok: (v: V) => boolean): Promise<V> {
  const deadline = Date.now() + 3_000;
  for (;;) {
    const v = await read<V>(src);
    if (ok(v) || Date.now() > deadline) return v;
    await sleep(50);
  }
}

async function reset(value?: unknown) {
  await app.eval(`__SK_TEST_EDITOR__.reset(${value === undefined ? "" : JSON.stringify(value)})`);
  await sleep(100);
}

const ime = (text: string, commit = false) => app.send("ime", { selector: "#ed", text, ...(commit ? { commit } : {}) });

beforeEach(() => reset());

describe("<sk-editor> editing", () => {
  test(
    "typing reaches the model, v-model style, with beforeinput ranges",
    async () => {
      await app.locator("#ed").type("ab");
      const text = await until<string>("E.draft.value.text", (v) => v === "ab");
      expect(text).toBe("ab");
      expect(await js("E.state.log")).toEqual(["insertText|a|0-0", "insertText|b|1-1"]);
      expect(await js("E.ed().length")).toBe(2);
    },
    T,
  );

  test(
    "Enter is insertParagraph and cancellable; Shift+Enter is insertLineBreak",
    async () => {
      await app.locator("#ed").type("hi");
      await app.eval("__SK_TEST_EDITOR__.state.blockEnter = true");
      await app.locator("#ed").press("Enter");
      await app.eval("__SK_TEST_EDITOR__.state.blockEnter = false");
      await app.locator("#ed").press("Shift+Enter");
      const text = await until<string>("E.draft.value.text", (v) => v === "hi\n");
      expect(text).toBe("hi\n");
      expect(await js("E.state.log.slice(2)")).toEqual([
        "insertParagraph|null|2-2",
        "sent",
        "insertLineBreak|null|2-2",
      ]);
      expect(await js("E.ed().lineCount")).toBe(2);
    },
    T,
  );

  test(
    "a typed run undoes in one step and redoes, and change reports just that text",
    async () => {
      await reset({ text: "x\ny", marks: [], atoms: [] });
      await app.locator("#ed").focus();
      await app.eval("__SK_TEST_EDITOR__.ed().dispatch({ selections: [{ anchor: 1 }] })");
      await app.locator("#ed").type("abc");
      await until<string>("E.draft.value.text", (v) => v === "xabc\ny");
      const last = (n: number) => until<unknown[]>("E.state.changes", (v) => v.length === n);
      await last(3);
      await app.locator("#ed").press(`${MOD}+Z`);
      expect(await until<string>("E.draft.value.text", (v) => v === "x\ny")).toBe("x\ny");
      expect((await last(4)).at(-1)).toEqual([{ from: 1, to: 4, insert: "" }]);
      await app.locator("#ed").press(REDO);
      expect(await until<string>("E.draft.value.text", (v) => v === "xabc\ny")).toBe("xabc\ny");
      expect((await last(5)).at(-1)).toEqual([{ from: 1, to: 1, insert: "abc" }]);
    },
    T,
  );

  test(
    "undo() of a transaction reports its inverse, one change per range",
    async () => {
      await reset({ text: "hello world", marks: [], atoms: [] });
      await app.eval(
        "__SK_TEST_EDITOR__.ed().dispatch({ changes: [{ from: 6, to: 11, insert: 'there' }, { from: 0, to: 5, insert: 'Hi' }] })",
      );
      expect(await js("E.ed().undo()")).toBe(true);
      expect(await js("E.ed().value.text")).toBe("hello world");
      expect(await js("E.state.changes")).toEqual([
        [
          { from: 0, to: 5, insert: "Hi" },
          { from: 6, to: 11, insert: "there" },
        ],
        [
          { from: 0, to: 2, insert: "hello" },
          { from: 3, to: 8, insert: "world" },
        ],
      ]);
    },
    T,
  );

  test(
    "Backspace and arrows treat an atom as one character",
    async () => {
      await reset({ text: "a￼b", marks: [], atoms: [{ id: "m1", at: 1, type: "mention", text: "@Ann" }] });
      await app.locator("#ed").focus();
      await app.eval("__SK_TEST_EDITOR__.ed().dispatch({ selections: [{ anchor: 3 }] })");
      await app.locator("#ed").press("ArrowLeft");
      await app.locator("#ed").press("Backspace");
      const v = await until<{ text: string; atoms: unknown[] }>("E.draft.value", (v) => v.text === "ab");
      expect(v).toMatchObject({ text: "ab", atoms: [] });
    },
    T,
  );
});

describe("<sk-editor> JS API", () => {
  test(
    "dispatch: marks, selection, change events and typing inside a mark",
    async () => {
      await reset({ text: "hello world", marks: [], atoms: [] });
      await app.eval(
        "__SK_TEST_EDITOR__.ed().dispatch({ addMarks: [{ type: 'bold', from: 0, to: 5 }], selections: [{ anchor: 3 }] })",
      );
      expect(await js("E.ed().marksAt(3)")).toEqual(["bold"]);
      expect(await js("E.ed().selections")).toEqual([{ anchor: 3, head: 3 }]);
      await app.locator("#ed").type("X");
      const v = await until<{ text: string; marks: { type: string; from: number; to: number }[] }>(
        "E.draft.value",
        (v) => v.text === "helXlo world",
      );
      expect(v.marks).toEqual([{ type: "bold", from: 0, to: 6 }]);
      await app.eval("__SK_TEST_EDITOR__.ed().dispatch({ changes: [{ from: 0, to: 3, insert: 'J' }] })");
      expect(await js("E.state.changes.at(-1)")).toEqual([{ from: 0, to: 3, insert: "J" }]);
      expect(await js("E.ed().slice(0, 4)")).toBe("JXlo");
      expect(await js("E.ed().line(0)")).toEqual({ from: 0, to: 10, text: "JXlo world" });
    },
    T,
  );

  test(
    "overlapping changes throw; decorations stay out of the value and the history",
    async () => {
      await reset({ text: "let x", marks: [], atoms: [] });
      const thrown = await read<string>(
        "(() => { try { E.ed().dispatch({ changes: [{ from: 0, to: 3 }, { from: 2, to: 4 }] }); return 'none' } catch (e) { return e.name } })()",
      );
      expect(thrown).toBe("RangeError");
      await app.eval("__SK_TEST_EDITOR__.ed().setDecorations('syntax', [{ from: 0, to: 3, name: 'kw' }])");
      expect(await js("E.ed().value.marks")).toEqual([]);
      expect(await js("E.ed().undo()")).toBe(false);
    },
    T,
  );

  test(
    "layout: an atom takes its element's width, a big mark makes its row taller, a quote line is indented",
    async () => {
      await reset({
        text: "a￼b\nquoted",
        marks: [],
        atoms: [{ id: "m1", at: 1, type: "mention" }],
        lines: [{ line: 1, type: "quote" }],
      });
      await sleep(200);
      const atom = await read<{ width: number }[]>("E.ed().rectsFor(1, 2).map(r => ({ width: r.width }))");
      expect(atom.length).toBe(1);
      expect(atom[0]!.width).toBeGreaterThanOrEqual(47);
      const plain = await read<{ x: number; h: number }>(
        "(() => { const r = E.ed().rectsFor(0, 1)[0]; return { x: r.x, h: r.height } })()",
      );
      const quoted = await read<{ x: number }>("(() => { const r = E.ed().rectsFor(4, 5)[0]; return { x: r.x } })()");
      expect(quoted.x - plain.x).toBeGreaterThanOrEqual(18);
      await app.eval("__SK_TEST_EDITOR__.ed().dispatch({ addMarks: [{ type: 'big', from: 0, to: 1 }] })");
      await sleep(200);
      const tall = await read<number>("E.ed().rectsFor(2, 3)[0].height");
      expect(tall).toBeGreaterThan(plain.h * 1.5);
      const at = await read<number>(
        "(() => { const r = E.ed().rectsFor(4, 5)[0]; return E.ed().positionAt(r.x + 1, r.y + r.height / 2) })()",
      );
      expect(at).toBe(4);
    },
    T,
  );

  test(
    "vertical-align on an atom: middle makes the row shorter than baseline; top and bottom meet the row's edges",
    async () => {
      await reset({ text: "a￼b", marks: [], atoms: [{ id: "m1", at: 1, type: "mention" }] });
      const measure = async (value: string) => {
        await app.eval(`__SK_TEST_EDITOR__.align.value = ${JSON.stringify(value)}`);
        await sleep(200);
        const row = await read<{ y: number; height: number }>(
          "(() => { const r = E.ed().rectsFor(0, 1)[0]; return { y: r.y, height: r.height } })()",
        );
        const chip = await app.locator('[atom="m1"]').boundingBox();
        return { row, chip };
      };
      const base = await measure("baseline");
      const middle = await measure("middle");
      expect(middle.row.height).toBeLessThan(base.row.height - 2);
      const top = await measure("top");
      expect(Math.abs(top.chip.y - top.row.y)).toBeLessThan(1.5);
      const bottom = await measure("bottom");
      expect(Math.abs(bottom.chip.y + bottom.chip.h - (bottom.row.y + bottom.row.height))).toBeLessThan(1.5);
    },
    T,
  );
});

type Rect = { x: number; y: number; width: number; height: number };

async function code(text: string) {
  await app.eval(
    `(() => { const c = __SK_TEST_EDITOR__.code(); c.value = ${JSON.stringify({ text, marks: [], atoms: [] })}; c.setLineDecorations("d", []); c.dispatch({ selections: [{ anchor: 0 }] }) })()`,
  );
  await sleep(200);
}

describe("<sk-editor> long documents and code", () => {
  test(
    "a long document lays out the lines on screen, and geometry reads still answer anywhere",
    async () => {
      const n = 20_000;
      await code(Array.from({ length: n }, (_, i) => `L${i}`).join("\n"));
      const view = await read<{ fromLine: number; toLine: number }>("E.code().viewport");
      expect(view.toLine - view.fromLine).toBeLessThanOrEqual(8);
      expect(Math.abs((await read<number>("E.code().scrollHeight")) - n * 16)).toBeLessThan(2);

      await app.eval("__SK_TEST_EDITOR__.code().scrollTop = 160000");
      const moved = await until<{ fromLine: number }>("E.code().viewport", (v) => v.fromLine > 9000);
      expect(Math.abs(moved.fromLine - 10_000)).toBeLessThanOrEqual(1);

      const box = await app.locator("#code").boundingBox();
      const near = await read<Rect>(
        "(() => { const l = E.code().line(10001); return E.code().rectsFor(l.from, l.from + 1)[0] })()",
      );
      const far = await read<Rect>(
        "(() => { const l = E.code().line(19000); return E.code().rectsFor(l.from, l.from + 1)[0] })()",
      );
      expect(Math.abs(far.y - box.y - (19_000 * 16 - 160_000))).toBeLessThan(1);
      expect(far.x).toBeCloseTo(near.x, 1);
      expect(far.width).toBeCloseTo(near.width, 1);

      await app.eval(
        "(() => { const c = __SK_TEST_EDITOR__.code(); c.dispatch({ selections: [{ anchor: c.line(500).from }] }) })()",
      );
      const caret = await until<Rect>("E.code().caretRect()", (r) => r.y >= box.y && r.y < box.y + box.h);
      expect(caret.y).toBeGreaterThanOrEqual(box.y);
      expect(caret.y + caret.height).toBeLessThanOrEqual(box.y + box.h + 0.5);
    },
    T,
  );

  test(
    "scrolling over wrapped lines whose estimated heights get corrected moves the text by the wheel delta",
    async () => {
      const lines = Array.from({ length: 3000 }, (_, i) =>
        i % 5 === 0 ? `${i} ${"wrapping words go on ".repeat(1 + (i % 7))}` : `${i} short`,
      );
      await reset({ text: lines.join("\n"), marks: [], atoms: [] });
      await app.eval("__SK_TEST_EDITOR__.ed().dispatch({ selections: [{ anchor: 0 }] })");
      await sleep(200);
      const at = (src: string) =>
        read<{ line: number; y: number }>(
          `(() => { const e = E.ed(); const n = ${src}; const l = e.line(n); return { line: n, y: e.rectsFor(l.from, l.from + 1)[0].y } })()`,
        );
      for (const dy of [90, 90, 2000, 90, 5000, 90]) {
        const before = await at("E.ed().viewport.fromLine + 1");
        await app.send("wheel", { selector: "#ed", dx: 0, dy });
        await sleep(80);
        const after = await at(String(before.line));
        expect(before.y - after.y).toBeCloseTo(dy, 0);
      }
    },
    T,
  );

  test(
    "an editor as tall as its text inside a scroller lays out the lines the scroller shows",
    async () => {
      const n = 6000;
      await app.eval(
        `(() => { const t = __SK_TEST_EDITOR__.tall(); t.value = ${JSON.stringify({ text: Array.from({ length: n }, (_, i) => `row ${i}`).join("\n"), marks: [], atoms: [] })}; t.dispatch({ selections: [{ anchor: 0 }] }) })()`,
      );
      await sleep(300);
      const box = await app.locator("#tall").boundingBox();
      expect(Math.abs(box.h - 8 - n * 16)).toBeLessThan(2);
      const view = await read<{ fromLine: number; toLine: number }>("E.tall().viewport");
      expect(view.toLine - view.fromLine).toBeLessThanOrEqual(5);
      await app.eval("document.querySelector('#scroller').scrollTop = 48000");
      const moved = await until<{ fromLine: number }>("E.tall().viewport", (v) => v.fromLine > 2000);
      expect(Math.abs(moved.fromLine - 3000)).toBeLessThanOrEqual(1);
    },
    T,
  );

  test(
    "white-space: pre keeps a long line on one row and scrolls it sideways; pre-wrap wraps it",
    async () => {
      await code("x".repeat(200));
      expect((await read<Rect[]>("E.code().rectsFor(0, 200)")).length).toBe(1);
      expect(await read<number>("E.code().scrollWidth")).toBeGreaterThan(await read<number>("E.code().clientWidth"));
      const before = await read<Rect>("E.code().rectsFor(5, 6)[0]");
      await app.eval("__SK_TEST_EDITOR__.code().scrollLeft = 100");
      const after = await until<Rect>("E.code().rectsFor(5, 6)[0]", (r) => r.x < before.x - 50);
      expect(after.x).toBeCloseTo(before.x - 100, 0);

      await reset({ text: "word ".repeat(40), marks: [], atoms: [] });
      await sleep(200);
      expect((await read<Rect[]>("E.ed().rectsFor(0, 200)")).length).toBeGreaterThan(1);
    },
    T,
  );

  test(
    "the gutter numbers the lines and makes room for line decorations' gutter text",
    async () => {
      await code("a\nb");
      const box = await app.locator("#code").boundingBox();
      const plain = await read<Rect>("E.code().rectsFor(0, 1)[0]");
      expect(plain.x - box.x).toBeGreaterThan(20);
      await app.eval(
        "__SK_TEST_EDITOR__.code().setLineDecorations('d', [{ line: 1, name: 'added', gutter: '+12345' }])",
      );
      const wider = await until<Rect>("E.code().rectsFor(0, 1)[0]", (r) => r.x > plain.x + 20);
      expect(wider.x - plain.x).toBeGreaterThan(30);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "a rounded decoration over two lines rounds only its outer ends",
    async () => {
      const blank = " ".repeat(10);
      await code(`${blank}\n${blank}\nx`);
      await app.eval(
        "(() => { const c = __SK_TEST_EDITOR__.code(); c.dispatch({ selections: [{ anchor: 23 }] }); c.setDecorations('d', [{ from: 0, to: 21, name: 'hit' }]) })()",
      );
      await sleep(200);
      const first = await read<Rect>("E.code().rectsFor(0, 10)[0]");
      const second = await read<Rect>("E.code().rectsFor(11, 21)[0]");
      const shot = readPng(await app.screenshot());
      const k = shot.width / 480;
      // The device pixel in each corner that lies wholly inside the rect: yellow for a square
      // corner, outside the arc of the 8px radius for a rounded one.
      const corner = (r: Rect, right: boolean, bottom: boolean) =>
        shot.at(
          right ? Math.floor((r.x + r.width) * k) - 1 : Math.ceil(r.x * k),
          bottom ? Math.floor((r.y + r.height) * k) - 1 : Math.ceil(r.y * k),
        )[2];
      const round = (r: Rect, right: boolean, bottom: boolean) => expect(corner(r, right, bottom)).toBeGreaterThan(150);
      const square = (r: Rect, right: boolean, bottom: boolean) => expect(corner(r, right, bottom)).toBeLessThan(60);
      expect(shot.at(Math.round((first.x + 8) * k), Math.round((first.y + 8) * k))[2]).toBeLessThan(60);
      round(first, false, false);
      round(first, false, true);
      square(first, true, false);
      square(first, true, true);
      square(second, false, false);
      square(second, false, true);
      round(second, true, false);
      round(second, true, true);
      await app.eval("__SK_TEST_EDITOR__.code().setDecorations('d', [])");
    },
    T,
  );

  test(
    "PageUp and PageDown move a page and scroll with it, keeping the column; the document keys reach either end",
    async () => {
      const n = 100_000;
      await code(
        Array.from({ length: n }, (_, i) => (i === 4 ? "x" : `${String(i).padStart(6, "0")} abcdefgh`)).join("\n"),
      );
      const ed = app.locator("#code");
      await ed.focus();
      const sel = "E.code().selections[0]";
      const pos = (line: number, col: number) => read<number>(`E.code().line(${line}).from + ${col}`);
      const headAt = async (want: number) => (await until<{ head: number }>(sel, (s) => s.head === want)).head;
      await app.eval(
        "(() => { const c = __SK_TEST_EDITOR__.code(); c.dispatch({ selections: [{ anchor: c.line(0).from + 9 }] }) })()",
      );

      await ed.press("PageDown");
      expect(await headAt(await pos(3, 9))).toBe(await pos(3, 9));
      expect(await read<number>("E.code().scrollTop")).toBe(48);
      await ed.press("ArrowDown");
      expect(await headAt(await pos(4, 1))).toBe(await pos(4, 1));
      await ed.press("ArrowDown");
      expect(await headAt(await pos(5, 9))).toBe(await pos(5, 9));
      await ed.press("PageDown");
      expect(await headAt(await pos(8, 9))).toBe(await pos(8, 9));
      await ed.press("Shift+PageUp");
      expect(await until<{ anchor: number; head: number }>(sel, (s) => s.head !== s.anchor)).toEqual({
        anchor: await pos(8, 9),
        head: await pos(5, 9),
      });

      const [docStart, docEnd] = MACOS ? ["Cmd+ArrowUp", "Cmd+ArrowDown"] : ["Ctrl+Home", "Ctrl+End"];
      const length = await read<number>("E.code().length");
      await ed.press(docEnd);
      expect(await headAt(length)).toBe(length);
      const view = await until<{ fromLine: number; toLine: number }>("E.code().viewport", (v) => v.toLine === n);
      expect(view.toLine).toBe(n);
      expect(view.toLine - view.fromLine).toBeLessThanOrEqual(8);
      await ed.press(`Shift+${docStart}`);
      expect(await until<{ anchor: number; head: number }>(sel, (s) => s.head === 0)).toEqual({
        anchor: length,
        head: 0,
      });
      expect(await until<number>("E.code().scrollTop", (v) => v === 0)).toBe(0);
      await ed.press(docEnd);
      await ed.press(docStart);
      expect(await until<{ anchor: number; head: number }>(sel, (s) => s.anchor === 0)).toEqual({ anchor: 0, head: 0 });
    },
    T,
  );

  test(
    "tab-size: a tab stops where that many spaces end, in the editor and in text",
    async () => {
      await code("\tx\n    x");
      const tabbed = await read<Rect>("E.code().rectsFor(1, 2)[0]");
      const spaced = await read<Rect>(
        "(() => { const l = E.code().line(1); return E.code().rectsFor(l.from + 4, l.from + 5)[0] })()",
      );
      expect(tabbed.x).toBeCloseTo(spaced.x, 0);

      const four = await app.locator("#tabbed").boundingBox();
      const spaces = await app.locator("#spaced").boundingBox();
      expect(Math.abs(four.w - spaces.w)).toBeLessThan(1);
      await app.eval("__SK_TEST_EDITOR__.tabs.value = 8");
      await sleep(200);
      const eight = await app.locator("#tabbed").boundingBox();
      expect(eight.w).toBeGreaterThan(spaces.w * 1.6);
      await app.eval("__SK_TEST_EDITOR__.tabs.value = 4");
    },
    T,
  );
});

describe("<sk-editor> IME and clipboard", () => {
  test.skipIf(!WINDOWS)(
    "a composition lands as one edit",
    async () => {
      await app.locator("#ed").focus();
      await ime("n");
      await ime("ни");
      await ime("ник", true);
      expect(await until<string>("E.draft.value.text", (v) => v === "ник")).toBe("ник");
    },
    T,
  );

  test.skipIf(process.platform !== "win32")(
    "copy keeps marks between editors; other apps get the plain text",
    async () => {
      const saved = clipboardSnapshot();
      try {
        clipboardRestore([]);
        await reset({ text: "bold!", marks: [{ type: "bold", from: 0, to: 4 }], atoms: [] });
        await app.locator("#ed").press(`${MOD}+A`);
        await app.locator("#ed").press(`${MOD}+C`);
        const deadline = Date.now() + 3_000;
        let plain = clipboardText();
        while (plain !== "bold!" && Date.now() < deadline) {
          await sleep(25);
          plain = clipboardText();
        }
        expect(plain).toBe("bold!");
        await app.locator("#ed2").press(`${MOD}+V`);
        const v = await until<{ text: string; marks: unknown[] }>("E.ed2().value", (v) => v.text === "bold!");
        expect(v.marks).toEqual([{ type: "bold", from: 0, to: 4 }]);
      } finally {
        clipboardRestore(saved);
      }
    },
    T,
  );
});
