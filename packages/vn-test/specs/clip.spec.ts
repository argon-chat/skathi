// Rounded clipping: `overflow` under a `border-radius` clips to the rounded
// padding box. gpui's content masks were rectangles, so the Argon client's
// avatars (reka's AvatarRoot, `rounded-full overflow-hidden` around an <img>)
// came out square, and its rounded cards showed their lists' rows in the
// corners. The fork's masks carry corner radii now (docs/ARCHITECTURE.md,
// fork item 33), and a percentage radius resolves against the box at paint
// (fork item 34).
//
//   bun test packages/vn-test/specs/clip.spec.ts
//
// `fixtures/clip` lays the shapes out at fixed positions on white.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/clip/src/main.ts",
  window: [720, 520],
  label: "clip",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Rgb = [number, number, number];

const box = async (selector: string): Promise<Bounds> => {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

/** Polls `read` until `ok` holds, and returns the last answer either way. */
async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

/** A screenshot, read at CSS pixel `(x, y)`'s centre whatever the scale. */
const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  return {
    at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)),
    /** How many CSS pixels of `b` satisfy `hit`, sampled at their centres. */
    count: (b: Bounds, hit: (rgb: Rgb) => boolean) => {
      let n = 0;
      for (let y = 0; y < b.h; y++) for (let x = 0; x < b.w; x++) if (hit(shot.at(Math.floor((b.x + x + 0.5) * k), Math.floor((b.y + y + 0.5) * k)))) n++;
      return n;
    },
  };
};

const white = ([r, g, b]: Rgb) => r > 235 && g > 235 && b > 235;
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const lime = ([r, g, b]: Rgb) => g > 200 && r < 60 && b < 60;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 60 && g < 60;
const gray = ([r, g, b]: Rgb) => Math.abs(r - 136) < 20 && Math.abs(g - 136) < 20 && Math.abs(b - 136) < 20;

/** The four corner pixels `inset` in from each corner of `b`. */
const corners = (b: Bounds, inset: number): [number, number][] => [
  [b.x + inset, b.y + inset],
  [b.x + b.w - 1 - inset, b.y + inset],
  [b.x + inset, b.y + b.h - 1 - inset],
  [b.x + b.w - 1 - inset, b.y + b.h - 1 - inset],
];

