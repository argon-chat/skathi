// <input type=file>, the File System Access pickers, FileReader and
// <a download>, through a real host (m25 desktop track C,
// research/reports/m25/desktop-c-impl.md, docs/DESKTOP.md "Pickers and files").
//
//   bun test packages/vn-test/specs/files.spec.ts
//
// vn-test launches every host with VN_DIALOGS=scripted. Every dialog here runs
// on its own dialog thread as a real one would, owned and queued the same way,
// and waits for `app.answerDialog()` instead of a person. So this spec opens no
// OS dialog on the machine it runs on. What it pins is everything on both sides
// of that seam:
// - which gestures open a picker, and that none opens without one;
// - what the dialog is asked for (kind, multiple, filters, suggested name);
// - that the answer becomes Files and handles the page can read and write;
// - that a cancel is a `cancel` event;
// - that a download writes the blob's bytes where the user "saved" it;
// - that no path reaches JS.
// The files are in a temp folder of this run, removed afterwards.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, launch, VnCommandError } from "../src/index";

const T = 30_000;
const app = await launch({
  app: "packages/vn-test/fixtures/files/src/main.ts",
  window: [480, 260],
  label: "files",
});

const scratch = join(tmpdir(), `vn-test-files-${process.pid}`);
rmSync(scratch, { recursive: true, force: true });
mkdirSync(join(scratch, "folder", "sub"), { recursive: true });
const one = join(scratch, "one.txt");
const two = join(scratch, "two.txt");
const edit = join(scratch, "edit.txt");
writeFileSync(one, "first file\n");
writeFileSync(two, "второй\n");
writeFileSync(edit, "original");
writeFileSync(join(scratch, "folder", "a.txt"), "a");
writeFileSync(join(scratch, "folder", "b.bin"), new Uint8Array([1, 2, 3]));
const saved = join(scratch, "saved.csv");
afterAll(async () => {
  await app.close();
  rmSync(scratch, { recursive: true, force: true });
});

/** Polls until `pred` holds or the deadline passes; returns the last value. */
async function until<V>(read: () => Promise<V>, pred: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  for (;;) {
    const v = await read();
    if (pred(v) || Date.now() > deadline) return v;
    await new Promise((r) => setTimeout(r, 50));
  }
}

type State = { events: string[]; results: Record<string, any> };
const state = () => app.eval("JSON.stringify(__VN_TEST_FILES__)").then((s) => JSON.parse(s) as State);
const result = (key: string) => until(state, (s) => s.results[key] !== undefined).then((s) => s.results[key]);

describe("file dialogs on the host (VN_DIALOGS=scripted)", () => {
  test("without a gesture every picker rejects with SecurityError, and no dialog is made", async () => {
    await expect(app.locator("#status")).toHaveText("ready");
    // First, before any click has given the page an activation.
    await app.eval(`__VN_TEST_FILES__.run("nogesture")`);
    bunExpect(await result("nogesture")).toBe("SecurityError|SecurityError|SecurityError");
    bunExpect(await app.dialogs()).toEqual([]);
  }, T);

  test("a real click on <input type=file> asks for files; the answer is a FileList, then input and change", async () => {
    await app.locator("#pick").click({ real: true });
    const d = await app.waitForDialog();
    bunExpect(d.what).toBe("picker");
    bunExpect(d.kind).toBe("open");
    bunExpect(d.multiple).toBe(true);
    bunExpect(d.filters.some((f) => f.spec.includes("*.txt"))).toBe(true);

    // A path the OS dialog would not accept is refused, and the dialog waits.
    await bunExpect(app.answerDialog(d.id, join(scratch, "missing.txt"))).rejects.toBeInstanceOf(VnCommandError);
    bunExpect((await app.dialogs()).map((x) => x.id)).toEqual([d.id]);

    await app.answerDialog(d.id, [one, two]);
    const change = await result("change");
    bunExpect(change.count).toBe(2);
    bunExpect(change.value).toBe("C:\\fakepath\\one.txt");
    bunExpect(change.files.map((f: any) => [f.name, f.reader, f.text])).toEqual([
      ["one.txt", "first file\n", "first file\n"],
      ["two.txt", "второй\n", "второй\n"],
    ]);
    bunExpect(change.files[0].type).toBe("text/plain");
    bunExpect((await state()).events).toEqual(["input", "change"]);
  }, T);

  test("a <label for> opens the same picker; a cancel is a `cancel` event", async () => {
    await app.locator("#label").click();
    const d = await app.waitForDialog();
    bunExpect(d.kind).toBe("open");
    await app.cancelDialog(d.id);
    const s = await until(state, (s) => s.events.length > 2);
    bunExpect(s.events).toEqual(["input", "change", "cancel"]);
    // A dismissal keeps what was chosen before, as Chromium does.
    bunExpect(await app.eval(`document.querySelector("#pick").files.length`)).toBe(2);
  }, T);

  test("showOpenFilePicker: a handle reads and writes the picked file; a structuredClone is the same entry", async () => {
    await app.locator("#open").click();
    const d = await app.waitForDialog();
    bunExpect(d.kind).toBe("open");
    bunExpect(d.multiple).toBe(false);
    await app.answerDialog(d.id, edit);
    const open = await result("open");
    bunExpect(typeof open).toBe("object");
    bunExpect(open).toMatchObject({
      kind: "file",
      name: "edit.txt",
      permission: "granted",
      before: "original",
      after: "changed",
      cloneIsHandle: true,
      cloneSame: true,
      clonePermission: "granted",
      cloneText: "changed",
      keys: [],
      json: "{}",
    });
    bunExpect(readFileSync(edit, "utf8")).toBe("changed");
  }, T);

  test("showDirectoryPicker: the folder's entries, by name and kind", async () => {
    await app.locator("#dir").click();
    const d = await app.waitForDialog();
    bunExpect(d.kind).toBe("folder");
    await app.answerDialog(d.id, join(scratch, "folder"));
    bunExpect(await result("dir")).toEqual({
      kind: "directory",
      name: "folder",
      entries: ["directory:sub", "file:a.txt", "file:b.bin"],
    });
  }, T);

  test("<a download> on a blob: URL is a Save As, and the bytes land where the user saved them", async () => {
    await app.locator("#dl").click();
    const d = await app.waitForDialog();
    bunExpect(d.what).toBe("download");
    bunExpect(d.kind).toBe("save");
    bunExpect(d.suggestedName).toBe("report.csv");
    await app.answerDialog(d.id, saved);
    await until(async () => existsSync(saved) && readFileSync(saved, "utf8"), (v) => v === "a,b\n1,2\n");
    bunExpect(readFileSync(saved, "utf8")).toBe("a,b\n1,2\n");
    const { entries } = await app.logs(0);
    // A compat advisory is a `warn` line with the "[vue-native compat]" prefix.
    bunExpect(entries.filter((e) => e.text.includes("<a download> (D-F6)")).length).toBe(1);
  }, T);

  test("no absolute path reached JS, and nothing broken was logged", async () => {
    const all = await app.eval("JSON.stringify(__VN_TEST_FILES__)");
    bunExpect(all.toLowerCase()).not.toContain(scratch.toLowerCase().replaceAll("\\", "\\\\"));
    bunExpect(all.toLowerCase()).not.toContain(scratch.toLowerCase());
    bunExpect(await app.dialogs()).toEqual([]);
    await app.expectNoErrors();
  }, T);
});
