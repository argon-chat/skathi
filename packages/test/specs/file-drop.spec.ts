import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect as expectLocator, launch, WORK_DIR } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/file-drop/src/main.ts",
  label: "file-drop",
  window: [430, 320],
});
const dir = mkdtempSync(join(WORK_DIR, "drop-source-"));
const file = join(dir, "video.txt");
const folder = join(dir, "clips");
writeFileSync(file, "hello");
mkdirSync(folder);
for (let i = 0; i < 205; ++i) writeFileSync(join(folder, i + ".txt"), String(i));
afterAll(async () => {
  await app.close();
  rmSync(dir, { recursive: true, force: true });
});
const read = async (expression: string) => JSON.parse(String(await app.eval("JSON.stringify(" + expression + ")")));
const drag = (kind: string, selector: string, args: Record<string, unknown> = {}) =>
  app.send("drag-files", { selector, kind, ...args });

test("an external drag protects files until drop and delivers the receiving element's decision", async () => {
  await app.locator("#child").waitFor();
  const offer = { paths: [file, folder], text: "plain", ctrl: true };
  expect((await drag("enter", "#child", offer)).dropEffect).toBe("copy");
  const log = await read("__dropTest.log");
  expect(log[0]).toEqual([
    "dragenter",
    "child",
    null,
    0,
    ["text/plain", "Files"],
    [
      ["file", "text/plain", null],
      ["file", "", null],
      ["string", "text/plain", null],
    ],
    "",
    true,
  ]);
  expect(log[1][0]).toBe("dragover");
  await drag("over", "#zone", { ctrl: true });
  expect(await read("__dropTest.log.slice(-3).map(e => e.slice(0,3))")).toEqual([
    ["dragenter", "zone", "child"],
    ["dragleave", "child", "zone"],
    ["dragover", "zone", null],
  ]);
  expect((await drag("drop", "#zone")).dropEffect).toBe("copy");
  expect(await read("__dropTest.drop")).toEqual([
    ["video.txt", 5, "text/plain", ""],
    ["clips", 0, "", ""],
  ]);
  expect(await read("__dropTest.handles.map(h => h.kind)")).toEqual(["file", "directory"]);
  expect(await read("__dropTest.log.at(-1).slice(0,5)")).toEqual(["drop", "zone", null, 2, ["text/plain", "Files"]]);
  expect(await read("__dropTest.log.at(-1)[6]")).toBe("plain");
  await app.eval("__dropTest.allow = false; __dropTest.log.length = 0");
  expect((await drag("enter", "#zone", offer)).dropEffect).toBe("none");
  expect((await drag("drop", "#zone")).dropEffect).toBe("none");
  expect(await read("__dropTest.log.map(e => e[0])")).toEqual(["dragenter", "dragover", "dragleave"]);
}, 30_000);

test("OS entries enumerate in batches, keep real paths and grant writable handles", async () => {
  await app.eval("__dropTest.allow=true");
  await drag("drop", "#child", { paths: [file, folder] });
  await app.eval(`globalThis.entryCheck=null; (async()=>{
    const t=__dropTest, dir=t.entries[1], file=t.entries[0];
    const invoke=(obj,key,...args)=>new Promise((done,fail)=>obj[key](...args,done,fail));
    const r=dir.createReader(), sizes=[], paths=[];
    for (;;) { let batch=await invoke(r,"readEntries"); sizes.push(batch.length); if (!batch.length) break;
      paths.push(...batch.map(e=>e.fullPath)); }
    const child=await invoke(dir,"getFile","42.txt",{});
    const childFile=await invoke(child,"file");
    const parent=await invoke(child,"getParent");
    const handle=t.handles[0]; const w=await handle.createWritable(); await w.write("saved"); await w.close();
    entryCheck={sizes, total:paths.length, kind:dir instanceof FileSystemDirectoryEntry && file instanceof FileSystemFileEntry,
      identity:dir.filesystem===file.filesystem && child.filesystem===dir.filesystem, root:dir.filesystem.root.fullPath,
      child:child.fullPath, text:await childFile.text(), parent:parent.fullPath,
      paths:[t.path(t.files[0]),t.path(dir),t.path(handle),t.path(childFile),t.path(new File(["x"],"x"))],
      protected:t.protectedEntry,expired:t.retained.webkitGetAsEntry()===null};
  })().catch(e=>{entryCheck={error:String(e),stack:e.stack}});true`);
  let result;
  for (let i = 0; i < 100; i++) {
    result = await read("entryCheck");
    if (result) break;
    await Bun.sleep(60);
  }
  expect(result).toEqual({
    sizes: [100, 100, 5, 0],
    total: 205,
    kind: true,
    identity: true,
    root: "/",
    child: "/clips/42.txt",
    text: "42",
    parent: "/clips",
    paths: [file, folder, file, join(folder, "42.txt"), null],
    protected: true,
    expired: true,
  });
  expect(readFileSync(file, "utf8")).toBe("saved");
}, 30_000);