describe("a round avatar (reka's AvatarRoot around an <img>)", () => {
  test("the image is clipped to the circle", async () => {
    const b = await box("#avatar");
    bunExpect(b).toMatchObject({ w: 64, h: 64 });
    const area = (s: Awaited<ReturnType<typeof shoot>>) => s.count(b, red);
    // The image decodes asynchronously; wait for it to paint at all.
    await until(async () => area(await shoot()), (n) => n > 1000);
    const s = await shoot();
    // A 64px disc is pi * 32^2 = 3217 px; the square was 4096.
    bunExpect(area(s)).toBeGreaterThan(3050);
    bunExpect(area(s)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
    bunExpect(red(s.at(b.x + 32, b.y + 32))).toBe(true);
    bunExpect(red(s.at(b.x + 32, b.y + 2))).toBe(true);
  }, T);

  test("an <img> with `border-radius: 50%` is a disc too", async () => {
    const b = await box("#round-img");
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeGreaterThan(3050);
    bunExpect(s.count(b, red)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
  }, T);

  test("`50%` on a box that is not square clips to a pill", async () => {
    // 120x44: CSS draws 60x22 elliptical corners (area pi*60*22 = 4147).
    // gpui's corners are circles, so the smaller radius makes a 22px pill:
    // 76*44 + pi*22^2 = 4865. The square box was 5280. This pins the
    // documented divergence (crates/vn-css/README.md §5 item 13).
    const b = await box("#oval");
    bunExpect(b).toMatchObject({ w: 120, h: 44 });
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeGreaterThan(4750);
    bunExpect(s.count(b, red)).toBeLessThan(5000);
    for (const [x, y] of corners(b, 2)) bunExpect(white(s.at(x, y))).toBe(true);
    bunExpect(red(s.at(b.x + 60, b.y + 1))).toBe(true);
    bunExpect(red(s.at(b.x + 1, b.y + 22))).toBe(true);
    // The advisory goes out on the build after the paint that drew the
    // circle; nudge a frame in case the page has gone idle.
    await app.eval(`document.getElementById("tick").style.right = "3px"`);
    const said = await until(
      async () => JSON.stringify((await app.logs()).entries),
      (text) => text.includes("elliptical corner"),
    );
    bunExpect(said).toContain("elliptical corner");
  }, T);
});

describe("`border-radius: inherit` (the Argon client's avatars and profile card)", () => {
  test("an inline `inherit` on an <img> takes its round wrapper's radius", async () => {
    const b = await box("#inh-img");
    bunExpect(b).toMatchObject({ w: 64, h: 64 });
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeGreaterThan(3050);
    bunExpect(s.count(b, red)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
  }, T);

  test("a card's layers round themselves from it, two levels down", async () => {
    // radius 16 on an 88x46 card: (2, 2) from a corner is outside the curve.
    const b = await box("#pcard");
    bunExpect(b).toMatchObject({ w: 88, h: 46 });
    const s = await until(shoot, (s) => red(s.at(b.x + 44, b.y + 6)));
    // The red background clips at the inherited curve; the blue body, two
    // `inherit`s down, paints its bottom corners round. Square, either one
    // showed in the corners.
    for (const [x, y] of corners(b, 2)) bunExpect(white(s.at(x, y))).toBe(true);
    const [tl, tr, bl, br] = corners(b, 8);
    for (const [x, y] of [tl, tr]) bunExpect(red(s.at(x, y))).toBe(true);
    for (const [x, y] of [bl, br]) bunExpect(blue(s.at(x, y))).toBe(true);
  }, T);
});

describe("a rounded card around a list that scrolls", () => {
  const banded = (rgb: Rgb) => lime(rgb) || blue(rgb);

  test("the rows clip at the card's curve, before and after scrolling", async () => {
    const b = await box("#card-a");
    const check = async () => {
      const s = await shoot();
      // radius 24: (3, 3) is 29px from the corner circle's centre.
      for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
      // Inside the curve, and the straight edges, are the rows.
      for (const [x, y] of corners(b, 9)) bunExpect(banded(s.at(x, y))).toBe(true);
      bunExpect(banded(s.at(b.x + 100, b.y))).toBe(true);
      bunExpect(banded(s.at(b.x, b.y + 80))).toBe(true);
    };
    await until(shoot, (s) => banded(s.at(b.x + 100, b.y + 80)));
    await check();

    const top = await app.eval(`(() => { const l = document.getElementById("list-a"); l.scrollTop = 37; return l.scrollTop; })()`);
    bunExpect(top).toBe(37);
    // Band 0 is lime and 40px tall, so 37px down the top row is lime's last
    // 3px and then blue: the list moved under the mask.
    await until(shoot, (s) => blue(s.at(b.x + 100, b.y + 10)));
    await check();
  }, T);

  test("a bordered scroller clips its rows to the inner radius", async () => {
    // radius 24 less the 4px border: the padding box's corners are 20px
    // circles around the same centres, (24, 24) in from each corner.
    const b = await box("#card-b");
    const check = async () => {
      const s = await shoot();
      for (const [x, y] of corners(b, 2)) bunExpect(white(s.at(x, y))).toBe(true);
      // 21.9px from the centre: inside the border's curve, outside the
      // padding box's. The rows used to cover it.
      for (const [x, y] of corners(b, 8)) bunExpect(gray(s.at(x, y))).toBe(true);
      bunExpect(banded(s.at(b.x + 100, b.y + 80))).toBe(true);
    };
    await until(shoot, (s) => banded(s.at(b.x + 100, b.y + 80)));
    await check();

    const top = await app.eval(`(() => { const l = document.getElementById("card-b"); l.scrollTop = 57; return l.scrollTop; })()`);
    bunExpect(top).toBe(57);
    await sleep(200);
    await check();
  }, T);
});

describe("box-shadow outside the border box (CSS Backgrounds 3 §7.1)", () => {
  // An outer shadow is drawn only outside the border box, so a control with a
  // transparent or translucent background and a focus ring (Tailwind's
  // `ring-[3px]`, a spread shadow) shows a ring, not a filled slab of the
  // ring colour. `filter: blur()` on a solid box, which vue-native draws as
  // the box's own shadow, stays filled.
  test("a ring around a transparent box is a ring, and a blurred box stays filled", async () => {
    await app.eval(`(() => {
      const pad = document.createElement("div");
      pad.id = "shadow-pad";
      pad.setAttribute("style", "position: fixed; left: 540px; top: 380px; width: 170px; height: 130px; z-index: 100; background-color: rgb(255, 255, 255)");
      const ring = document.createElement("div");
      ring.id = "ring";
      ring.setAttribute("style", "position: absolute; left: 20px; top: 20px; width: 80px; height: 40px; border-radius: 6px; background-color: transparent; box-shadow: 0 0 0 6px rgb(255, 0, 0)");
      const blur = document.createElement("div");
      blur.id = "blurred";
      blur.setAttribute("style", "position: absolute; left: 115px; top: 20px; width: 40px; height: 40px; background-color: rgb(0, 0, 255); filter: blur(2px)");
      pad.append(ring, blur);
      document.body.appendChild(pad);
      return 0;
    })()`);
    const r = await box("#ring");
    const b = await box("#blurred");
    const s = await until(shoot, (s) => red(s.at(r.x - 3, r.y + 20)));
    bunExpect(red(s.at(r.x - 3, r.y + 20))).toBe(true);
    bunExpect(red(s.at(r.x + 40, r.y + r.h + 3))).toBe(true);
    // The middle is the white pad, not the ring colour.
    bunExpect(white(s.at(r.x + 40, r.y + 20))).toBe(true);
    bunExpect(white(s.at(r.x + 3, r.y + 3))).toBe(true);
    bunExpect(blue(s.at(b.x + 20, b.y + 20))).toBe(true);
    await app.eval(`(document.getElementById("shadow-pad").remove(), 0)`);
  }, T);
});

describe("clip-path (CSS Masking 1 §5)", () => {
  // A rounded mask around the element's whole painting: background, border,
  // shadow and subtree, and its hit test, inside its transform (gpui
  // `Style::vn_clip_path`). The shapes sit on a white pad over the fixture.
  test("inset() and circle() clip the painting and the hit test, inside the transform", async () => {
    await app.eval(`(() => {
      const pad = document.createElement("div");
      pad.id = "cp-pad";
      pad.setAttribute("style", "position: fixed; left: 520px; top: 380px; width: 190px; height: 130px; z-index: 100; background-color: rgb(255, 255, 255)");
      const shape = (id, style) => {
        const el = document.createElement("div");
        el.id = id;
        el.setAttribute("style", "position: absolute; background-color: rgb(255, 0, 0); " + style);
        el.addEventListener("click", (e) => { e.stopPropagation(); globalThis.cpHits.push(id); });
        pad.appendChild(el);
      };
      globalThis.cpHits = [];
      pad.addEventListener("click", () => globalThis.cpHits.push("pad"));
      // The border and the ring shadow lie outside the inset: both go.
      shape("cp-inset", "left: 10px; top: 10px; width: 54px; height: 34px; border: 3px solid rgb(0, 0, 255); box-shadow: 0 0 0 6px rgb(0, 255, 0); clip-path: inset(8px 15px round 6px)");
      shape("cp-circle", "left: 80px; top: 10px; width: 60px; height: 60px; clip-path: circle(50%)");
      // The clip moves with the transform: the left half of the painted box.
      shape("cp-moved", "left: 10px; top: 80px; width: 60px; height: 40px; transform: translateX(20px); clip-path: inset(0 50% 0 0)");
      // The screen-reader-only idiom: nothing painted, nothing hit.
      shape("cp-sr", "left: 150px; top: 80px; width: 30px; height: 30px; clip-path: inset(50%)");
      document.body.appendChild(pad);
      return 0;
    })()`);
    const pad = await box("#cp-pad");
    const i = await box("#cp-inset");
    const c = await box("#cp-circle");
    bunExpect(c).toMatchObject({ w: 60, h: 60 });
    const s = await until(shoot, (s) => red(s.at(c.x + 30, c.y + 30)));

    // inset(8px 15px round 6px) of the 60x40 border box: x 15..45, y 8..32.
    bunExpect(red(s.at(i.x + 30, i.y + 20))).toBe(true);
    bunExpect(red(s.at(i.x + 17, i.y + 20))).toBe(true);
    for (const [x, y] of [
      [i.x + 5, i.y + 20], // the left inset, over the blue border
      [i.x + 30, i.y + 3], // the top inset
      [i.x - 3, i.y + 20], // the ring shadow
      [i.x + 15, i.y + 8], // outside the 6px corner
    ]) bunExpect(white(s.at(x, y))).toBe(true);

    // circle(50%) of a 60px square is its disc: pi * 30^2 = 2827 of 3600.
    bunExpect(s.count(c, red)).toBeGreaterThan(2700);
    bunExpect(s.count(c, red)).toBeLessThan(2950);
    for (const [x, y] of corners(c, 3)) bunExpect(white(s.at(x, y))).toBe(true);

    // translateX(20px) carries the clip along: the painted box is x 30..90
    // of the pad and its left half, 30..60, shows.
    bunExpect(red(s.at(pad.x + 45, pad.y + 100))).toBe(true);
    bunExpect(white(s.at(pad.x + 75, pad.y + 100))).toBe(true);
    bunExpect(white(s.at(pad.x + 20, pad.y + 100))).toBe(true);

    // inset(50%) encloses nothing.
    bunExpect(s.count({ x: pad.x + 150, y: pad.y + 80, w: 30, h: 30 }, red)).toBe(0);

    // A press outside the shape reaches what is under it.
    const click = async (x: number, y: number) => {
      await app.send("mouse", { x, y, kind: "click" });
      await sleep(100);
    };
    await click(c.x + 3, c.y + 3);
    await click(c.x + 30, c.y + 30);
    await click(pad.x + 165, pad.y + 95);
    await click(pad.x + 75, pad.y + 100);
    await click(pad.x + 45, pad.y + 100);
    const hits = await until(
      async () => JSON.parse(String(await app.eval("JSON.stringify(globalThis.cpHits)"))) as string[],
      (h) => h.length >= 5,
    );
    bunExpect(hits).toEqual(["pad", "cp-circle", "pad", "pad", "cp-moved"]);
    await app.eval(`(document.getElementById("cp-pad").remove(), 0)`);
  }, T);
});

describe("clip-path: polygon() and path() (CSS Shapes 1 §3.1)", () => {
  // A shape that is no rounded rectangle paints the element into an isolated
  // layer, composited through the shape's coverage (gpui `VnClipGeometry`,
  // docs/ARCHITECTURE.md fork item 49). Red shapes on a white pad.
  const PAD = "position: fixed; left: 10px; top: 10px; width: 700px; height: 220px; z-index: 200; background-color: rgb(255, 255, 255)";
  // The EXOBIO overlay's notch, 120 wide: concave shoulders from the top
  // corners down to x = 20 and x = 100, rounded bottom corners.
  const NOTCH = "M0 0 C11 0 20 9 20 20 V40 Q20 60 40 60 H80 Q100 60 100 40 V20 C100 9 109 0 120 0 Z";
  const HOLE = "M0 0 h60 v60 h-60 z M15 15 h30 v30 h-30 z";

  test("a polygon, a path with curves, and evenodd clip the painting and the hit test", async () => {
    await app.eval(`(() => {
      const pad = document.createElement("div");
      pad.id = "cps-pad";
      pad.setAttribute("style", ${JSON.stringify(PAD)});
      globalThis.cpsHits = [];
      pad.addEventListener("click", () => globalThis.cpsHits.push("pad"));
      const shape = (id, style, parent = pad) => {
        const el = document.createElement("div");
        el.id = id;
        el.setAttribute("style", "position: absolute; " + style);
        parent.appendChild(el);
        return el;
      };
      const hit = (el) => el.addEventListener("click", (e) => { e.stopPropagation(); globalThis.cpsHits.push(el.id); });
      hit(shape("cps-tri", "left: 10px; top: 10px; width: 80px; height: 60px; background-color: rgb(255, 0, 0); clip-path: polygon(50% 0, 100% 100%, 0 100%)"));
      // The notch's painting is all a positioned child's, which spills
      // 20px around it: the child paints apart from the clipped element
      // (a deferred draw) and is cut all the same, as the overlay's sprites are.
      const notch = shape("cps-notch", "left: 110px; top: 30px; width: 120px; height: 60px; clip-path: path('${NOTCH}')");
      shape("cps-glow", "left: -20px; top: -20px; width: 160px; height: 100px; background-color: rgb(255, 0, 0)", notch);
      hit(notch);
      shape("cps-hole", "left: 250px; top: 10px; width: 60px; height: 60px; background-color: rgb(255, 0, 0); clip-path: path(evenodd, '${HOLE}')");
      shape("cps-nonzero", "left: 330px; top: 10px; width: 60px; height: 60px; background-color: rgb(255, 0, 0); clip-path: path('${HOLE}')");
      document.body.appendChild(pad);
      return 0;
    })()`);
    const t = await box("#cps-tri");
    const n = await box("#cps-notch");
    const h = await box("#cps-hole");
    const z = await box("#cps-nonzero");
    bunExpect(t).toMatchObject({ w: 80, h: 60 });
    const s = await until(shoot, (s) => red(s.at(t.x + 40, t.y + 45)) && red(s.at(n.x + 60, n.y + 30)));

    // polygon(50% 0, 100% 100%, 0 100%) of 80x60: half the box, 2400 px.
    bunExpect(s.count(t, red)).toBeGreaterThan(2250);
    bunExpect(s.count(t, red)).toBeLessThan(2550);
    for (const [x, y] of [[t.x + 5, t.y + 5], [t.x + 75, t.y + 5], [t.x + 15, t.y + 15]]) bunExpect(white(s.at(x, y))).toBe(true);
    for (const [x, y] of [[t.x + 40, t.y + 3], [t.x + 40, t.y + 57], [t.x + 5, t.y + 57]]) bunExpect(red(s.at(x, y))).toBe(true);

    // The notch: inside, right of the concave shoulder near the top, and
    // between the rounded bottom corners.
    for (const [x, y] of [[n.x + 60, n.y + 30], [n.x + 25, n.y + 4], [n.x + 95, n.y + 4], [n.x + 60, n.y + 58]]) {
      bunExpect(red(s.at(x, y))).toBe(true);
    }
    // Outside: the shoulders' hollows, the rounded corners, and the glow
    // where it spills above, left and below the element.
    for (const [x, y] of [
      [n.x + 5, n.y + 12],
      [n.x + 3, n.y + 40],
      [n.x + 116, n.y + 30],
      [n.x + 21, n.y + 58],
      [n.x + 99, n.y + 58],
      [n.x + 60, n.y - 8],
      [n.x - 10, n.y + 30],
      [n.x + 60, n.y + 70],
    ]) bunExpect(white(s.at(x, y))).toBe(true);

    // evenodd cuts the inner square out; nonzero, both squares wound the
    // same way, keeps it.
    bunExpect(white(s.at(h.x + 30, h.y + 30))).toBe(true);
    bunExpect(red(s.at(h.x + 5, h.y + 5))).toBe(true);
    bunExpect(red(s.at(z.x + 30, z.y + 30))).toBe(true);
    bunExpect(s.count(h, red)).toBeGreaterThan(2600); // 3600 - 900
    bunExpect(s.count(h, red)).toBeLessThan(2800);

    // A press outside the shape reaches what is under it; inside it is the
    // element's. The glow that spills above the notch is not hit either.
    const click = async (x: number, y: number) => {
      await app.send("mouse", { x, y, kind: "click" });
      await sleep(100);
    };
    await click(t.x + 15, t.y + 15);
    await click(t.x + 40, t.y + 45);
    await click(n.x + 4, n.y + 40);
    await click(n.x + 60, n.y + 30);
    await click(n.x + 60, n.y - 8);
    const hits = await until(
      async () => JSON.parse(String(await app.eval("JSON.stringify(globalThis.cpsHits)"))) as string[],
      (h) => h.length >= 5,
    );
    bunExpect(hits).toEqual(["pad", "cps-tri", "pad", "cps-notch", "pad"]);
    await app.eval(`(document.getElementById("cps-pad").remove(), 0)`);
  }, T);

  test("an inline path() rewritten every frame clips to the frame's shape", async () => {
    // The overlay's spring rewrites `clip-path: path(...)` inline on every
    // animation frame; each frame's shape is the one that clips.
    await app.eval(`(() => {
      const pad = document.createElement("div");
      pad.id = "cps-pad";
      pad.setAttribute("style", ${JSON.stringify(PAD)});
      const el = document.createElement("div");
      el.id = "cps-anim";
      el.setAttribute("style", "position: absolute; left: 10px; top: 10px; width: 140px; height: 40px; background-color: rgb(255, 0, 0)");
      pad.appendChild(el);
      document.body.appendChild(pad);
      globalThis.cpsAnimate = (from, to, frames) => new Promise((done) => {
        const gaps = [];
        let i = 0, last = performance.now();
        const step = (now) => {
          gaps.push(now - last);
          last = now;
          const w = from + ((to - from) * i) / frames;
          el.style.clipPath = "path('M0 0 H" + w + " Q" + (w + 8) + " 20 " + w + " 40 H0 Z')";
          if (i++ < frames) requestAnimationFrame(step);
          else done(gaps.slice(2));
        };
        requestAnimationFrame(step);
      });
      return 0;
    })()`);
    const a = await box("#cps-anim");
    // `eval` does not wait for a promise: the animation leaves its frame
    // gaps in a global when it is done.
    const animate = async (from: number, to: number) => {
      await app.eval(`(globalThis.cpsGaps = null, cpsAnimate(${from}, ${to}, 40).then((g) => (globalThis.cpsGaps = g)), 0)`);
      return (await until(
        async () => JSON.parse(String(await app.eval("JSON.stringify(globalThis.cpsGaps)"))) as number[] | null,
        (g) => g !== null,
        10_000,
      )) as number[];
    };
    const gaps = await animate(10, 60);
    let s = await until(shoot, (s) => red(s.at(a.x + 55, a.y + 20)) && white(s.at(a.x + 75, a.y + 20)));
    bunExpect(red(s.at(a.x + 55, a.y + 20))).toBe(true);
    bunExpect(red(s.at(a.x + 63, a.y + 20))).toBe(true); // the bulge of the curve
    bunExpect(white(s.at(a.x + 75, a.y + 20))).toBe(true);
    bunExpect(white(s.at(a.x + 62, a.y + 2))).toBe(true);
    await animate(60, 120);
    s = await until(shoot, (s) => red(s.at(a.x + 115, a.y + 20)));
    bunExpect(red(s.at(a.x + 115, a.y + 20))).toBe(true);
    bunExpect(white(s.at(a.x + 134, a.y + 20))).toBe(true);
    // Frames keep coming while the clip changes every one of them.
    const mean = gaps.reduce((x, y) => x + y, 0) / gaps.length;
    console.log(`clip-path path() animation: ${gaps.length} frames, mean rAF gap ${mean.toFixed(2)} ms`);
    bunExpect(mean).toBeLessThan(40);
    await app.eval(`(document.getElementById("cps-pad").remove(), 0)`);
  }, T);

  test("nested clips intersect: a circle inside a polygon, and a polygon inside a polygon", async () => {
    await app.eval(`(() => {
      const pad = document.createElement("div");
      pad.id = "cps-pad";
      pad.setAttribute("style", ${JSON.stringify(PAD)});
      const outer = (id, left) => {
        const el = document.createElement("div");
        el.id = id;
        el.setAttribute("style", "position: absolute; top: 10px; left: " + left + "px; width: 100px; height: 100px; clip-path: polygon(0 0, 100% 0, 0 100%)");
        pad.appendChild(el);
        return el;
      };
      const inner = (parent, clip) => {
        const el = document.createElement("div");
        el.setAttribute("style", "width: 100px; height: 100px; background-color: rgb(255, 0, 0); clip-path: " + clip);
        parent.appendChild(el);
      };
      // A rounded mask inside the layer: circle(40%), centre (50, 50), r 40.
      inner(outer("cps-o1", 10), "circle(40%)");
      // A second layer inside the first: the upper-right triangle.
      inner(outer("cps-o2", 130), "polygon(0 0, 100% 0, 100% 100%)");
      // A rounded overflow clip inside the layer.
      const o3 = outer("cps-o3", 250);
      const round = document.createElement("div");
      round.setAttribute("style", "width: 100px; height: 100px; border-radius: 50%; overflow: hidden");
      o3.appendChild(round);
      inner(round, "none");
      document.body.appendChild(pad);
      return 0;
    })()`);
    const o1 = await box("#cps-o1");
    const o2 = await box("#cps-o2");
    const o3 = await box("#cps-o3");
    const s = await until(shoot, (s) => red(s.at(o1.x + 35, o1.y + 35)) && red(s.at(o2.x + 50, o2.y + 20)));
    // In both, in the triangle only, in the circle only.
    bunExpect(red(s.at(o1.x + 35, o1.y + 35))).toBe(true);
    bunExpect(white(s.at(o1.x + 12, o1.y + 12))).toBe(true);
    bunExpect(white(s.at(o1.x + 62, o1.y + 62))).toBe(true);
    // The two triangles leave the top one, apex at the centre.
    bunExpect(red(s.at(o2.x + 50, o2.y + 20))).toBe(true);
    bunExpect(white(s.at(o2.x + 20, o2.y + 50))).toBe(true);
    bunExpect(white(s.at(o2.x + 80, o2.y + 50))).toBe(true);
    bunExpect(s.count(o2, red)).toBeGreaterThan(2350); // 100 * 50 / 2 = 2500
    bunExpect(s.count(o2, red)).toBeLessThan(2650);
    // The disc of `overflow: hidden` and `border-radius: 50%`, under the
    // triangle.
    bunExpect(red(s.at(o3.x + 30, o3.y + 30))).toBe(true);
    bunExpect(white(s.at(o3.x + 4, o3.y + 4))).toBe(true);
    bunExpect(white(s.at(o3.x + 60, o3.y + 60))).toBe(true);
    await app.eval(`(document.getElementById("cps-pad").remove(), 0)`);
  }, T);
});
