import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType, JSCallback, ptr, read } from "bun:ffi";
import { launch } from "../src/index";

if (process.platform !== "win32") {
  describe("overlay", () => {
    test.skip("Windows only: reads Win32 window styles, activation and screen pixels", () => {});
  });
} else {
  const user32 = dlopen("user32.dll", {
    SetProcessDpiAwarenessContext: { args: [FFIType.i64], returns: FFIType.i32 },
    SendMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i64 },
    GetForegroundWindow: { args: [], returns: FFIType.u64 },
    GetWindowThreadProcessId: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.u32 },
    WindowFromPoint: { args: [FFIType.u64], returns: FFIType.u64 },
    GetAncestor: { args: [FFIType.u64, FFIType.u32], returns: FFIType.u64 },
    ClientToScreen: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
    GetDpiForWindow: { args: [FFIType.u64], returns: FFIType.u32 },
    GetDC: { args: [FFIType.u64], returns: FFIType.u64 },
    ReleaseDC: { args: [FFIType.u64, FFIType.u64], returns: FFIType.i32 },
    EnumDisplayMonitors: { args: [FFIType.u64, FFIType.ptr, FFIType.function, FFIType.i64], returns: FFIType.i32 },
  });
  const gdi32 = dlopen("gdi32.dll", {
    GetPixel: { args: [FFIType.u64, FFIType.i32, FFIType.i32], returns: FFIType.u32 },
  });
  user32.symbols.SetProcessDpiAwarenessContext(-4n);

  const WM_MOUSEACTIVATE = 0x0021;
  const MA_ACTIVATE = 1;
  const MA_NOACTIVATE = 3;
  const WDA_EXCLUDEFROMCAPTURE = 0x11;
  const GA_ROOT = 2;

  const app = await launch({ app: "packages/test/fixtures/overlay/src/main.ts", label: "overlay" });
  afterAll(() => app.close());

  const T = 60_000;
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  type Popout = {
    title: string;
    hwnd: number;
    visible: boolean;
    shown: boolean;
    foreground: boolean;
    rect: [number, number, number, number] | null;
    ownedByMain: boolean;
    appWindow: boolean;
    toolWindow: boolean;
    nonRude: boolean;
    focusable: boolean;
    ignoreMouseEvents: boolean;
    layered: boolean;
    transparent: boolean;
    noActivate: boolean;
    displayAffinity: number;
    monitor: number | null;
    mainMonitor: [number, number, number, number] | null;
  };
  const popouts = (): Promise<Popout[]> => app.eval("JSON.parse(__sk_window_popouts())");
  const win = async (title: string) => {
    for (let i = 0; i < 100; i++) {
      const w = (await popouts()).find((p) => p.title === title);
      if (w && w.hwnd) return w;
      await sleep(100);
    }
    throw new Error(`no ${title} window`);
  };
  const set = async (js: string) => {
    await app.eval(`(() => { const S = __SK_TEST_OV__.S; ${js}; return "ok"; })()`);
    await sleep(500);
  };

  function monitors(): [number, number, number, number][] {
    const out: [number, number, number, number][] = [];
    const each = new JSCallback(
      (_m: bigint, _dc: bigint, r: number) => {
        out.push([read.i32(r as never, 0), read.i32(r as never, 4), read.i32(r as never, 8), read.i32(r as never, 12)]);
        return 1;
      },
      { args: [FFIType.u64, FFIType.u64, FFIType.ptr, FFIType.i64], returns: FFIType.i32 },
    );
    user32.symbols.EnumDisplayMonitors(0n, null, each.ptr, 0n);
    each.close();
    return out;
  }

  function screenPoint(hwnd: bigint, x: number, y: number): [number, number] {
    const pt = new Int32Array(2);
    user32.symbols.ClientToScreen(hwnd, ptr(pt));
    const scale = user32.symbols.GetDpiForWindow(hwnd) / 96;
    return [Math.round(pt[0]! + x * scale), Math.round(pt[1]! + y * scale)];
  }

  function pixel([x, y]: [number, number]): string {
    const dc = user32.symbols.GetDC(0n);
    const c = gdi32.symbols.GetPixel(dc, x, y);
    user32.symbols.ReleaseDC(0n, dc);
    return "#" + [c & 255, (c >> 8) & 255, (c >> 16) & 255].map((v) => v.toString(16).padStart(2, "0")).join("");
  }

  /** The top-level window a click at `(x, y)` would land on. */
  function under([x, y]: [number, number]): bigint {
    const w = user32.symbols.WindowFromPoint(BigInt(x >>> 0) | (BigInt(y >>> 0) << 32n));
    return user32.symbols.GetAncestor(w, GA_ROOT);
  }

  /** What the window answers when the system asks it before a left click. */
  const mouseActivate = (hwnd: bigint, top: bigint) =>
    Number(user32.symbols.SendMessageW(hwnd, WM_MOUSEACTIVATE, top, BigInt(1 | (0x0201 << 16))));

  /** The foreground window, when it is one of the overlays — `null` otherwise. */
  const overlayInForeground = async (): Promise<string | null> => {
    const fg = Number(user32.symbols.GetForegroundWindow());
    return (await popouts()).find((p) => p.hwnd === fg)?.title ?? null;
  };

  describe("overlay", () => {
    test(
      "a nonactivating HUD keeps its taskbar policy and opens as an unowned tool window",
      async () => {
        const hud = await win("HUD");
        await sleep(500);
        expect(hud.shown).toBe(true);
        expect(hud.focusable).toBe(false);
        expect(hud.toolWindow).toBe(true);
        expect(hud.appWindow).toBe(false);
        expect(hud.noActivate).toBe(true);
        expect(hud.nonRude).toBe(true);
        expect(hud.ownedByMain).toBe(false);
        expect(hud.foreground).toBe(false);
        expect(await overlayInForeground()).toBeNull();
        expect((await app.logs(0)).entries.filter((l) => l.kind === "advisory" && /SkWindow/.test(l.text))).toEqual([]);
      },
      T,
    );

    test(
      "a click would not activate it; one on an ordinary gpui window would",
      async () => {
        const hud = BigInt((await win("HUD")).hwnd);
        const main = BigInt((await app.send("screenshot", { backend: "print_window" })).hwnd);
        expect(mouseActivate(hud, hud)).toBe(MA_NOACTIVATE);
        expect(mouseActivate(main, main)).toBe(MA_ACTIVATE);
        expect(await overlayInForeground()).toBeNull();
      },
      T,
    );

    test(
      "click-through: layered + transparent, a click lands on the window under it, and back, live",
      async () => {
        let hud = await win("HUD");
        const h = BigInt(hud.hwnd);
        expect(hud.ignoreMouseEvents).toBe(true);
        expect(hud.layered).toBe(true);
        expect(hud.transparent).toBe(true);
        const box = screenPoint(h, 24, 24);
        expect(under(box)).not.toBe(h);

        await set("S.ignore = false");
        hud = await win("HUD");
        expect(hud.layered).toBe(false);
        expect(hud.transparent).toBe(false);
        expect(under(box)).toBe(h);

        await set("S.ignore = true");
        expect(under(box)).not.toBe(h);
        expect(await overlayInForeground()).toBeNull();
      },
      T,
    );

    test(
      "capture leaves it out; without contentProtection the screen shows it, and its new frames, while it is click-through",
      async () => {
        let hud = await win("HUD");
        const h = BigInt(hud.hwnd);
        const box = screenPoint(h, 24, 24);
        const clear = screenPoint(h, 120, 70);
        expect(hud.displayAffinity).toBe(WDA_EXCLUDEFROMCAPTURE);
        expect(pixel(box)).not.toBe("#ff0000");

        await set("S.protect = false");
        hud = await win("HUD");
        expect(hud.displayAffinity).toBe(0);
        expect(pixel(box)).toBe("#ff0000");
        await set('S.color = "#00ff00"');
        expect(pixel(box)).toBe("#00ff00");
        // The unpainted rest is the transparent backdrop: what is under the window, which is never the
        // box's colour.
        expect(["#ff0000", "#00ff00"]).not.toContain(pixel(clear));

        await set("S.protect = true");
        expect((await win("HUD")).displayAffinity).toBe(WDA_EXCLUDEFROMCAPTURE);
        expect(pixel(box)).not.toBe("#00ff00");
      },
      T,
    );

    test(
      "focusable is creation-time: changing it advises and changes nothing",
      async () => {
        const since = (await app.logs(0)).next;
        await set("S.focusable = true");
        const { entries } = await app.logs(since);
        expect(entries.some((l) => l.kind === "advisory" && /SkWindow focusable/.test(l.text))).toBe(true);
        const hud = await win("HUD");
        expect(hud.toolWindow).toBe(true);
        expect(hud.noActivate).toBe(true);
        expect(mouseActivate(BigInt(hud.hwnd), BigInt(hud.hwnd))).toBe(MA_NOACTIVATE);
        await set("S.focusable = false");
      },
      T,
    );

    test(
      "a cover overlay covers the monitor asked for by index, moves live, and falls back out loud",
      async () => {
        const all = monitors();
        expect(all.length).toBeGreaterThan(0);
        await set("S.monitor = 0; S.coverVisible = true");
        await sleep(500);
        let cover = await win("Cover");
        expect(cover.shown).toBe(true);
        expect(cover.rect).toEqual(all[0]!);
        expect(cover.toolWindow).toBe(true);
        expect(cover.nonRude).toBe(false);
        expect(cover.transparent).toBe(true);
        expect(await overlayInForeground()).toBeNull();

        if (all.length > 1) {
          await set("S.monitor = 1");
          await sleep(500);
          expect((await win("Cover")).rect).toEqual(all[1]!);
        }

        const since = (await app.logs(0)).next;
        await set("S.monitor = 99");
        await sleep(500);
        const { entries } = await app.logs(since);
        expect(entries.some((l) => l.kind === "advisory" && /there is no monitor 99/.test(l.text))).toBe(true);
        cover = await win("Cover");
        expect(cover.rect).toEqual(cover.mainMonitor);

        await set("S.coverVisible = false");
        expect((await win("Cover")).shown).toBe(false);
        expect(await overlayInForeground()).toBeNull();
      },
      T,
    );
  });
}
