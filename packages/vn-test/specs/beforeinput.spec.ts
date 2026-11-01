// `beforeinput`, `input` and the composition events of a text field, on the
// real host (m26 fixes, docs/RUNTIME.md §5.25, crates/vn-elements
// text_input/state.rs).
//
//   bun test packages/vn-test/specs/beforeinput.spec.ts
//
// Keys go through devtools `type` / `key`, which is gpui's own keystroke
// dispatch: the field's keydown interceptor, its key bindings and its input
// handler, as for a physical key. The IME goes through devtools `ime`, which
// makes the two calls the platform's IME makes on the field's input handler.
// The JS half (what the payload becomes) is test/runtime-events-m26.test.ts.
//
// One character per `type`, and no `keyup` in the log: devtools hands gpui a
// whole command in one update, and the widget's `input` goes out at the end
// of the update it edited in. A physical key is separate platform messages
// (keydown, the character, keyup), each its own update, so there `input` comes
// before `keyup` and before the next key's `keydown`.
//
// One test pastes and cuts, which reads and writes the system clipboard: it
// saves the text that was there first and puts it back afterwards, as
// selection.spec.ts does.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/beforeinput/src/main.ts",
  window: [420, 220],
  label: "beforeinput",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The events recorded since the last call. */
let seen = 0;
async function fresh(): Promise<string[]> {
  await sleep(150);
  const all = JSON.parse((await app.eval("JSON.stringify(__VN_TEST_BI__.events)")) as string) as string[];
  const out = all.slice(seen);
  seen = all.length;
  return out;
}

const cancel = (type: string | null, data?: string) =>
  app.eval(`(Object.assign(__VN_TEST_BI__.cancel, { type: ${JSON.stringify(type)} }), ` +
    `${data === undefined ? "delete __VN_TEST_BI__.cancel.data" : `__VN_TEST_BI__.cancel.data = ${JSON.stringify(data)}`}, 0)`);
const value = async (sel: string) => String(await app.eval(`document.querySelector(${JSON.stringify(sel)}).value`));
const ime = (sel: string, text: string, commit = false) => app.send("ime", { selector: sel, text, ...(commit ? { commit } : {}) });

/** `beforeinput` / `input` as the fixture logs them. */
const bi = (type: string, data: string | null, v: string, composing = false) =>
  `beforeinput@ta|${type} data=${JSON.stringify(data)} composing=${composing} cancelable=${!composing} v=${JSON.stringify(v)}`;
const inp = (type: string, data: string | null, v: string, composing = false, where = "ta") =>
  `input@${where}|${type} data=${JSON.stringify(data)} composing=${composing} cancelable=false v=${JSON.stringify(v)}`;

function powershell(cmd: string): string {
  const r = spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", cmd], { encoding: "utf8" });
  return r.stdout ?? "";
}

