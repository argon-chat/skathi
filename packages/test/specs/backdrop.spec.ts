// DWM draws the system backdrops only for an ACTIVE frame, and sk-test never activates a window
// (SK_WINDOW_FOCUS=never).

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { launch, readPng } from "../src/index";

const user32 = dlopen("user32.dll", {
  SendMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i64 },
  GetForegroundWindow: { args: [], returns: FFIType.u64 },
});
const WM_NCACTIVATE = 0x0086;

const app = await launch({ app: "packages/test/fixtures/backdrop/src/main.ts", label: "backdrop" });
afterAll(() => app.close());

const T = 60_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

/** `DWMWA_SYSTEMBACKDROP_TYPE` per material, on Windows 11 22H2 and later. */
const DWMSBT: Record<string, number> = { none: 1, mica: 2, acrylic: 3, "mica-alt": 4, transparent: 1 };
const MATERIALS = ["none", "mica", "mica-alt", "acrylic", "transparent"];

type State = { backdrop: string; system: number | null; dark: boolean | null };
const state = async (): Promise<State> =>
  JSON.parse(
    await app.eval(`(() => {
      const s = JSON.parse(__sk_window_state());
      return JSON.stringify({ backdrop: s.backdrop, system: s.systemBackdrop, dark: s.systemDarkMode });
    })()`),
  );
const setBackdrop = async (b: string) => {
  await app.eval(`__SK_WINDOW__.appWindow.backdrop = ${JSON.stringify(b)}; "ok"`);
  await sleep(500);
};
const setTheme = async (t: string) => {
  await app.eval(`__SK_WINDOW__.appWindow.theme = ${JSON.stringify(t)}; "ok"`);
  await sleep(500);
};

let hwnd = 0n;
const activeFrame = async (on: boolean) => {
  user32.symbols.SendMessageW(hwnd, WM_NCACTIVATE, on ? 1n : 0n, -1n);
  await sleep(500);
};

type Shot = { mean: Rgb; spread: number; solid: Rgb; top: Rgb };
async function shoot(): Promise<Shot> {
  const r = await app.send("screenshot", { backend: "print_window" });
  hwnd = BigInt(r.hwnd);
  const shot = readPng(r.path);
  const at = (x: number, y: number) =>
    shot.at(Math.round(x * r.scale + r.clientX), Math.round(y * r.scale + r.clientY));
  const sum: Rgb = [0, 0, 0];
  let n = 0;
  let lo = 255;
  let hi = 0;
  for (let y = 60; y < 144; y += 4) {
    for (let x = 60; x < 224; x += 4) {
      const p = at(x, y);
      for (let i = 0; i < 3; i++) sum[i]! += p[i]!;
      const l = (p[0] + p[1] + p[2]) / 3;
      lo = Math.min(lo, l);
      hi = Math.max(hi, l);
      n++;
    }
  }
  return { mean: sum.map((v) => Math.round(v / n)) as Rgb, spread: hi - lo, solid: at(20, 20), top: at(120, 1) };
}

const luma = (c: Rgb) => (c[0] + c[1] + c[2]) / 3;
const close = (a: Rgb, b: Rgb, tol = 6) => a.every((v, i) => Math.abs(v - b[i]!) <= tol);

