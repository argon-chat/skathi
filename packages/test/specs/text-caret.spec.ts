import { afterAll, expect, test } from "bun:test";
import { launch, MACOS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/text-caret/src/main.ts",
  window: [480, 320],
  label: "text-caret",
});
afterAll(() => app.close());

const T = 30_000;
const UNDO = MACOS ? "Cmd+Z" : "Ctrl+Z";

async function read(js: string): Promise<string> {
  const r = await app.eval(js);
  return String(r?.value ?? r);
}

async function until(js: string, want: string): Promise<string> {
  const deadline = Date.now() + 3_000;
  for (;;) {
    const got = await read(js);
    if (got === want || Date.now() > deadline) return got;
    await Bun.sleep(50);
  }
}

const TA = "document.querySelector('#ta')";
const CARET = `(() => { const t = ${TA}; return [t.selectionStart, t.selectionEnd, t.selectionDirection].join(","); })()`;
const VALUE = `JSON.stringify(${TA}.value)`;
const FORWARD = MACOS ? "none" : "forward";

async function reset(): Promise<void> {
  await app.eval(`(${TA}.value = "", __SK_TEST_CARET__.text.value = "", __SK_TEST_CARET__.log.length = 0, 0)`);
  await app.locator("#ta").focus();
  await Bun.sleep(100);
}

const logged = async (): Promise<string[]> => {
  await Bun.sleep(150);
  return JSON.parse(await read("JSON.stringify(__SK_TEST_CARET__.log.splice(0))")) as string[];
};

test(
  "selectionStart and selectionEnd follow typing, arrows, a backward selection and an IME commit, in UTF-16 units",
  async () => {
    await reset();
    await app.locator("#ta").type("abc");
    expect(await until(CARET, `3,3,${FORWARD}`)).toBe(`3,3,${FORWARD}`);
    await app.locator("#ta").press("ArrowLeft");
    expect(await until(CARET, `2,2,${FORWARD}`)).toBe(`2,2,${FORWARD}`);
    await app.locator("#ta").press("Shift+ArrowLeft");
    expect(await until(CARET, "1,2,backward")).toBe("1,2,backward");
    await app.send("ime", { selector: "#ta", text: "🙂", commit: true });
    expect(await until(VALUE, JSON.stringify("a🙂c"))).toBe(JSON.stringify("a🙂c"));
    expect(await until(CARET, `3,3,${FORWARD}`)).toBe(`3,3,${FORWARD}`);
  },
  T,
);

test(
  "the document start and end keys move and extend the caret in a textarea",
  async () => {
    await reset();
    const [start, end] = MACOS ? ["Cmd+ArrowUp", "Cmd+ArrowDown"] : ["Ctrl+Home", "Ctrl+End"];
    await read(`(${TA}.value = "ab\\ncd\\nef", ${TA}.selectionStart = ${TA}.selectionEnd = 4, 0)`);
    await app.locator("#ta").press(end);
    expect(await until(CARET, `8,8,${FORWARD}`)).toBe(`8,8,${FORWARD}`);
    await app.locator("#ta").press(start);
    expect(await until(CARET, `0,0,${FORWARD}`)).toBe(`0,0,${FORWARD}`);
    await app.locator("#ta").press(`Shift+${end}`);
    expect(await until(CARET, `0,8,${FORWARD}`)).toBe(`0,8,${FORWARD}`);
  },
  T,
);

test(
  "keydown and beforeinput see the selection the key acts on, input the one after it",
  async () => {
    await reset();
    await app.locator("#ta").type("ab");
    await app.locator("#ta").press("ArrowLeft");
    await logged();
    await app.locator("#ta").type("x");
    expect(await logged()).toEqual(["keydown:x@1,1", "beforeinput:insertText@1,1", "input:insertText@2,2"]);
  },
  T,
);

test(
  "a value assigned from script puts the caret at its end at once",
  async () => {
    await reset();
    expect(await read(`(${TA}.value = "я🙂z", ${CARET})`)).toBe(`4,4,${FORWARD}`);
    expect(await read(`(${TA}.selectionStart = 1, ${CARET})`)).toBe(`1,4,${FORWARD}`);
    expect(await read(`(${TA}.selectionEnd = 3, ${TA}.selectionDirection = "backward", ${CARET})`)).toBe(
      "1,3,backward",
    );
  },
  T,
);

async function untilLive(selector: string, want: string): Promise<string> {
  const deadline = Date.now() + 3_000;
  for (;;) {
    const got = await app.locator(selector).textContent();
    if (got === want || Date.now() > deadline) return got;
    await Bun.sleep(50);
  }
}