describe("beforeinput from the keyboard", () => {
  test("typing: keydown, beforeinput, the edit, input — each with insertText and the character", async () => {
    await expect(app.locator("#ta")).toHaveCount(1);
    await fresh();
    await app.locator("#ta").type("a");
    await app.locator("#ta").type("b");
    bunExpect(await fresh()).toEqual([
      "keydown@ta|a",
      bi("insertText", "a", ""),
      inp("insertText", "a", "a"),
      "keydown@ta|b",
      bi("insertText", "b", "a"),
      inp("insertText", "b", "ab"),
    ]);
  }, T);

  test("a cancelled beforeinput makes no edit and no input (useEmojiSuggest's swallowed space)", async () => {
    await cancel("insertText", " ");
    await app.locator("#ta").type(" ");
    await app.locator("#ta").type("c");
    bunExpect(await fresh()).toEqual([
      "keydown@ta| ",
      bi("insertText", " ", "ab"),
      "keydown@ta|c",
      bi("insertText", "c", "ab"),
      inp("insertText", "c", "abc"),
    ]);
    bunExpect(await value("#ta")).toBe("abc");
    await cancel(null);
  }, T);

  test("Enter in a textarea is insertLineBreak, with no data", async () => {
    await app.locator("#ta").press("Enter");
    bunExpect(await fresh()).toEqual(["keydown@ta|Enter", bi("insertLineBreak", null, "abc"), inp("insertLineBreak", null, "abc\n")]);
  }, T);

  test("Backspace, Ctrl+Backspace and Delete: the deletion types, no data; nothing to delete is no event", async () => {
    await app.locator("#ta").press("Backspace");
    await app.locator("#ta").press("Ctrl+Backspace");
    await app.locator("#ta").press("Delete");
    bunExpect(await fresh()).toEqual([
      "keydown@ta|Backspace",
      bi("deleteContentBackward", null, "abc\n"),
      inp("deleteContentBackward", null, "abc"),
      "keydown@ta|Backspace",
      bi("deleteWordBackward", null, "abc"),
      inp("deleteWordBackward", null, ""),
      // At the end of an empty value: nothing to delete, so no beforeinput.
      "keydown@ta|Delete",
    ]);
  }, T);

  test("a cancelled deletion keeps the text", async () => {
    await app.locator("#ta").type("x");
    await app.locator("#ta").type("y");
    await fresh();
    await cancel("deleteContentBackward");
    await app.locator("#ta").press("Backspace");
    bunExpect(await fresh()).toEqual(["keydown@ta|Backspace", bi("deleteContentBackward", null, "xy")]);
    bunExpect(await value("#ta")).toBe("xy");
    await cancel(null);
  }, T);

  test("Ctrl+Z and Ctrl+Y: historyUndo and historyRedo", async () => {
    await app.locator("#ta").press("Ctrl+Z");
    await app.locator("#ta").press("Ctrl+Y");
    bunExpect(await fresh()).toEqual([
      "keydown@ta|z",
      bi("historyUndo", null, "xy"),
      inp("historyUndo", null, "x"),
      "keydown@ta|y",
      bi("historyRedo", null, "x"),
      inp("historyRedo", null, "xy"),
    ]);
  }, T);

  test("paste and cut: after the ClipboardEvent, insertFromPaste with the text, deleteByCut", async () => {
    const saved = powershell("Get-Clipboard -Raw");
    try {
      powershell("Set-Clipboard -Value 'pasted'");
      await app.locator("#ta").press("Ctrl+V");
      await app.locator("#ta").press("Ctrl+A");
      await cancel("deleteByCut");
      await app.locator("#ta").press("Ctrl+X");
      await cancel(null);
      await app.locator("#ta").press("Ctrl+X");
      bunExpect(await fresh()).toEqual([
        "keydown@ta|v",
        "paste@ta",
        bi("insertFromPaste", "pasted", "xy"),
        inp("insertFromPaste", "pasted", "xypasted"),
        "keydown@ta|a",
        // Cancelled: the text stays (Chromium has already copied it).
        "keydown@ta|x",
        "cut@ta",
        bi("deleteByCut", null, "xypasted"),
        "keydown@ta|x",
        "cut@ta",
        bi("deleteByCut", null, "xypasted"),
        inp("deleteByCut", null, ""),
      ]);
      bunExpect(powershell("Get-Clipboard -Raw").replace(/\r?\n$/, "")).toBe("xypasted");
    } finally {
      // Put the owner's text back (Set-Clipboard with nothing clears it).
      const escaped = saved.replace(/\r?\n$/, "").replace(/'/g, "''");
      powershell(saved ? `Set-Clipboard -Value '${escaped}'` : "Set-Clipboard -Value $null");
    }
  }, T);
});

describe("the IME", () => {
  test("a composition: start, then beforeinput / compositionupdate / input per pre-edit, and the end after the commit's input", async () => {
    await fresh();
    await ime("#ta", "k");
    await ime("#ta", "か");
    await ime("#ta", "火", true);
    bunExpect(await fresh()).toEqual([
      'compositionstart@ta|data="" v=""',
      bi("insertCompositionText", "k", "", true),
      'compositionupdate@ta|data="k" v=""',
      inp("insertCompositionText", "k", "k", true),
      bi("insertCompositionText", "か", "k", true),
      'compositionupdate@ta|data="か" v="k"',
      inp("insertCompositionText", "か", "か", true),
      bi("insertCompositionText", "火", "か", true),
      'compositionupdate@ta|data="火" v="か"',
      inp("insertCompositionText", "火", "火", true),
      'compositionend@ta|data="火" v="火"',
    ]);
  }, T);

  test("its beforeinput cannot be cancelled", async () => {
    await cancel("insertCompositionText");
    await ime("#ta", "s");
    await ime("#ta", "す", true);
    const got = await fresh();
    bunExpect(got.filter((l) => l.startsWith("input@"))).toEqual([
      inp("insertCompositionText", "s", "火s", true),
      inp("insertCompositionText", "す", "火す", true),
    ]);
    await cancel(null);
  }, T);

  test("an emptied pre-edit ends the composition with no text", async () => {
    await ime("#ta", "n");
    await ime("#ta", "");
    bunExpect(await fresh()).toEqual([
      'compositionstart@ta|data="" v="火す"',
      bi("insertCompositionText", "n", "火す", true),
      'compositionupdate@ta|data="n" v="火す"',
      inp("insertCompositionText", "n", "火すn", true),
      bi("insertCompositionText", "", "火すn", true),
      'compositionupdate@ta|data="" v="火すn"',
      inp("insertCompositionText", "", "火す", true),
      'compositionend@ta|data="" v="火す"',
    ]);
  }, T);

  test("a value assigned in the middle of a composition ends it", async () => {
    await ime("#ta", "m");
    await fresh();
    await app.eval('__VN_TEST_BI__.assign("#ta", "reset")');
    let got: string[] = [];
    for (let i = 0; i < 10 && got.length === 0; i++) got = await fresh();
    bunExpect(got).toEqual(['compositionend@ta|data="m" v="reset"']);
  }, T);

  test("v-model leaves the model alone until the composition ends, as runtime-dom does", async () => {
    await app.locator("#vm").type("a"); // one character: see the header
    await expect(app.locator("#model")).toHaveText("model=a");
    await fresh();
    await ime("#vm", "k");
    await ime("#vm", "か");
    await sleep(150);
    await expect(app.locator("#model")).toHaveText("model=a");
    await ime("#vm", "火", true);
    await expect(app.locator("#model")).toHaveText("model=a火");
    bunExpect(await fresh()).toEqual([
      'compositionstart@vm|data="" v="a"',
      inp("insertCompositionText", "k", "ak", true, "vm"),
      inp("insertCompositionText", "か", "aか", true, "vm"),
      inp("insertCompositionText", "火", "a火", true, "vm"),
      'compositionend@vm|data="火" v="a火"',
      // runtime-dom's re-dispatch, which assigns the model.
      'input@vm|undefined data=undefined composing=undefined cancelable=false v="a火"',
    ]);
  }, T);
});
