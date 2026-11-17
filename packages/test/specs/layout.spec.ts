import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { type Bounds, launch, MACOS, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/layout/src/main.ts",
  window: [720, 520],
  label: "layout",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const box = async (selector: string): Promise<Bounds | null> => {
  const { nodes } = await app.query(selector);
  return nodes[0]?.bounds ?? null;
};

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const shown = (b: Bounds | null) => !!b && b.w > 0 && b.h > 0;

describe("#1 group-hover that changes `display`", () => {
  // macOS CI: hover, focus and popups do not follow the harness's synthetic input.
  test.skipIf(MACOS)(
    "a scroll under a still pointer neither crashes the host nor leaves the wrong row's timestamp",
    async () => {
      // Painted first (`boundingBox` waits for it): on a cold host the first hover could otherwise
      // land before the row had a hitbox.
      await app.locator("#msg-2").boundingBox();
      await app.locator("#msg-2").hover();
      bunExpect(shown(await until(() => box("#ts-2"), shown, 10_000))).toBe(true);

      for (let step = 1; step <= 6; step++) {
        await app.locator("#msgs").scroll(40);
        await sleep(120);
        bunExpect(await app.eval("1 + 1")).toBe(2);
      }

      const under = await until(() => box("#ts-8"), shown);
      bunExpect(shown(under)).toBe(true);
      bunExpect(under?.w).toBe(36);
      bunExpect(
        shown(
          await until(
            () => box("#ts-2"),
            (b) => !shown(b),
          ),
        ),
      ).toBe(false);
    },
    T,
  );
});

describe("#9 inline content in a block", () => {
  test("a mention and the text after it sit on one line inside a bubble that fits them", async () => {
    const at = await app.locator("#m-at").boundingBox();
    const name = await app.locator("#m-name").boundingBox();
    const bubble = await app.locator("#m-bubble").boundingBox();
    const text = await app.locator("#m-text").boundingBox();
    const trigger = await app.locator("#m-trigger").boundingBox();

    bunExpect(Math.abs(at.y - name.y)).toBeLessThanOrEqual(1);
    bunExpect(name.x).toBeGreaterThanOrEqual(at.x + at.w - 1);
    bunExpect(text.h).toBeLessThanOrEqual(22);
    bunExpect(bubble.w).toBeGreaterThan(24 + 60);
    bunExpect(text.w).toBeGreaterThan(60);
    bunExpect(trigger.w).toBeGreaterThan(60);
  });
});

describe("#12 calc() mixing a percentage and a length", () => {
  test("reka's slider thumb is placed at `left: calc(38.89% + 13.33px)`", async () => {
    const track = await app.locator("#track").boundingBox();
    const thumb = await app.locator("#thumb").boundingBox();
    bunExpect(Math.abs(thumb.x - track.x - 130)).toBeLessThanOrEqual(1);
  });

  test("`width: calc(100% - 40px)` is resolved against the parent", async () => {
    const w = await app.locator("#calc-w").boundingBox();
    bunExpect(Math.abs(w.w - 260)).toBeLessThanOrEqual(1);
  });
});

describe("#15 repeat(auto-fill | auto-fit, …)", () => {
  test("auto-fill makes as many columns as fit, and wraps the rest", async () => {
    const cells = await Promise.all([1, 2, 3, 4, 5].map((i) => app.locator(`#fill-${i}`).boundingBox()));
    for (let i = 0; i < 4; i++) {
      bunExpect(Math.abs(cells[i]!.w - 112.5)).toBeLessThanOrEqual(1);
      bunExpect(cells[i]!.y).toBe(cells[0]!.y);
    }
    bunExpect(cells[4]!.y).toBeGreaterThan(cells[0]!.y);
    bunExpect(cells[4]!.x).toBe(cells[0]!.x);
  });

  test("auto-fit collapses the empty columns, so two items share the row", async () => {
    const a = await app.locator("#fit-1").boundingBox();
    const b = await app.locator("#fit-2").boundingBox();
    bunExpect(Math.abs(a.w - 225)).toBeLessThanOrEqual(1);
    bunExpect(Math.abs(b.x - a.x - 225)).toBeLessThanOrEqual(1);
  });
});

describe("#14 background-image: url()", () => {
  type Rgb = [number, number, number];
  const RED: Rgb = [255, 0, 0];
  const BLUE: Rgb = [0, 0, 255];
  const LIME: Rgb = [0, 255, 0];
  const near = (a: Rgb, b: Rgb) => a.every((v, i) => Math.abs(v - b[i]!) <= 24);

  /**
   * The colours at `points` (offsets inside `selector`'s box), once the images have decoded — a
   * data: URL decodes off the render thread, so the first frames paint the box without it.
   */
  async function colours(selector: string, points: [number, number][], want: Rgb[]): Promise<Rgb[]> {
    const b = await app.locator(selector).boundingBox();
    const read = async () => {
      const shot = readPng(await app.screenshot());
      return points.map(([dx, dy]) => shot.at(Math.round(b.x + dx), Math.round(b.y + dy)));
    };
    return until(read, (got) => got.every((c, i) => near(c, want[i]!)));
  }

  test.skipIf(!SCREENSHOTS)(
    "emojix's pixel sprite shows its cell",
    async () => {
      const got = await colours("#bg-px", [[10, 10]], [BLUE]);
      bunExpect(near(got[0]!, BLUE)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "emojix's percentage sprite shows its cell",
    async () => {
      const got = await colours("#bg-pct", [[10, 10]], [BLUE]);
      bunExpect(near(got[0]!, BLUE)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`cover` scales the image over the box and `center` centres it",
    async () => {
      const got = await colours(
        "#bg-cover",
        [
          [30, 5],
          [30, 25],
        ],
        [RED, BLUE],
      );
      bunExpect(near(got[0]!, RED)).toBe(true);
      bunExpect(near(got[1]!, BLUE)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`repeat-x` tiles along the box",
    async () => {
      const points: [number, number][] = [
        [5, 10],
        [15, 10],
        [25, 10],
        [35, 10],
        [45, 10],
        [55, 10],
      ];
      const want = [RED, BLUE, RED, BLUE, RED, BLUE];
      const got = await colours("#bg-tile", points, want);
      got.forEach((c, i) => bunExpect(near(c, want[i]!)).toBe(true));
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "a gradient written above an image paints over it",
    async () => {
      const got = await colours("#bg-over", [[10, 10]], [LIME]);
      bunExpect(near(got[0]!, LIME)).toBe(true);
    },
    T,
  );
});

describe("#18 backdrop-filter under a transform", () => {
  test.skipIf(!SCREENSHOTS)(
    "the blur is drawn where the box is drawn, and nowhere else",
    async () => {
      const stage = await app.locator("#bd-stage").boundingBox();
      const shot = readPng(await app.screenshot());
      const grey = (luma: number) => luma > 50 && luma < 205;
      const region = (dx: number, dy: number) => ({ x: stage.x + dx, y: stage.y + dy, w: 40, h: 20 });
      const drawn = shot.count(region(110, 60), grey);
      bunExpect(drawn).toBeGreaterThan(40 * 20 * 0.8);
      // Before the fix the blur was drawn here instead.
      const ghost = shot.count(region(10, 10), grey);
      bunExpect(ghost).toBeLessThan(40 * 20 * 0.2);
    },
    T,
  );
});

describe("#36 a radio's dot", () => {
  test.skipIf(!SCREENSHOTS)(
    "is drawn in the inherited colour, in the middle of the circle",
    async () => {
      // It is centred only because a button centres its content vertically, and it is lime only
      // because an absolutely positioned (deferred) <svg> paints under its ancestors' text colour.
      const b = await app.locator("#radio").boundingBox();
      const lime = ([r, g, bl]: [number, number, number]) => g > 150 && r < 120 && bl < 120;
      const read = async () => {
        const shot = readPng(await app.screenshot());
        let n = 0;
        let sx = 0;
        let sy = 0;
        for (let y = Math.floor(b.y) - 8; y < b.y + b.h + 8; y++) {
          for (let x = Math.floor(b.x) - 8; x < b.x + b.w + 8; x++) {
            if (lime(shot.at(x, y))) {
              n++;
              sx += x + 0.5;
              sy += y + 0.5;
            }
          }
        }
        return { n, cx: n ? sx / n : 0, cy: n ? sy / n : 0 };
      };
      const got = await until(read, (g) => g.n >= 30);
      bunExpect(got.n).toBeGreaterThanOrEqual(30);
      bunExpect(Math.abs(got.cx - (b.x + b.w / 2))).toBeLessThanOrEqual(1);
      bunExpect(Math.abs(got.cy - (b.y + b.h / 2))).toBeLessThanOrEqual(1);
    },
    T,
  );
});

describe("the gradient-border mask (the Ultima price card)", () => {
  test.skipIf(!SCREENSHOTS)(
    "paints the lime in the padding ring and leaves the text uncovered",
    async () => {
      // Before the fix the mask was dropped, and the lime covered the whole card, text included.
      const b = await app.locator("#ring-card").boundingBox();
      const lime = ([r, g, bl]: [number, number, number]) => g > 200 && r < 100 && bl < 100;
      const read = async () => {
        const shot = readPng(await app.screenshot());
        let inner = 0;
        let ring = 0;
        for (let y = Math.ceil(b.y); y < b.y + b.h; y++) {
          for (let x = Math.ceil(b.x); x < b.x + b.w; x++) {
            if (!lime(shot.at(x, y))) continue;
            const edge = Math.min(x - b.x, b.x + b.w - x, y - b.y, b.y + b.h - y);
            if (edge > 7) inner++;
            else ring++;
          }
        }
        return { inner, ring };
      };
      const got = await until(read, (g) => g.ring > 100);
      bunExpect(got.ring).toBeGreaterThan(300);
      bunExpect(got.inner).toBe(0);
    },
    T,
  );
});

describe("#10 (b) the hidden attribute", () => {
  test(
    "hides the box, until-found too, unless the author sets a display",
    async () => {
      const flex = await until(() => box("#hid-flex"), shown);
      bunExpect(flex).toMatchObject({ w: 20, h: 20 });
      bunExpect(shown(await box("#hid"))).toBe(false);
      bunExpect(shown(await box("#hid-found"))).toBe(false);
    },
    T,
  );
});
