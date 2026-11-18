// Writes ../assets: hand-authored Lottie animations, the <video> clip (from templates/weather's
// textures) and the <iframe> page. `bun apps/elements/tools/gen-assets.ts`; needs ffmpeg on PATH.
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { gunzipSync, gzipSync } from "node:zlib";
import {
  A,
  doc,
  ellipse,
  fill,
  gradient,
  group,
  heart,
  hex,
  layer,
  path,
  rect,
  star,
  stroke,
  trim,
} from "./lottie-kit";

const REPO = resolve(import.meta.dir, "../../..");
const FFMPEG = process.env.FFMPEG ?? "ffmpeg";
const app = resolve(import.meta.dir, "..");
const assets = join(app, "assets");
const stickers = join(assets, "stickers");
mkdirSync(stickers, { recursive: true });

const PINK = "#f7a8d0";
const PINK_D = "#ec7fb6";
const LILAC = "#c9a7f5";
const LILAC_D = "#a283ea";
const BLUE = "#9fcdf5";
const BLUE_D = "#6eaee8";
const PEACH = "#ffd3b0";
const INK = "#2a2233";
const WHITE = "#ffffff";

const write = (file: string, json: object) => writeFileSync(file, JSON.stringify(json));

// ---------------------------------------------------------------------------------------------
// <lottie>: the hero, 3 s at 60 fps.
{
  const F = 180;
  const C = [256, 256];
  const beat = A([
    [0, [100, 100]],
    [12, [114, 114]],
    [26, [100, 100]],
    [38, [108, 108]],
    [56, [100, 100]],
    [90, [100, 100]],
    [102, [114, 114]],
    [116, [100, 100]],
    [128, [108, 108]],
    [146, [100, 100]],
    [180, [100, 100]],
  ]);
  const twinkle = (x: number, y: number, at: number, color: string) =>
    layer(
      "sparkle",
      [
        group([star(4, 20, 5, 0), fill(hex(color))], {
          r: A(
            [
              [0, 0],
              [F, 90],
            ],
            true,
          ),
        }),
      ],
      F,
      {
        p: [x, y],
        s: A([
          [0, [0, 0]],
          [at, [0, 0]],
          [at + 20, [100, 100]],
          [at + 40, [0, 0]],
          [F, [0, 0]],
        ]),
      },
    );
  const lottie = doc("orbit", F, [
    layer(
      "glow",
      [
        group([
          ellipse([330, 330]),
          fill(
            hex(LILAC),
            A([
              [0, 10],
              [90, 18],
              [180, 10],
            ]),
          ),
        ]),
      ],
      F,
      { p: C },
    ),
    layer(
      "outer ring",
      [
        group([ellipse([420, 420]), stroke(hex(WHITE), 2, 18)], {}, "track"),
        group([ellipse([420, 420]), stroke(hex(BLUE), 7), trim(0, 26, 0)], {}, "a"),
        group([ellipse([420, 420]), stroke(hex(PINK), 7), trim(50, 66, 0)], {}, "b"),
      ],
      F,
      {
        p: C,
        r: A(
          [
            [0, 0],
            [F, -360],
          ],
          true,
        ),
      },
    ),
    layer(
      "inner ring",
      [
        group([
          ellipse([316, 316]),
          stroke(hex(LILAC), 11),
          trim(
            A([
              [0, 0],
              [60, 0],
              [150, 100],
              [180, 100],
            ]),
            A([
              [0, 0],
              [90, 100],
              [180, 100],
            ]),
          ),
        ]),
      ],
      F,
      {
        p: C,
        r: A(
          [
            [0, -90],
            [F, 270],
          ],
          true,
        ),
      },
    ),
    layer(
      "orbit",
      [
        group([ellipse([24, 24], [232, 0]), fill(hex(PINK))]),
        group([ellipse([18, 18], [-116, 201]), fill(hex(LILAC))]),
        group([ellipse([20, 20], [-116, -201]), fill(hex(BLUE))]),
      ],
      F,
      {
        p: C,
        r: A(
          [
            [0, 0],
            [F, 360],
          ],
          true,
        ),
      },
    ),
    layer(
      "heart",
      [
        group([
          heart(150),
          gradient(
            [
              [0, PINK],
              [1, PINK_D],
            ],
            [0, -60],
            [0, 70],
          ),
        ]),
        group([ellipse([34, 22], [-36, -42]), fill(hex(WHITE), 55)], { r: -30 }),
      ],
      F,
      { p: [256, 262], s: beat },
    ),
    twinkle(392, 112, 10, LILAC),
    twinkle(108, 150, 60, BLUE),
    twinkle(410, 400, 105, PINK),
    twinkle(96, 388, 140, LILAC),
  ]);
  write(join(assets, "orbit.json"), lottie);
}

