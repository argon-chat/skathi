import { expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { closeSync, copyFileSync, ftruncateSync, mkdtempSync, openSync, rmSync, statSync, writeSync } from "node:fs";
import { join } from "node:path";
import { launch, REPO, WINDOWS, WORK_DIR } from "../src/index";

const enabled = WINDOWS && process.env.SK_TEST_LARGE_DROP === "1";

// The existing MP4 stays playable. A sparse, extended-size ISO BMFF free box
// puts real media and a distant valid byte inside one file larger than 4 GiB.
test.skipIf(!enabled)(
  "a dropped 4 GiB video plays, seeks and streams with bounded working set",
  async () => {
    const app = await launch({
      app: "packages/test/fixtures/file-drop/src/main.ts",
      label: "file-drop-large",
      window: [560, 390],
    });
    const scratch = mkdtempSync(join(WORK_DIR, "drop-media-"));
    const file = join(scratch, "video.mp4");
    const size = 4 * 1024 ** 3 + 16 * 1024 ** 2;
    const rss = () =>
      Number(
        execFileSync("pwsh", ["-NoProfile", "-Command", "(Get-Process -Id " + app.pid + ").WorkingSet64"], {
          encoding: "utf8",
          windowsHide: true,
        }).trim(),
      );
    try {
      copyFileSync(join(REPO, "packages/test/fixtures/paint-only/src/clip-tiny.mp4"), file);
      const original = statSync(file).size;
      execFileSync("fsutil", ["sparse", "setflag", file], { windowsHide: true });
      const fd = openSync(file, "r+");
      try {
        const header = Buffer.alloc(16);
        header.writeUInt32BE(1);
        header.write("free", 4);
        header.writeBigUInt64BE(BigInt(size - original), 8);
        writeSync(fd, header, 0, header.length, original);
        ftruncateSync(fd, size);
      } finally {
        closeSync(fd);
      }

      await app.locator("#child").waitFor();
      const initial = rss();
      await app.send("drag-files", { kind: "drop", selector: "#child", paths: [file] });
      await app.eval(`globalThis.mediaDrop = {done:false, bytes:0, slice:0, seek:0}; (async()=>{
      const state=mediaDrop, file=__dropTest.files[0], video=document.createElement('video');
      const url=URL.createObjectURL(file);
      try {
        state.slice=(await file.slice(2*1024**3,2*1024**3+1024**2).arrayBuffer()).byteLength;
        video.muted=true; video.style.cssText='width:160px;height:90px';
        document.querySelector('#outer').appendChild(video); video.src=url; await video.play();
        for (let i=0; i<150 && !(video.currentTime>0); ++i) await new Promise(r=>setTimeout(r,50));
        if (!(video.currentTime>0 && video.duration>0)) throw Error('first media frame did not play');
        video.currentTime=video.duration/2; await new Promise(r=>setTimeout(r,500));
        state.seek=video.currentTime; video.pause();
        const reader=file.stream().getReader();
        for (;;) {const chunk=await reader.read();if(chunk.done)break; state.bytes+=chunk.value.byteLength;}
      } catch(e) {state.error=String(e)} finally {video.remove();URL.revokeObjectURL(url);state.done=true}
    })();true`);
      let peak = initial;
      let state: { done: boolean; bytes: number; slice: number; seek: number; error?: string };
      const deadline = Date.now() + 180_000;
      do {
        peak = Math.max(peak, rss());
        state = JSON.parse(String(await app.eval("JSON.stringify(mediaDrop)")));
        if (state.done) break;
        if (Date.now() > deadline) throw Error("large media read timed out at " + state.bytes);
        await Bun.sleep(120);
      } while (true);
      expect(state.error).toBeUndefined();
      expect(state.slice).toBe(1024 ** 2);
      expect(state.seek).toBeGreaterThan(0.75);
      expect(state.bytes).toBe(size);
      expect(peak - initial).toBeLessThan(512 * 1024 ** 2);
      console.log(`large drop: ${state.bytes} bytes; initial ${initial}, peak ${peak}, final ${rss()} bytes RSS`);
    } finally {
      await app.close();
      rmSync(scratch, { recursive: true, force: true });
    }
  },
  200_000,
);
