import { appWindow, createApp, h, onMounted, ref } from "@skathi-native/runtime";

type Kind = "webgpu" | "2d" | "iframe";

const CLEAR = "#2050c0";
const kind = ref<Kind>("webgpu");
const state = { frames: 0, error: "" };
Object.assign(globalThis, {
  __caption: {
    state,
    use: (k: Kind) => {
      state.frames = 0;
      state.error = "";
      kind.value = k;
    },
  },
});

async function paint(canvas: HTMLCanvasElement, k: Kind) {
  canvas.width = Math.round(innerWidth * devicePixelRatio);
  canvas.height = Math.round(innerHeight * devicePixelRatio);
  if (k === "2d") {
    const g = canvas.getContext("2d");
    if (!g) return void (state.error = "no 2d context");
    g.fillStyle = CLEAR;
    g.fillRect(0, 0, canvas.width, canvas.height);
    state.frames++;
    return;
  }
  const adapter = await navigator.gpu?.requestAdapter();
  const device = await adapter?.requestDevice();
  const ctx = canvas.getContext("webgpu");
  if (!device || !ctx) return void (state.error = "no WebGPU");
  ctx.configure({ device, format: navigator.gpu.getPreferredCanvasFormat(), alphaMode: "opaque" });
  const frame = () => {
    const encoder = device.createCommandEncoder();
    encoder
      .beginRenderPass({
        colorAttachments: [
          {
            view: ctx.getCurrentTexture().createView(),
            clearValue: { r: 0x20 / 255, g: 0x50 / 255, b: 0xc0 / 255, a: 1 },
            loadOp: "clear",
            storeOp: "store",
          },
        ],
      })
      .end();
    device.queue.submit([encoder.finish()]);
    state.frames++;
    if (kind.value === "webgpu" && state.frames < 120) requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

const FILL = "position: absolute; inset: 0; width: 100%; height: 100%; border: 0";

const Surface = {
  props: { kind: { type: String, required: true } },
  setup(props: { kind: Kind }) {
    const el = ref<HTMLCanvasElement | null>(null);
    onMounted(() => {
      if (el.value) void paint(el.value, props.kind).catch((e: unknown) => (state.error = String(e)));
    });
    return () =>
      props.kind === "iframe"
        ? h("iframe", {
            id: "surface",
            style: FILL,
            srcdoc: `<body style="margin: 0; background: ${CLEAR}"></body>`,
            onLoad: () => state.frames++,
          })
        : h("canvas", { ref: el, id: "surface", style: FILL });
  },
};

createApp({
  setup() {
    onMounted(() => appWindow.setTitleBarOverlay({ symbolColor: "#ffffff" }));
    return () =>
      h("main", { style: "position: absolute; inset: 0; background: #ff00ff" }, [
        h(Surface, { kind: kind.value, key: kind.value }),
        h("header", {
          id: "strip",
          style:
            "position: absolute; top: env(titlebar-area-y, 0px); left: env(titlebar-area-x, 0px); " +
            "width: env(titlebar-area-width, 100%); height: env(titlebar-area-height, 40px); app-region: drag",
        }),
      ]);
  },
}).mount();
