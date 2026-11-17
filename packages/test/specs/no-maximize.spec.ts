// `window.maximizable: false`: no maximize box, and no user path (caption button, el.click(), the
// system menu or a caption double-click) maximizes the window.

import { afterAll, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/no-maximize/src/main.ts",
  window: [480, 320],
  label: "no-maximize",
});
const user32 =
  process.platform === "win32"
    ? dlopen("user32.dll", {
        GetWindowLongPtrW: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i64 },
        IsZoomed: { args: [FFIType.u64], returns: FFIType.i32 },
        PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
      })
    : null;
afterAll(async () => {
  user32?.close();
  await app.close();
});

const T = 30_000;
const GWL_STYLE = -16;
const WS_MAXIMIZEBOX = 0x10000n;
const WM_SYSCOMMAND = 0x0112;
const SC_MAXIMIZE = 0xf030n;

const hwnd = await app.hwnd();
const zoomed = async (): Promise<boolean> =>
  user32 ? user32.symbols.IsZoomed(hwnd) !== 0 : await app.eval("__SK_WINDOW__.appWindow.isMaximized()");

test(
  "no maximize box, and maximize requests from the user are ignored",
  async () => {
    if (user32) expect(BigInt(user32.symbols.GetWindowLongPtrW(hwnd, GWL_STYLE)) & WS_MAXIMIZEBOX).toBe(0n);

    expect(await app.eval(`document.getElementById("max").click(); "ok"`)).toBe("ok");
    user32?.symbols.PostMessageW(hwnd, WM_SYSCOMMAND, SC_MAXIMIZE, 0n);
    await Bun.sleep(1_000);
    expect(await zoomed()).toBe(false);

    await app.eval("__sk_window_verb('maximize'); 'ok'");
    const deadline = Date.now() + 10_000;
    while (!(await zoomed()) && Date.now() < deadline) await Bun.sleep(50);
    expect(await zoomed()).toBe(true);
  },
  T,
);
