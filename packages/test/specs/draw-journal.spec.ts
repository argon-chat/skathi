import { expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { create } from "../../cli/cli/create";
import { appDataDir, launch, REPO, WORK_DIR } from "../src/index";

test("the native drawing journal retries conflicts, restores drafts and replays sketchbook commands", async () => {
  mkdirSync(WORK_DIR, { recursive: true });
  const dir = mkdtempSync(join(WORK_DIR, "draw-journal-"));
  create(dir, { name: "Journal", template: "draw" });
  const entry = join(dir, "src", "main.ts");
  const probe = readFileSync(join(REPO, "templates/draw/test/journalProbe.ts"), "utf8").replaceAll('"../src/', '"./');
  writeFileSync(entry, probe + "\nmountJournalProbe();\n");
  const restart = mkdtempSync(join(WORK_DIR, "draw-journal-check-"));
  create(restart, { name: "Journal", template: "draw" });
  const profile = basename(dir);
  const host = (entry: string) =>
    launch({ app: relative(REPO, entry), label: "draw-journal", env: { SK_APP_NAME: profile } });
  let app = await host(entry);
  const exited = async (pid: number) => {
    const killed = Date.now();
    for (;;) {
      try {
        process.kill(pid, 0);
      } catch {
        return;
      }
      if (Date.now() - killed > 8000) throw new Error("The previous host still holds its IndexedDB lock.");
      await Bun.sleep(40);
    }
  };
  const kill = async () => {
    const pid = app.pid;
    await app.close();
    await exited(pid);
  };
  try {
    let result: unknown;
    const start = Date.now();
    while (!result && Date.now() - start < 30000) {
      result = await app.eval("globalThis.journalResult");
      if (!result) await Bun.sleep(80);
    }
    expect(result).toBe("ok");
    const count = Number(await app.eval("globalThis.journalCount"));
    expect(count).toBeGreaterThan(0);
    // The document and its outbox committed while the recorder's edit batch still awaited its timer.
    await kill();
    app = await host(join(restart, "src/main.ts"));
    await app.locator("#draw[data-recording=recording][data-ready=true]").waitFor();
    expect(Number(await app.eval('document.querySelector("#draw").getAttribute("data-strokes")'))).toBe(count);
    await app.locator("#recording").click();
    await app.locator("#record-stop").click();
    await app.locator("#draw[data-recording=stopped]").waitFor();
    if (!(await app.eval('!!document.querySelector("#record-preview")'))) await app.locator("#recording").click();
    await app.locator("#record-preview").click();
    await app.locator("#player-transport").waitFor();
    await app.eval(
      'window.__draw.seek(Number(document.querySelector("#player-transport").getAttribute("data-duration")))',
    );
    await app.locator(`#draw[data-strokes="${count}"]`).waitFor();
    await app.expectNoErrors();
    if (process.platform !== "win32") return;
    await kill();
    const closeEntry = join(dir, "src", "closeProbe.ts");
    writeFileSync(closeEntry, probe + "\nmountCloseProbe();\n");
    app = await host(closeEntry);
    await app.locator("#book-state[data-ready=true]").waitFor();
    const hwnd = await app.hwnd();
    const user32 = dlopen("user32.dll", {
      PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
    });
    try {
      user32.symbols.PostMessageW(hwnd, 0x0010, 0n, 0n);
      const requested = Date.now();
      let warning: unknown;
      while (!warning && Date.now() - requested < 7000) {
        warning = await app.eval("globalThis.journalCloseWarning");
        if (!warning) await Bun.sleep(90);
      }
      expect(String(warning)).toContain("Close again");
      expect(Date.now() - requested).toBeGreaterThanOrEqual(2900);
      user32.symbols.PostMessageW(hwnd, 0x0010, 0n, 0n);
      await exited(app.pid);
    } finally {
      user32.close();
    }
  } finally {
    await kill();
    rmSync(dir, { recursive: true, force: true });
    rmSync(restart, { recursive: true, force: true });
    const storage = appDataDir(profile);
    if (basename(storage) === profile) {
      const deadline = Date.now() + 5000;
      for (;;) {
        try {
          rmSync(storage, { recursive: true, force: true });
          break;
        } catch (e) {
          if (Date.now() >= deadline) throw e;
          await Bun.sleep(100);
        }
      }
    }
  }
}, 60000);
