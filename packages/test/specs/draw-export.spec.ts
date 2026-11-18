import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { create } from "../../cli/cli/create";
import { appDataDir, launch, REPO, WORK_DIR } from "../src/index";

test("Draw prepares the recording before Save As, preserves the destination on failure and retries", async () => {
  mkdirSync(WORK_DIR, { recursive: true });
  const dir = mkdtempSync(join(WORK_DIR, "draw-export-"));
  const profile = basename(dir);
  create(dir, { name: "Draw export", template: "draw" });
  const app = await launch({
    app: relative(REPO, join(dir, "src/main.ts")),
    env: { SK_APP_NAME: profile },
    label: "draw-export",
    window: [1000, 700],
  });
  const wait = async (check: () => Promise<unknown>) => {
    const until = Date.now() + 8000;
    while (!(await check())) {
      if (Date.now() > until) throw new Error(await app.tailLogs());
      await Bun.sleep(40);
    }
  };
  const save = () => app.locator("#record-save").click({ real: true });
  const editEndChunk = async (edit: string) => {
    await app.eval(`{
      globalThis.chunkEdited = false;
      const request = indexedDB.open("skathi-draw-events");
      request.onsuccess = () => {
        const db = request.result;
        const tx = db.transaction(["session", "chunks"], "readwrite");
        const chunks = tx.objectStore("chunks");
        const session = tx.objectStore("session").get("current");
        session.onsuccess = () => {
          const key = session.result.chunks - 1;
          const end = chunks.get(key);
          end.onsuccess = () => { globalThis.exportEnd ??= end.result; ${edit} };
        };
        tx.oncomplete = () => { db.close(); globalThis.chunkEdited = true; };
      };
    }`);
    await wait(() => app.eval("globalThis.chunkEdited"));
  };
  const target = join(dir, "drawing.skev");
  const original = Buffer.from("existing drawing");
  writeFileSync(target, original);
  try {
    await app.locator('#draw[data-ready="true"]').waitFor();
    await app.locator("#recording").click();
    await app.locator("#record-start").click();
    await app.locator('#draw[data-recording="recording"]').waitFor();
    await app.locator("#recording").click();
    await app.locator("#record-stop").click();
    await app.locator('#draw[data-recording="stopped"]').waitFor();

    // A missing chunk or a damaged final CRC must fail before the native save dialog.
    for (const [damage, message] of [
      ["chunks.delete(key);", "read all saved recording chunks"],
      ["const bad = exportEnd.slice(); bad[bad.length - 1] ^= 1; chunks.put(bad, key);", "missing or damaged"],
    ] as const) {
      await editEndChunk(damage);
      await save();
      const error = app.locator("#record-save-error");
      await error.waitFor();
      expect(await error.textContent()).toContain(message);
      expect(await app.dialogs()).toHaveLength(0);
      expect(readFileSync(target)).toEqual(original);
    }
    await editEndChunk("chunks.put(exportEnd, key);");
    await save();
    const canceled = await app.waitForDialog();
    expect(canceled.what).toBe("download");
    await app.cancelDialog(canceled.id);
    expect(readFileSync(target)).toEqual(original);

    // The name itself fits a filesystem component. Appending the swap suffix does not.
    const writeFailure = join(dir, "x".repeat(248) + ".skev");
    writeFileSync(writeFailure, original);
    await save();
    await app.answerDialog((await app.waitForDialog()).id, writeFailure);
    await wait(async () => (await app.tailLogs()).includes("nothing was saved"));
    expect(readFileSync(writeFailure)).toEqual(original);

    await save();
    await app.answerDialog((await app.waitForDialog()).id, target);
    await wait(async () => !readFileSync(target).equals(original));
    await app.locator("#record-open").click({ real: true });
    await app.answerDialog((await app.waitForDialog()).id, target);
    await app.locator('#draw[data-player="true"]').waitFor();
    expect(
      Number(await app.eval('document.querySelector("#player-transport").getAttribute("data-duration")')),
    ).toBeGreaterThan(0);
    await app.expectNoErrors();
  } finally {
    await app.close();
    rmSync(dir, { recursive: true, force: true });
    // On Windows the host releases the IndexedDB WAL after close() returns.
    const deadline = Date.now() + 5000;
    for (;;) {
      try {
        rmSync(appDataDir(profile), { recursive: true, force: true });
        break;
      } catch (error) {
        if (Date.now() >= deadline) throw error;
        await Bun.sleep(100);
      }
    }
  }
}, 60000);
