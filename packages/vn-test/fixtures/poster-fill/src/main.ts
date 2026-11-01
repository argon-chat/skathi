// A fixture for packages/vn-test/specs/poster-fill.spec.ts: a replaced
// element laid over its box. Argon's media grid (AttachmentImageGrid.vue) is
// flex rows of fixed height holding cells of fixed width, and a video cell is
// VideoAttachment.vue: a `relative; overflow: hidden; width: 100%; height:
// 100%` box with a grey background, and over it a poster `<img>` (or a
// thumbhash `<canvas>`, or a `<video>` still) at `position: absolute; inset:
// 0; width: 100%; height: 100%; object-fit: cover`. In Chromium the poster
// fills the cell and is cropped; on vue-native it sat at the top at its own
// aspect ratio, with the grey below it.
//
// Every source is three bands, green | red | blue, across its long side, so
// `cover` puts red at the centre of the cell and fills the cell to its edges.
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h, onMounted, ref } from "vue-native";
import "./poster.css";

/** Three equal bands across the long side of a w x h picture. */
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

/** The same bands on a canvas, as the thumbhash placeholder is drawn. */
function paint(canvas: HTMLCanvasElement) {
  const ctx = canvas.getContext("2d")!;
  const step = canvas.width / 3;
  ["#00ff00", "#ff0000", "#0000ff"].forEach((fill, i) => {
    ctx.fillStyle = fill;
    ctx.fillRect(i * step, 0, step, canvas.height);
  });
}

/** One grid cell: a VideoAttachment box with `poster` laid over it. */
const cell = (id: string, width: number, poster: ReturnType<typeof h>, fit: "cover" | "contain" = "cover") =>
  h("div", { class: "grid-cell", style: { width: `${width}px` } }, [
    h("div", { id, class: fit === "contain" ? "video-attachment va-contain" : "video-attachment" }, [
      poster,
      h("span", { class: "va-play" }),
    ]),
  ]);

/**
 * VideoAttachment.vue as it renders a poster: the thumbhash canvas while the
 * poster loads, the `<img>` at `opacity: 0` until its `load`, then `.visible`
 * (an opacity transition).
 */
const Attachment = {
  props: { id: String, src: String },
  setup(props: { id: string; src?: string }) {
    const loaded = ref(false);
    const thumb = ref<HTMLCanvasElement | null>(null);
    // renderThumbHash(): the canvas is resized from script, then filled.
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
            h("video", { id: "p-video", class: "va-poster", src: "/src/clip-tiny.mp4", muted: true, preload: "metadata" }),
          ),
        ]),
        h("div", { class: "grid-row", style: { height: "120px" } }, [
          // The single-attachment wrapper's `fit="contain"`: letterboxed,
          // centred, grey above and below.
          cell("c-contain", 200, h("img", { id: "p-contain", class: "va-poster", src: WIDE, alt: "" }), "contain"),
          // Raster posters, 1280x720 and 720x1280: drawn at a fraction of
          // their size, as a CDN poster is.
          h("div", { class: "grid-cell", style: { width: "100px" } }, [h(Attachment, { id: "c-png-wide", src: "/src/wide.png" })]),
          h("div", { class: "grid-cell", style: { width: "220px" } }, [h(Attachment, { id: "c-png-tall", src: "/src/tall.png" })]),
        ]),
        h("div", { class: "grid-row", style: { height: "120px" } }, [
          // No poster yet: the blurred thumbhash canvas is what shows.
          h("div", { class: "grid-cell", style: { width: "100px" } }, [h(Attachment, { id: "c-thumb" })]),
          // The same picture IN FLOW at `width: 100%; height: 100%`, in a
          // block box and in a flex box.
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
