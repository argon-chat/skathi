import { expect, test } from "bun:test";
import { dlopen, FFIType as T, ptr } from "bun:ffi";
import { artifactPath, launch, WINDOWS } from "../src/index";

if (WINDOWS) {
  const { symbols: win } = dlopen("user32.dll", {
    GetKeyboardLayoutList: { args: [T.i32, T.ptr], returns: T.i32 },
    GetKeyboardLayout: { args: [T.u32], returns: T.u64 },
    GetWindowThreadProcessId: { args: [T.u64, T.ptr], returns: T.u32 },
    MapVirtualKeyExW: { args: [T.u32, T.u32, T.u64], returns: T.u32 },
    ToUnicodeEx: { args: [T.u32, T.u32, T.ptr, T.ptr, T.i32, T.u32, T.u64], returns: T.i32 },
    PostMessageW: { args: [T.u64, T.u32, T.u64, T.i64], returns: T.i32 },
  });
  const handles = new BigUint64Array(win.GetKeyboardLayoutList(0, null));
  const count = handles.length && win.GetKeyboardLayoutList(handles.length, ptr(handles));
  const layouts = Array.from(handles.subarray(0, count), (handle) => {
    const scan = 0x10; // DOM KeyQ
    const vk = win.MapVirtualKeyExW(scan, 3, handle);
    const state = new Uint8Array(256),
      out = new Uint16Array(16);
    const n = win.ToUnicodeEx(vk, scan, ptr(state), ptr(out), out.length, 4, handle);
    return { handle, legend: n > 0 ? String.fromCharCode(...out.subarray(0, n)).toUpperCase() : "" };
  }).filter(({ legend }) => legend);
  const first = layouts[0],
    second = layouts.find(({ legend }) => legend !== first?.legend);
  test.skipIf(!first || !second)(
    "Windows input-source changes update a mounted physical shortcut",
    async () => {
      const app = await launch({
        app: "packages/test/fixtures/context-menu/src/main.ts",
        label: "keymap-layout",
        window: [320, 180],
      });
      try {
        const shot = await app.send("screenshot", { path: artifactPath("keymap-hwnd", "png") });
        const hwnd = BigInt(shot.hwnd);
        const original = win.GetKeyboardLayout(win.GetWindowThreadProcessId(hwnd, null));
        try {
          for (const { handle, legend } of [first!, second!]) {
            expect(win.PostMessageW(hwnd, 0x0050, 0n, handle)).not.toBe(0);
            await app.locator('#layout-shortcut[aria-label="Control plus ' + legend + '"]').waitFor();
          }
        } finally {
          win.PostMessageW(hwnd, 0x0050, 0n, original);
        }
      } finally {
        await app.close();
      }
    },
    45_000,
  );
}
