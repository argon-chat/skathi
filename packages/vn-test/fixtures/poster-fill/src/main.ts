import { createApp, h, onMounted, ref } from "vue-native";
import "./poster.css";

function bands(w: number, h: number): string {
  const wide = w >= h;
  const step = (wide ? w : h) / 3;
  const rect = (i: number, fill: string) =>
    wide
      ? `<rect x='${i * step}' y='0' width='${step}' height='${h}' fill='${fill}'/>`
      : `<rect x='0' y='${i * step}' width='${w}' height='${step}' fill='${fill}'/>`;
  const svg =
    `<svg xmlns='http://www.w3.org/2000/svg' width='${w}' height='${h}'>` +
    rect(0, "%2300ff00") +
    rect(1, "%23ff0000") +
    rect(2, "%230000ff") +
    `</svg>`;
  return `data:image/svg+xml,${svg}`;
}

const WIDE = bands(160, 90);
const TALL = bands(90, 160);

function paint(canvas: HTMLCanvasElement) {
  const ctx = canvas.getContext("2d")!;
  const step = canvas.width / 3;
  ["#00ff00", "#ff0000", "#0000ff"].forEach((fill, i) => {
    ctx.fillStyle = fill;
    ctx.fillRect(i * step, 0, step, canvas.height);
  });
}

const cell = (id: string, width: number, poster: ReturnType<typeof h>, fit: "cover" | "contain" = "cover") =>
  h("div", { class: "grid-cell", style: { width: `${width}px` } }, [
    h("div", { id, class: fit === "contain" ? "video-attachment va-contain" : "video-attachment" }, [
      poster,
      h("span", { class: "va-play" }),
    ]),
  ]);

const Attachment = {
  props: { id: String, src: String },
  setup(props: { id: string; src?: string }) {
    const loaded = ref(false);
    const thumb = ref<HTMLCanvasElement | null>(null);
    onMounted(() => {
      const canvas = thumb.value;
      if (!canvas) return;
      canvas.width = 32;
      canvas.height = 18;
      const ctx = canvas.getContext("2d")!;
      const data = ctx.createImageData(32, 18);
      for (let y = 0; y < 18; y++)
        for (let x = 0; x < 32; x++) {
          const o = (y * 32 + x) * 4;
          data.data.set(x < 11 ? [0, 255, 0, 255] : x < 21 ? [255, 0, 0, 255] : [0, 0, 255, 255], o);
        }
      ctx.putImageData(data, 0, 0);
    });
    return () =>
      h("div", { id: props.id, class: "video-attachment" }, [
        loaded.value ? null : h("canvas", { id: `t-${props.id}`, ref: thumb, class: "va-placeholder" }),
        props.src
          ? h("img", {
              id: `p-${props.id}`,
              class: loaded.value ? "va-poster va-fade visible" : "va-poster va-fade",
              src: props.src,
              alt: "",
              crossorigin: "anonymous",
              draggable: "false",
              onLoad: () => (loaded.value = true),
            })
          : null,
        h("span", { class: "va-play" }),
      ]);
  },
};

const Grid = {
  setup() {
    onMounted(() => paint(document.getElementById("p-canvas") as HTMLCanvasElement));
    return () =>
      h("div", { class: "image-grid" }, [
        h("div", { class: "grid-row", style: { height: "120px" } }, [
          cell("c-wide", 100, h("img", { id: "p-wide", class: "va-poster", src: WIDE, alt: "" })),
          cell("c-tall", 220, h("img", { id: "p-tall", class: "va-poster", src: TALL, alt: "" })),
          cell("c-canvas", 100, h("canvas", { id: "p-canvas", class: "va-poster", width: 160, height: 90 })),
          cell(
            "c-video",
            100,
            h("video", {
              id: "p-video",
              class: "va-poster",
              src: "/src/clip-tiny.mp4",
              muted: true,
              preload: "metadata",
            }),
          ),
        ]),
        h("div", { class: "grid-row", style: { height: "120px" } }, [
          cell("c-contain", 200, h("img", { id: "p-contain", class: "va-poster", src: WIDE, alt: "" }), "contain"),
          h("div", { class: "grid-cell", style: { width: "100px" } }, [
            h(Attachment, { id: "c-png-wide", src: "/src/wide.png" }),
          ]),
          h("div", { class: "grid-cell", style: { width: "220px" } }, [
            h(Attachment, { id: "c-png-tall", src: "/src/tall.png" }),
          ]),
        ]),
        h("div", { class: "grid-row", style: { height: "120px" } }, [
          h("div", { class: "grid-cell", style: { width: "100px" } }, [h(Attachment, { id: "c-thumb" })]),
          h("div", { id: "c-inflow-block", class: "inflow" }, [
            h("img", { id: "p-inflow-block", class: "fill-img", src: "/src/wide.png", alt: "" }),
          ]),
          h("div", { id: "c-inflow-flex", class: "inflow inflow-flex" }, [
            h("img", { id: "p-inflow-flex", class: "fill-img", src: "/src/wide.png", alt: "" }),
          ]),
        ]),
      ]);
  },
};

createApp({ render: () => h("div", { id: "root" }, [h(Grid)]) }).mount();