test("VueUse useDropZone accepts its MIME type and clears the hover counter", async () => {
  await drag("enter", "#vueuse", { paths: [file] });
  await expectLocator(app.locator("#vueuse")).toHaveText("over");
  await drag("leave", "#vueuse");
  await expectLocator(app.locator("#vueuse")).toHaveText("idle");
  await drag("drop", "#vueuse", { paths: [folder] });
  await expectLocator(app.locator("#vueuse")).toHaveText("idle");
  await drag("drop", "#vueuse", { paths: [file] });
  await expectLocator(app.locator("#vueuse")).toHaveText("video.txt");
}, 30_000);

test("Svelte receives drops in the correct native window", async () => {
  const svelte = await launch({
    app: "packages/test/fixtures/svelte-file-drop/src/main.ts",
    label: "svelte-file-drop",
    window: [360, 240],
  });
  try {
    await svelte.locator("#open").click();
    await svelte.locator("#svelte-child").waitFor();
    const wins = (await svelte.send("windows")).windows as { id: number; title: string }[];
    const child = wins.find((w) => w.title === "File drop");
    expect(child).toBeDefined();
    expect(
      (await svelte.send("drag-files", { kind: "drop", window: child!.id, selector: "#svelte-child", paths: [file] }))
        .dropEffect,
    ).toBe("copy");
    await expectLocator(svelte.locator("#svelte-child")).toHaveText("video.txt");
    await expectLocator(svelte.locator("#svelte-main")).toHaveText("idle");
    await svelte.send("drag-files", { kind: "drop", selector: "#svelte-main", paths: [folder] });
    await expectLocator(svelte.locator("#svelte-main")).toHaveText("clips");

    async function startDocumentDrag() {
      await svelte.send("drag-document", { kind: "arm" });
      const down = await svelte.send("mouse", { kind: "down", selector: "#drag-source" });
      await svelte.send("mouse", { kind: "move", x: Number(down.x) + 9, y: Number(down.y), held: true });
    }
    const target = { selector: "#svelte-child", window: child!.id };
    await startDocumentDrag();
    await svelte.send("drag-document", { kind: "enter", ...target });
    expect((await svelte.send("drag-document", { kind: "over", ...target })).dropEffect).toBe("move");
    await expectLocator(svelte.locator("#drag-preview")).toHaveText("0//copyMove");
    expect((await svelte.send("drag-document", { kind: "drop", ...target })).dropEffect).toBe("move");
    await expectLocator(svelte.locator("#svelte-child")).toHaveText("private/public/true/drawing.png");
    await expectLocator(svelte.locator("#svelte-main")).toHaveText("move");

    await startDocumentDrag();
    await svelte.send("drag-document", { kind: "cancel" });
    await expectLocator(svelte.locator("#svelte-main")).toHaveText("none");
  } finally {
    await svelte.close();
  }
}, 30_000);

test("dropping on file inputs honors multiple and fires input before change", async () => {
  await app.eval("__dropTest.log.length = 0");
  expect((await drag("drop", "#single", { paths: [file, folder] })).dropEffect).toBe("copy");
  expect(await read("__dropTest.log")).toEqual(["input", "change"]);
  expect(await read("Array.from(document.querySelector('#single').files,f=>f.name)")).toEqual(["video.txt"]);
  expect((await drag("drop", "#multi", { paths: [file, folder] })).dropEffect).toBe("copy");
  expect(await read("Array.from(document.querySelector('#multi').files,f=>f.name)")).toEqual(["video.txt", "clips"]);
}, 30_000);
