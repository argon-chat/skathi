import { afterAll, describe, expect, setDefaultTimeout, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { type Bounds, launch, readPng, SCREENSHOTS, WINDOWS } from "../src/index";

setDefaultTimeout(30_000);

const app = await launch({
  app: "packages/test/fixtures/css-scheme-sizing/src/main.ts",
  window: [720, 560],
  label: "css-scheme",
});
afterAll(async () => {
  await app.eval(`__SK_WINDOW__.appWindow.theme = "system"; "ok"`);
  app.close();
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(30);
    v = await read();
  }
  return v;
}

const LIGHT = "rgb(10, 20, 30)";
const DARK = "rgb(200, 210, 220)";
const TONES = ["ld", "ld-light", "ld-dark", "ld-normal", "ld-var"] as const;

const colors = async (): Promise<Record<string, string>> =>
  JSON.parse(
    await app.eval(`JSON.stringify(Object.fromEntries(${JSON.stringify(TONES)}.map(
      (id) => [id, getComputedStyle(document.getElementById(id)).color])))`),
  );
const setTheme = (theme: string) => app.eval(`__SK_WINDOW__.appWindow.theme = ${JSON.stringify(theme)}; "ok"`);
const media = async (query: string) =>
  JSON.parse(await app.eval(`JSON.stringify(matchMedia(${JSON.stringify(query)}).matches)`)) as boolean;

describe("color-scheme and light-dark()", () => {
  test("the computed value inherits from :root and an element's own wins", async () => {
    const scheme = (id: string) => app.eval(`getComputedStyle(document.getElementById("${id}")).colorScheme`);
    expect(await scheme("ld")).toBe("light dark");
    expect(await scheme("ld-light")).toBe("light");
    expect(await scheme("ld-dark")).toBe("dark only");
    expect(await scheme("ld-normal")).toBe("normal");
  });

  test("a forced dark theme takes the dark half where the scheme allows it", async () => {
    await setTheme("dark");
    const seen = await until(colors, (c) => c.ld === DARK);
    expect(seen).toEqual({ ld: DARK, "ld-light": LIGHT, "ld-dark": DARK, "ld-normal": LIGHT, "ld-var": DARK });
    expect(await media("(prefers-color-scheme: dark)")).toBe(true);
  });

  test("switching back to light updates live", async () => {
    await setTheme("light");
    const seen = await until(colors, (c) => c.ld === LIGHT);
    expect(seen).toEqual({ ld: LIGHT, "ld-light": LIGHT, "ld-dark": DARK, "ld-normal": LIGHT, "ld-var": LIGHT });
  });

  test("`system` follows the OS setting", async () => {
    await setTheme("system");
    const os = await media("(prefers-color-scheme: dark)");
    const want = os ? DARK : LIGHT;
    expect((await until(colors, (c) => c.ld === want)).ld).toBe(want);
  });

  test.if(SCREENSHOTS)("a checkbox under `color-scheme: dark` draws dark", async () => {
    await setTheme("light");
    const centre = async (id: string) => {
      const { nodes } = await app.query(`#${id}`);
      const b = nodes[0]?.bounds as Bounds;
      const shot = readPng(await app.screenshot());
      const k = shot.width / 720;
      return shot.at(Math.floor((b.x + b.w / 2) * k), Math.floor((b.y + b.h / 2) * k));
    };
    const [dr, dg, db] = await until(
      () => centre("cb-dark"),
      ([r]) => r < 100,
    );
    expect(Math.max(dr, dg, db)).toBeLessThan(100);
    const [lr, lg, lb] = await centre("cb-light");
    expect(Math.min(lr, lg, lb)).toBeGreaterThan(200);
    await setTheme("system");
  });
});

/** `EnableTransparency` under the Personalize key; 0 is "Transparency effects" off. */
const osReducesTransparency = (): boolean => {
  try {
    const out = execFileSync(
      "reg",
      ["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "/v", "EnableTransparency"],
      { encoding: "utf8" },
    );
    return /EnableTransparency\s+REG_DWORD\s+0x0\b/.test(out);
  } catch {
    return false;
  }
};

describe("media features", () => {
  test("prefers-reduced-transparency is understood, and answers the OS setting on Windows", async () => {
    const reduce = await media("(prefers-reduced-transparency: reduce)");
    expect(await media("(prefers-reduced-transparency: no-preference)")).toBe(!reduce);
    expect(await media("(prefers-reduced-transparency)")).toBe(reduce);
    expect(reduce).toBe(WINDOWS ? osReducesTransparency() : false);
  });

  test("device-width and device-height are the monitor's size, with min- and max- forms", async () => {
    const [w, h] = JSON.parse(await app.eval(`JSON.stringify([screen.width, screen.height])`)) as [number, number];
    expect(w).toBeGreaterThan(0);
    expect(await media(`(device-width: ${w}px)`)).toBe(true);
    expect(await media(`(min-device-width: ${w}px)`)).toBe(true);
    expect(await media(`(min-device-width: ${w + 1}px)`)).toBe(false);
    expect(await media(`(max-device-height: ${h}px)`)).toBe(true);
    expect(await media(`(max-device-height: ${h - 1}px)`)).toBe(false);
    expect(await media(`(${h - 1}px < device-height <= ${h}px)`)).toBe(true);
  });

  test("a device-size @media block applies", async () => {
    const b = await until(
      async () => (await app.query("#device")).nodes[0]?.bounds?.w ?? 0,
      (w) => w === 33,
    );
    expect(b).toBe(33);
  });
});
