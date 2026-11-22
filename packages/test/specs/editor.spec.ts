import { afterAll, beforeEach, describe, expect, test } from "bun:test";
import {
  CF_UNICODETEXT,
  clipboardFormat,
  clipboardRestore,
  clipboardSnapshot,
  launch,
  MACOS,
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
