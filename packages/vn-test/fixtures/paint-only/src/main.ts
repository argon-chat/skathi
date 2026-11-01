// A fixture for packages/vn-test/specs/paint-only.spec.ts: a picture that
// changes in a box that does not move is shown by a present of the frame
// already drawn, not by a build (docs/ARCHITECTURE.md decision 28,
// crates/vn-tree/src/live.rs).
//
// `#vid` plays apps/hello's 2 s, 160x90 clip in a loop, muted. `#cv` is a
// 64x64 canvas that `__VN_TEST_PAINT__.start(color)` fills with `color` on
// every animation frame until `stop()`. The colour can change while it runs
// (`state.color`), which is what the spec reads back from the screen.
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";

const state = {
  color: "#ff0000",
  running: false,
  /** Frames the loop drew since the last `start`. */
  frames: 0,
};

function loop() {
  if (!state.running) return;
  const canvas = document.getElementById("cv") as HTMLCanvasElement;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = state.color;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  state.frames++;
  requestAnimationFrame(loop);
}

(globalThis as any).__VN_TEST_PAINT__ = {
  state,
  start(color: string) {
    state.color = color;
    state.frames = 0;
    if (!state.running) {
      state.running = true;
      requestAnimationFrame(loop);
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
      h("video", { id: "vid", src: "/src/clip-tiny.mp4", autoplay: true, loop: true, muted: true, style: box(20, 160, 90) }),
      h("canvas", { id: "cv", width: 64, height: 64, style: box(220, 64, 64) }),
    ]),
}).mount();