describe("backdrop", () => {
  test(
    "opens on acrylic from sk.config.json: DWM's own, read back, and nothing advised",
    async () => {
      await sleep(500);
      const s = await state();
      expect(s.backdrop).toBe("acrylic");
      expect(s.system).toBe(DWMSBT.acrylic!);
      const { entries } = await app.logs(0);
      expect(entries.filter((l) => l.kind === "advisory" && /acrylic/.test(l.text))).toEqual([]);
      const shot = await shoot();
      expect(close(shot.solid, [255, 0, 0], 8)).toBe(true);
    },
    T,
  );

  test("every pair of materials, both orders: DWM reads back the one asked for, and its pixels do not depend on the one before", async () => {
    const ref: Record<string, Rgb> = {};
    for (const m of MATERIALS) {
      await setBackdrop("none");
      await setBackdrop(m);
      ref[m] = (await shoot()).mean;
    }
    expect(luma(ref.none!)).toBeGreaterThan(240);
    expect(close(ref.acrylic!, ref.mica!)).toBe(false);
    expect(close(ref.acrylic!, ref.none!)).toBe(false);

    const bad: string[] = [];
    for (const a of MATERIALS) {
      for (const b of MATERIALS) {
        if (a === b) continue;
        await setBackdrop(a);
        await setBackdrop(b);
        const s = await state();
        const { mean } = await shoot();
        if (s.backdrop !== b || s.system !== DWMSBT[b]) bad.push(`${a} -> ${b}: read back ${s.backdrop} / ${s.system}`);
        // `transparent` is the desktop behind, which PrintWindow does not carry.
        if (b !== "transparent" && !close(mean, ref[b]!))
          bad.push(`${a} -> ${b}: ${mean} where ${b} alone is ${ref[b]}`);
      }
    }
    expect(bad).toEqual([]);
  }, 180_000);

  // Each active read therefore re-tells the frame right before its shot, and each comparison gets
  // three tries.
  const activeAfter = async (from: string, to: string): Promise<Rgb> => {
    await setBackdrop(from);
    await setBackdrop(to);
    await activeFrame(true);
    return (await shoot()).mean;
  };
  const agree = async (a: () => Promise<Rgb>, b: () => Promise<Rgb>, same: boolean) => {
    const seen: string[] = [];
    for (let i = 0; i < 3; i++) {
      const [x, y] = [await a(), await b()];
      if (close(x, y) === same) return "ok";
      seen.push(`${x} vs ${y}`);
    }
    return seen.join("; ");
  };

  test("an active frame draws the material itself, the same whichever material came before", async () => {
    try {
      const inactive = async () => {
        await activeFrame(false);
        await setBackdrop("acrylic");
        return (await shoot()).mean;
      };
      expect(await agree(() => activeAfter("none", "acrylic"), inactive, false)).toBe("ok");
      expect(
        await agree(
          () => activeAfter("none", "acrylic"),
          () => activeAfter("none", "mica"),
          false,
        ),
      ).toBe("ok");
      for (const [from, to] of [
        ["mica", "acrylic"],
        ["mica-alt", "acrylic"],
        ["acrylic", "mica"],
        ["acrylic", "mica-alt"],
      ]) {
        expect(
          await agree(
            () => activeAfter(from!, to!),
            () => activeAfter("none", to!),
            true,
          ),
        ).toBe("ok");
      }
    } finally {
      await activeFrame(false);
    }
    // The machine may be in use, so the foreground can move for its own reasons; what must not
    // happen is that it moves to this window.
    expect(user32.symbols.GetForegroundWindow()).not.toBe(hwnd);
  }, 120_000);

  test(
    "acrylic follows appWindow.theme, dark and light, active or not",
    async () => {
      const read = async () => {
        await activeFrame(false);
        const inactive = (await shoot()).mean;
        const active = await activeAfter("acrylic", "acrylic");
        await activeFrame(false);
        return { inactive, active, dark: (await state()).dark };
      };
      await setBackdrop("acrylic");
      await setTheme("dark");
      const dark = await read();
      await setTheme("light");
      const light = await read();
      await setTheme("system");
      expect(dark.dark).toBe(true);
      expect(light.dark).toBe(false);
      expect(luma(dark.inactive)).toBeLessThan(128);
      expect(luma(light.inactive)).toBeGreaterThan(128);
      expect(luma(light.active) - luma(dark.active)).toBeGreaterThan(40);
    },
    T,
  );

  test(
    "frameless, the material reaches the top edge as it fills the middle",
    async () => {
      await setBackdrop("acrylic");
      const inactive = await shoot();
      await activeFrame(true);
      const active = await shoot();
      await activeFrame(false);
      expect(close(inactive.top, inactive.mean, 10)).toBe(true);
      expect(close(active.top, active.mean, 10)).toBe(true);
    },
    T,
  );
});