// ---------------------------------------------------------------------------------------------
// <sk-sticker>: 2 s at 60 fps each, so every one of them loops inside an 8 s clip.
const F = 120;
const C = [256, 256];

write(
  join(stickers, "heart.json"),
  doc("heart", F, [
    layer("wave", [group([ellipse([260, 260]), stroke(hex(PINK), 10)])], F, {
      p: C,
      s: A(
        [
          [0, [70, 70]],
          [60, [150, 150]],
          [61, [70, 70]],
          [120, [150, 150]],
        ],
        true,
      ),
      o: A(
        [
          [0, 70],
          [60, 0],
          [61, 70],
          [120, 0],
        ],
        true,
      ),
    }),
    layer(
      "heart",
      [
        group([
          heart(300),
          gradient(
            [
              [0, PINK],
              [1, PINK_D],
            ],
            [0, -120],
            [0, 140],
          ),
        ]),
        group([ellipse([60, 40], [-70, -84]), fill(hex(WHITE), 55)], { r: -30 }),
      ],
      F,
      {
        p: [256, 266],
        s: A([
          [0, [100, 100]],
          [10, [116, 116]],
          [24, [100, 100]],
          [34, [108, 108]],
          [50, [100, 100]],
          [60, [100, 100]],
          [70, [116, 116]],
          [84, [100, 100]],
          [94, [108, 108]],
          [110, [100, 100]],
          [120, [100, 100]],
        ]),
      },
    ),
  ]),
);

{
  const y = (top: number) =>
    A([
      [0, [256, top]],
      [27, [256, 404]],
      [30, [256, 410]],
      [33, [256, 404]],
      [60, [256, top]],
      [87, [256, 404]],
      [90, [256, 410]],
      [93, [256, 404]],
      [120, [256, top]],
    ]);
  write(
    join(stickers, "bounce.json"),
    doc("bounce", F, [
      layer("shadow", [group([ellipse([150, 30]), fill(hex(INK), 45)])], F, {
        p: [256, 420],
        s: A([
          [0, [50, 50]],
          [30, [110, 110]],
          [60, [50, 50]],
          [90, [110, 110]],
          [120, [50, 50]],
        ]),
      }),
      layer(
        "ball",
        [
          group([
            ellipse([170, 170]),
            gradient(
              [
                [0, BLUE],
                [1, BLUE_D],
              ],
              [-40, -80],
              [40, 85],
            ),
          ]),
          group([ellipse([44, 30], [-38, -44]), fill(hex(WHITE), 60)], { r: -35 }),
        ],
        F,
        {
          p: y(190),
          a: [0, 85],
          s: A([
            [0, [100, 100]],
            [26, [94, 108]],
            [30, [124, 78]],
            [36, [96, 106]],
            [60, [100, 100]],
            [86, [94, 108]],
            [90, [124, 78]],
            [96, [96, 106]],
            [120, [100, 100]],
          ]),
        },
      ),
    ]),
  );
}

{
  const dot = (x: number, delay: number, color: string) =>
    layer("dot", [group([ellipse([84, 84]), fill(hex(color))])], F, {
      p: A([
        [0, [x, 256]],
        [delay, [x, 256]],
        [delay + 16, [x, 196]],
        [delay + 32, [x, 256]],
        [delay + 60, [x, 256]],
        [delay + 76, [x, 196]],
        [delay + 92, [x, 256]],
        [120, [x, 256]],
      ]),
    });
  write(
    join(stickers, "dots.json"),
    doc("dots", F, [
      layer("bubble", [group([rect([400, 220], 110), fill(hex("#3a3046"))])], F, { p: C }),
      dot(146, 0, PINK),
      dot(256, 8, LILAC),
      dot(366, 16, BLUE),
    ]),
  );
}

