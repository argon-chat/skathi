// A fixture for packages/vn-test/specs/resample.spec.ts: images drawn far
// below their own size (m26 client sweep #22). The Argon client draws 1080 px
// avatars at 40 px, and gpui's atlas samples one mip level linearly, so they
// aliased. The host now draws such an image from a filtered copy
// (crates/vn-tree/src/resample.rs).
//
// Every probe is a one-pixel black/white checkerboard shown at 40 x 40 CSS px
// on white. Filtered, it is a flat 50% grey, the colour of `#ref`. Unfiltered,
// the sampler lands on texel centres and it is black and white noise.
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";

const box = (left: number) => ({
  position: "absolute",
  left: `${left}px`,
  top: "20px",
  width: "40px",
  height: "40px",
});

/** A 1080 px checkerboard drawn into `#cv` once: a still canvas. */
function drawCanvas() {
  const canvas = document.getElementById("cv") as HTMLCanvasElement;
  const ctx = canvas.getContext("2d")!;
  const n = 1080;
  const data = ctx.createImageData(n, n);
  for (let y = 0; y < n; y++)
    for (let x = 0; x < n; x++) {
      const o = (y * n + x) * 4;
      const c = (x + y) % 2 === 0 ? 255 : 0;
      data.data[o] = c;
      data.data[o + 1] = c;
      data.data[o + 2] = c;
      data.data[o + 3] = 255;
    }
  ctx.putImageData(data, 0, 0);
}

createApp({
  mounted: drawCanvas,
  render: () =>
    h("div", { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      // An <img>, small enough that its first copy is made while painting.
      h("img", { id: "img", src: "/src/checker-1080.png", alt: "", style: box(20) }),
      // CSS `background-image`, through the same paint path.
      h("div", {
        id: "bg",
        style: { ...box(80), backgroundImage: "url(/src/checker-1080.png)", backgroundSize: "100% 100%" },
      }),
      // A still <canvas>, its 1080 px bitmap shown at 40 px.
      h("canvas", { id: "cv", width: 1080, height: 1080, style: box(140) }),
      // An <img> past the painting thread's budget: its copy is made on the
      // resampling thread and the frame tick repaints when it lands. An odd
      // size: at a whole or half ratio (1600:40) the atlas samples halfway
      // between texels, which averages the checkerboard by luck.
      h("img", { id: "big", src: "/src/checker-1531.png", alt: "", style: box(200) }),
      // The reference: what a filtered downscale of the checkerboard is.
      h("div", { id: "ref", style: { ...box(260), background: "#808080" } }),
    ]),
}).mount();
