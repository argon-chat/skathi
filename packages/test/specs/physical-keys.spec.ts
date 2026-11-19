import { afterAll, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { launch } from "../src/index";

if (process.platform === "win32") {
  const win = dlopen("user32.dll", {
    PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
  });
  const app = await launch({ app: "packages/test/fixtures/keys/src/main.ts", label: "physical-keys" });
  afterAll(async () => {
    await app.close();
    win.close();
  });
  const hwnd = await app.hwnd();
  await app.eval(`globalThis.keyTrace = []; for (const name of ["keydown", "keyup"]) document.addEventListener(name, e => {
    keyTrace.push([e.type, e.key, e.code, e.location, e.repeat, e.target.id]);
  }, true);`);

  // Target this HWND. Bits 16..24 are the scan and E0 flag, 30 and 31 track previous state and release.
  const post = (vk: number, scan: number, previous: boolean, up: boolean, count = 1) => {
    const lparam = BigInt(count) | (BigInt(scan) << 16n) | (BigInt(previous) << 30n) | (BigInt(up) << 31n);
    expect(win.symbols.PostMessageW(hwnd, up ? 0x101 : 0x100, BigInt(vk), lparam)).toBe(1);
  };
  for (const target of ["inner", "field"]) {
    test(`native positions, repeats and releases on #${target}`, async () => {
      await app.eval(`document.getElementById("${target}").focus(); keyTrace.length = 0`);
      for (const [vk, scan] of [
        [0x20, 0x39],
        [0x0d, 0x1c],
        [0x0d, 0x11c],
      ] as const) {
        if (vk === 0x20) post(vk, scan, false, false, 2);
        else {
          post(vk, scan, false, false);
          post(vk, scan, true, false);
        }
        post(vk, scan, true, true);
      }
      for (let i = 0; i < 100 && Number(await app.eval("keyTrace.length")) < 9; i++) await Bun.sleep(20);
      const trace: unknown = JSON.parse(String(await app.eval("JSON.stringify(keyTrace)")));
      expect(trace).toEqual(
        [
          [" ", "Space", 0],
          ["Enter", "Enter", 0],
          ["Enter", "NumpadEnter", 3],
        ].flatMap(([key, code, location]) => [
          ["keydown", key, code, location, false, target],
          ["keydown", key, code, location, true, target],
          ["keyup", key, code, location, false, target],
        ]),
      );
    }, 30_000);
  }
}
