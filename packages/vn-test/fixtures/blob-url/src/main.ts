// A fixture for packages/vn-test/specs/blob-url.spec.ts (m25 desktop track B,
// research/reports/m25/desktop-b-impl.md): `URL.createObjectURL(blob)` into an
// <img>, the browser's ordinary preview idiom, on the real host.
//
// Three images:
//
// - `#live`: a red 8×8 PNG built from bytes in memory, behind a blob: URL
//   that stays alive. vn-tree must resolve it through redjs's store and paint
//   red.
// - `#revoked`: the same bytes behind a URL that is revoked BEFORE the <img>
//   ever sees it. A browser shows a broken image; so must this, with one
//   advisory that says the URL is not in the store.
// - `#sliced`: the PNG behind a slice of a larger Blob (junk in front), so
//   the URL names a window, not the whole of its bytes.
//
// `__VN_TEST_BLOB__` publishes what the spec reads back over eval (GLOBAL
// scope, as in the other fixtures): the URLs, how a fetch() of the live one
// settled, and what redjs's own image loaders made of the same URLs: an
// offscreen `new Image()` (`src` = blob:) and `createImageBitmap` of a URL
// string and of a Blob. Those go through redjs's store, not vn-tree's walk.
import { createApp, h, onMounted, ref } from "vue-native";

/** The slice of redjs's offscreen image and bitmap API this fixture uses.
 *  The app tier-1 types do not declare it, since no app has needed it yet. */
type OffscreenImage = {
  onload: (() => void) | null;
  onerror: (() => void) | null;
  src: string;
  naturalWidth: number;
  naturalHeight: number;
  complete: boolean;
};
const images = globalThis as unknown as {
  Image: new () => OffscreenImage;
  createImageBitmap(source: unknown): Promise<{ width: number; height: number }>;
};

const RED_PNG = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAEklEQVR4nGP4z8DwHx9mGBkKAMLXf4EvceABAAAAAElFTkSuQmCC";

function pngBytes(): Uint8Array {
  const bin = atob(RED_PNG);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

const png = pngBytes();
const live = URL.createObjectURL(new Blob([png], { type: "image/png" }));
const revoked = URL.createObjectURL(new Blob([png], { type: "image/png" }));
URL.revokeObjectURL(revoked);
const junk = new Uint8Array(37).fill(7);
const sliced = URL.createObjectURL(new Blob([junk, png]).slice(junk.length));

const results: Record<string, string> = {};
(globalThis as unknown as { __VN_TEST_BLOB__: unknown }).__VN_TEST_BLOB__ = { live, revoked, sliced, results };

/** `new Image()` with `src` = url: "load|w|h|complete", or "error". */
function loadImage(url: string): Promise<string> {
  return new Promise((done) => {
    const i = new images.Image();
    i.onload = () => done(`load|${i.naturalWidth}|${i.naturalHeight}|${i.complete}`);
    i.onerror = () => done("error");
    i.src = url;
  });
}

/** `createImageBitmap(source)`: "w|h", or the rejection's name. */
async function bitmap(source: unknown): Promise<string> {
  try {
    const b = await images.createImageBitmap(source);
    return `${b.width}|${b.height}`;
  } catch (e) {
    return `ERR ${(e as { name?: string }).name}`;
  }
}

const shown = ref(false);

const img = (id: string, src: string) =>
  h("img", { id, src, alt: id, style: { width: "96px", height: "96px" } });

createApp({
  setup() {
    onMounted(async () => {
      try {
        const r = await fetch!(live);
        const b = new Uint8Array(await r.arrayBuffer!());
        results.fetch = `${r.status}|${r.headers.get("content-type")}|${b.length}|${b[1] === 0x50}`;
      } catch (e) {
        results.fetch = `ERR ${(e as { name?: string }).name}`;
      }
      results.image = await loadImage(live);
      results.imageSliced = await loadImage(sliced);
      results.imageRevoked = await loadImage(revoked);
      results.bitmapUrl = await bitmap(live);
      results.bitmapRevoked = await bitmap(revoked);
      results.bitmapBlob = await bitmap(new Blob([png], { type: "image/png" }));
      shown.value = true;
    });
    return () =>
      h("div", { id: "root", style: { padding: "12px", display: "flex", gap: "12px", background: "#ffffff" } }, [
        h("div", { id: "status" }, shown.value ? "ready" : "loading"),
        img("live", live),
        img("revoked", revoked),
        img("sliced", sliced),
      ]);
  },
}).mount();