{
  const petals = [0, 60, 120, 180, 240, 300].map((r, i) =>
    group([ellipse([110, 190], [0, -112]), fill(hex(i % 2 ? LILAC : PINK))], { r }),
  );
  write(
    join(stickers, "flower.json"),
    doc("flower", F, [
      layer("petals", petals, F, {
        p: C,
        r: A(
          [
            [0, 0],
            [F, 120],
          ],
          true,
        ),
      }),
      layer(
        "centre",
        [
          group([
            ellipse([120, 120]),
            gradient(
              [
                [0, PEACH],
                [1, "#ffb98a"],
              ],
              [0, -60],
              [0, 60],
            ),
          ]),
        ],
        F,
        {
          p: C,
          s: A([
            [0, [100, 100]],
            [60, [116, 116]],
            [120, [100, 100]],
          ]),
        },
      ),
    ]),
  );
}

{
  const small = (x: number, y: number, at: number, color: string, size: number) =>
    layer("small", [group([star(4, size, size / 4), fill(hex(color))])], F, {
      p: [x, y],
      s: A([
        [0, [0, 0]],
        [at, [0, 0]],
        [at + 18, [100, 100]],
        [at + 36, [0, 0]],
        [F, [0, 0]],
      ]),
    });
  write(
    join(stickers, "sparkle.json"),
    doc("sparkle", F, [
      layer(
        "big",
        [
          group([
            star(4, 190, 46),
            gradient(
              [
                [0, WHITE],
                [0.45, LILAC],
                [1, LILAC_D],
              ],
              [0, 0],
              [190, 0],
              2,
            ),
          ]),
        ],
        F,
        {
          p: C,
          r: A(
            [
              [0, 0],
              [F, 90],
            ],
            true,
          ),
          s: A([
            [0, [92, 92]],
            [60, [104, 104]],
            [120, [92, 92]],
          ]),
        },
      ),
      small(400, 120, 6, PINK, 50),
      small(120, 380, 46, BLUE, 42),
      small(404, 398, 80, PINK, 34),
    ]),
  );
}

write(
  join(stickers, "loader.json"),
  doc("loader", F, [
    layer("a", [group([ellipse([400, 400]), stroke(hex(PINK), 30), trim(0, 30)])], F, {
      p: C,
      r: A(
        [
          [0, 0],
          [F, 360],
        ],
        true,
      ),
    }),
    layer("b", [group([ellipse([290, 290]), stroke(hex(LILAC), 30), trim(0, 40)])], F, {
      p: C,
      r: A(
        [
          [0, 120],
          [F, -240],
        ],
        true,
      ),
    }),
    layer("c", [group([ellipse([180, 180]), stroke(hex(BLUE), 30), trim(0, 50)])], F, {
      p: C,
      r: A(
        [
          [0, 240],
          [F, 960],
        ],
        true,
      ),
    }),
  ]),
);

{
  const eye = (x: number) =>
    group([ellipse([38, 54]), fill(hex(INK))], {
      p: [x, -30],
      s: A([
        [0, [100, 100]],
        [64, [100, 100]],
        [70, [100, 8]],
        [76, [100, 100]],
        [120, [100, 100]],
      ]),
    });
  write(
    join(stickers, "face.json"),
    doc("face", F, [
      layer(
        "face",
        [
          group([
            ellipse([380, 380]),
            gradient(
              [
                [0, "#ffd0e6"],
                [1, PINK],
              ],
              [0, -190],
              [0, 190],
            ),
          ]),
          group([ellipse([70, 40], [-110, 50]), fill(hex(PINK_D), 70)]),
          group([ellipse([70, 40], [110, 50]), fill(hex(PINK_D), 70)]),
          eye(-62),
          eye(62),
          group([
            path(
              [
                [-60, 40],
                [60, 40],
              ],
              [
                [0, 0],
                [30, 50],
              ],
              [
                [-30, 50],
                [0, 0],
              ],
              false,
            ),
            stroke(hex(INK), 18),
          ]),
        ],
        F,
        {
          p: [256, 376],
          a: [0, 120],
          r: A([
            [0, -7],
            [60, 7],
            [120, -7],
          ]),
        },
      ),
    ]),
  );
}