test(
  "assigning a value the user typed on the way takes effect, caret at its end",
  async () => {
    const PLAIN = "document.querySelector('#plain')";
    const PLAIN_CARET = `(() => { const t = ${PLAIN}; return [t.selectionStart, t.selectionEnd].join(","); })()`;
    await app.locator("#plain").type("abcde");
    expect(await untilLive("#plain", "abcde")).toBe("abcde");
    await app.eval(`(${PLAIN}.value = "ab", 0)`);
    expect(await untilLive("#plain", "ab")).toBe("ab");
    expect(await until(PLAIN_CARET, "2,2")).toBe("2,2");
    await app.locator("#plain").type("X");
    expect(await untilLive("#plain", "abX")).toBe("abX");
  },
  T,
);

test(
  "fast typing into a controlled field keeps every character and the caret at the end",
  async () => {
    await reset();
    const typed = "the quick brown fox jumps over the lazy dog";
    await app.locator("#ta").type(typed);
    expect(await untilLive("#ta", typed)).toBe(typed);
    expect(await until("__SK_TEST_CARET__.text.value", typed)).toBe(typed);
    expect(await until(CARET, `${typed.length},${typed.length},${FORWARD}`)).toBe(
      `${typed.length},${typed.length},${FORWARD}`,
    );
    await app.locator("#ta").press("ArrowLeft");
    await app.locator("#ta").type("!");
    const edited = typed.slice(0, -1) + "!g";
    expect(await untilLive("#ta", edited)).toBe(edited);
    expect(await until(CARET, `${typed.length},${typed.length},${FORWARD}`)).toBe(
      `${typed.length},${typed.length},${FORWARD}`,
    );
  },
  T,
);

test(
  "setRangeText replaces a range, places the selection by mode, and the next keystroke lands there",
  async () => {
    await reset();
    await app.eval(`(${TA}.value = "hello world", ${TA}.setSelectionRange(6, 11), 0)`);
    await logged();
    expect(await read(`(${TA}.setRangeText("there"), ${VALUE} + "|" + ${CARET})`)).toBe(
      `"hello there"|6,11,${FORWARD}`,
    );
    expect(await read(`(${TA}.setRangeText("X", 0, 5, "end"), ${VALUE} + "|" + ${CARET})`)).toBe(
      `"X there"|1,1,${FORWARD}`,
    );
    expect(await logged()).toContain("select");
    await app.locator("#ta").type("Y");
    expect(await until(VALUE, JSON.stringify("XY there"))).toBe(JSON.stringify("XY there"));
    expect(
      await read(
        `(() => { try { ${TA}.setRangeText("z", 3, 1); return "no throw"; } catch (e) { return e.name; } })()`,
      ),
    ).toBe("IndexSizeError");
    const num = "document.querySelector('#num')";
    expect(await read(`String(${num}.selectionStart)`)).toBe("null");
    expect(
      await read(`(() => { try { ${num}.setRangeText("1"); return "no throw"; } catch (e) { return e.name; } })()`),
    ).toBe("InvalidStateError");
  },
  T,
);

test(
  "Shift+Enter inserts a line break in a textarea, as Enter does",
  async () => {
    await reset();
    await app.locator("#ta").type("a");
    await app.locator("#ta").press("Shift+Enter");
    await app.locator("#ta").type("b");
    expect(await until(VALUE, JSON.stringify("a\nb"))).toBe(JSON.stringify("a\nb"));
    expect((await logged()).filter((l) => l.startsWith("input:"))).toEqual([
      "input:insertText@1,1",
      "input:insertLineBreak@2,2",
      "input:insertText@3,3",
    ]);
  },
  T,
);

test(
  "typing is one undo step until the caret moves",
  async () => {
    await reset();
    await app.locator("#ta").type("abc");
    await app.locator("#ta").press(UNDO);
    expect(await until(VALUE, '""')).toBe('""');
    await app.locator("#ta").type("abc");
    await app.locator("#ta").press("ArrowLeft");
    await app.locator("#ta").type("d");
    expect(await until(VALUE, '"abdc"')).toBe('"abdc"');
    await app.locator("#ta").press(UNDO);
    expect(await until(VALUE, '"abc"')).toBe('"abc"');
    await app.locator("#ta").press(UNDO);
    expect(await until(VALUE, '""')).toBe('""');
  },
  T,
);

test(
  "maxlength caps typing and an IME commit, not a value set from script",
  async () => {
    const SHORT = "JSON.stringify(document.querySelector('#short').value)";
    await app.locator("#short").type("abcdefg");
    expect(await until(SHORT, '"abcde"')).toBe('"abcde"');
    await app.send("ime", { selector: "#short", text: "xyz", commit: true });
    await Bun.sleep(150);
    expect(await read(SHORT)).toBe('"abcde"');
    await app.eval("(document.querySelector('#short').value = '12', 0)");
    await app.send("ime", { selector: "#short", text: "🙂🙂", commit: true });
    expect(await until(SHORT, JSON.stringify("12🙂"))).toBe(JSON.stringify("12🙂"));
    expect(await read("(document.querySelector('#short').value = '123456789', " + SHORT + ")")).toBe('"123456789"');
  },
  T,
);
