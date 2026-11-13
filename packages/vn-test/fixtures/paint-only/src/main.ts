import { createApp, h } from "vue-native";

const dom = document as unknown as VnDomDocument;

const state = {
  color: "#ff0000",
  running: false,
  /** Frames the loop drew since the last `start`. */
  frames: 0,
};

function loop() {
  if (!state.running) return;
  const canvas = dom.getElementById("cv") as HTMLCanvasElement;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = state.color;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  state.frames++;
  requestAnimationFrame!(loop);
}

(globalThis as unknown as { __VN_TEST_PAINT__: unknown }).__VN_TEST_PAINT__ = {
  state,
  start(color: string) {
    state.color = color;
    state.frames = 0;
    if (!state.running) {
      state.running = true;
      requestAnimationFrame!(loop);
    }
  },
  stop() {
    state.running = false;
  },
};

const box = (left: number, w: number, hgt: number) => ({
  position: "absolute",
  left: `${left}px`,
  top: "20px",
  width: `${w}px`,
  height: `${hgt}px`,
});

createApp({
  render: () =>
    h("div", { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      h("video", {
        id: "vid",
        src: "/src/clip-tiny.mp4",
        autoplay: true,
        loop: true,
        muted: true,
        style: box(20, 160, 90),
      }),
      h("canvas", { id: "cv", width: 64, height: 64, style: box(220, 64, 64) }),
    ]),
}).mount();