{
  const colors = [PINK, PINK, LILAC, BLUE, BLUE];
  const bars = [0, 1, 2, 3, 4].map((i) => {
    const x = -160 + i * 80;
    const h = (n: number) => [56, n];
    const o = i * 9;
    return group(
      [
        rect(
          A([
            [0, h(90)],
            [12 + o, h(300)],
            [30 + o, h(130)],
            [48 + o, h(250)],
            [72 + o, h(110)],
            [96, h(220)],
            [120, h(90)],
          ]),
          28,
        ),
        fill(hex(colors[i]!)),
      ],
      { p: [x, 0] },
    );
  });
  write(join(stickers, "bars.json"), doc("bars", F, [layer("bars", bars, F, { p: C })]));
}

// The CC0 star from apps/hello/assets (see its README for provenance).
// Recoloured to lilac; the shape and the motion are untouched.
{
  const star = JSON.parse(gunzipSync(readFileSync(join(REPO, "apps/hello/assets/sticker.tgs"))).toString("utf8"));
  const recolor = (node: unknown): void => {
    if (Array.isArray(node)) return node.forEach(recolor);
    if (!node || typeof node !== "object") return;
    const o = node as Record<string, unknown>;
    if (o.ty === "fl" && o.c && typeof o.c === "object") (o.c as { k: number[] }).k = hex(LILAC);
    Object.values(o).forEach(recolor);
  };
  recolor(star);
  writeFileSync(join(stickers, "star.tgs"), gzipSync(JSON.stringify(star)));
}

// ---------------------------------------------------------------------------------------------
// <video>: dawn, day, dusk, night and back, a 9 s loop made from templates/weather's textures.
{
  const tex = join(REPO, "templates/weather/assets/textures");
  const tmp = join(tmpdir(), `sk-elements-video-${process.pid}`);
  mkdirSync(tmp, { recursive: true });
  const ff = (args: string[]) => {
    const r = spawnSync(FFMPEG, ["-hide_banner", "-loglevel", "error", "-y", ...args], { encoding: "utf8" });
    if (r.status !== 0) throw new Error(r.stderr);
  };
  const phases = ["dawn", "day", "dusk", "night"];
  for (const p of phases) {
    ff([
      "-i",
      join(tex, `sky-${p}.jpg`),
      "-i",
      join(tex, `land-${p}.webp`),
      "-filter_complex",
      "[0][1]overlay",
      "-frames:v",
      "1",
      join(tmp, `${p}.png`),
    ]);
  }
  const order = [...phases, "dawn"];
  const inputs = order.flatMap((p) => ["-loop", "1", "-framerate", "30", "-t", "3", "-i", join(tmp, `${p}.png`)]);
  const fades = [1, 2, 3, 4]
    .map(
      (k) =>
        `${k === 1 ? "[0]" : `[x${k - 1}]`}[${k}]xfade=transition=fade:duration=0.75:offset=${(k * 2.25).toFixed(2)}[x${k}]`,
    )
    .join(";");
  const graph =
    `${fades};[x4]trim=start=1:end=10,setpts=PTS-STARTPTS,` +
    `crop=1440:810:x='73+73*sin(2*PI*t/9)':y=150,scale=1280:720:flags=lanczos,fps=30,format=yuv420p[v]`;
  ff([
    ...inputs,
    "-filter_complex",
    graph,
    "-map",
    "[v]",
    "-c:v",
    "libx264",
    "-profile:v",
    "high",
    "-preset",
    "slow",
    "-crf",
    "26",
    "-g",
    "30",
    "-movflags",
    "+faststart",
    "-an",
    join(assets, "daylight.mp4"),
  ]);
  ff([
    "-i",
    join(tmp, "dusk.png"),
    "-vf",
    "crop=1440:810:73:150,scale=1280:720",
    "-frames:v",
    "1",
    join(assets, "daylight-poster.jpg"),
  ]);
  rmSync(tmp, { recursive: true, force: true });
}

