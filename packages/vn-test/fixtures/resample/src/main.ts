// The Argon client draws 1080 px avatars at 40 px, and gpui's atlas samples one mip level
// linearly, so they aliased.
import { createApp, h } from "vue-native";

const box = (left: number) => ({
  position: "absolute",
  left: `${left}px`,
  top: "20px",
  width: "40px",
  height: "40px",
});

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
      h("img", { id: "img", src: "/src/checker-1080.png", alt: "", style: box(20) }),
      h("div", {
        id: "bg",
        style: { ...box(80), backgroundImage: "url(/src/checker-1080.png)", backgroundSize: "100% 100%" },
      }),
      h("canvas", { id: "cv", width: 1080, height: 1080, style: box(140) }),
      h("img", { id: "big", src: "/src/checker-1531.png", alt: "", style: box(200) }),
      h("div", { id: "ref", style: { ...box(260), background: "#808080" } }),
    ]),
}).mount();