// ---------------------------------------------------------------------------------------------
// <iframe>: a page from the app's files. It takes an accent from the app and answers.
writeFileSync(
  join(assets, "page.html"),
  `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width" />
<title>Field notes</title>
<style>
  :root { --accent: ${PINK}; --accent-2: ${LILAC}; color-scheme: dark; }
  * { box-sizing: border-box; }
  html, body { margin: 0; overflow: hidden; background: #1c1a24; color: #e3dcec; font: 16px/1.6 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; }
  header { position: relative; padding: 46px 48px 38px; overflow: hidden;
    background: linear-gradient(135deg, color-mix(in oklab, var(--accent) 34%, #1c1a24), color-mix(in oklab, var(--accent-2) 26%, #1c1a24));
    transition: background 600ms ease; }
  header::after { content: ""; position: absolute; right: -60px; top: -80px; width: 280px; height: 280px; border-radius: 50%;
    background: radial-gradient(circle, color-mix(in oklab, var(--accent) 70%, transparent), transparent 70%); opacity: .55; transition: background 600ms ease; }
  .kicker { font-size: 12px; letter-spacing: .14em; text-transform: uppercase; color: var(--accent); font-weight: 600; transition: color 600ms ease; }
  h1 { margin: 8px 0 6px; font: 600 38px/1.15 "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif; color: #fff; }
  .meta { font-size: 14px; color: #b9b1c6; }
  main { padding: 30px 48px 40px; max-width: 760px; }
  p { margin: 0 0 16px; color: #c9c1d6; }
  .cards { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; margin: 24px 0; }
  .card { border-radius: 12px; padding: 14px 16px; background: #24212e; border: 1px solid #332e3f; }
  .card b { display: block; font-size: 22px; color: #fff; }
  .card span { font-size: 13px; color: #a69eb4; }
  .bar { height: 6px; border-radius: 3px; background: #332e3f; margin-top: 10px; overflow: hidden; }
  .bar i { display: block; height: 100%; border-radius: 3px; background: var(--accent); transition: background 600ms ease, width 600ms ease; }
  a { color: var(--accent); transition: color 600ms ease; }
</style>
</head>
<body>
<header>
  <div class="kicker">Field notes · WebView2</div>
  <h1>The lilac hour</h1>
  <div class="meta">An HTML page from the app's files, running in the system webview.</div>
</header>
<main>
  <p>Just after sunset the valley turns violet. The fog settles between the ridges, the first stars come out over the far peaks, and the heather on the slope keeps the last of the light.</p>
  <div class="cards">
    <div class="card"><b>19:42</b><span>Sunset</span><div class="bar"><i style="width:72%"></i></div></div>
    <div class="card"><b>8°C</b><span>Clear, light wind</span><div class="bar"><i style="width:38%"></i></div></div>
    <div class="card"><b>2.4 km</b><span>To the ridge</span><div class="bar"><i style="width:56%"></i></div></div>
  </div>
  <p>This document is plain <a href="#">HTML and CSS</a>. The app above it is drawn by Skathi, and the two talk through <code>postMessage</code>: the accent you pick in the app's panel arrives here, and the page answers.</p>
  <p>Bring a warm layer; the path along the ridge is easy but long, and the light goes quickly once the sun is down.</p>
</main>
<script>
  const names = { "${PINK}": "pink", "${LILAC}": "lilac", "${BLUE}": "blue" };
  window.addEventListener("message", (e) => {
    const accent = e.data && e.data.accent;
    if (!accent) return;
    document.documentElement.style.setProperty("--accent", accent);
    document.documentElement.style.setProperty("--accent-2", e.data.second || accent);
    window.parent.postMessage({ applied: names[accent] || accent }, "*");
  });
</script>
</body>
</html>
`,
);

spawnSync("bun", ["fmt", assets], { cwd: REPO, stdio: "inherit" });
console.log("assets written to", assets);
