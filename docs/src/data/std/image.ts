import { NO, OK, importLine, section } from "./shared";

const SAME_EVERYWHERE =
  "The codecs are the same Rust code on every platform, so a file decodes to the same pixels on Windows, macOS and Linux.";

const LIMITS =
  "A file is untrusted input. Images up to 16384 pixels a side and 67,108,864 pixels in all are decoded; the dimensions are checked before the pixels are allocated, so a file that claims more fails with `EncodingError` instead of taking the memory. Data longer than 256 MiB is refused.";

export const image = section("skathi:image", [
  {
    slug: "image",
    name: "Image",
    kind: "class",
    summary: "Decoded pixels in native memory, straight RGBA8, seen from JS without a copy.",
    syntax: `${importLine("skathi:image", "Image")}

readonly image.width: number
readonly image.height: number
readonly image.pixels: Uint8Array
image.resize(width: number, height: number): Promise<Image>
image.toImageBitmap(): ImageBitmap
image.close(): void`,
    values: [
      { value: "width", status: OK, note: "In pixels, after the EXIF orientation is applied; 0 once closed." },
      { value: "height", status: OK, note: "In pixels; 0 once closed." },
      {
        value: "pixels",
        status: OK,
        note: "`width × height × 4` bytes, row by row, R, G, B, A, with straight (not premultiplied) alpha. A view of the native memory: writes change the image.",
      },
      {
        value: "resize(width, height)",
        status: OK,
        note: "A new, smaller `Image`, filtered on a worker thread with alpha weighting, so transparent pixels do not bleed their colour into the edges.",
      },
      {
        value: "toImageBitmap()",
        status: OK,
        note: "An `ImageBitmap` for `drawImage()`, WebGPU or `<canvas>`, which takes the pixels over without a copy. The image is closed afterwards.",
      },
      { value: "close()", status: OK, note: "Frees the pixels and detaches `pixels`. A second call does nothing." },
    ],
    notes: [
      "An `Image` comes from [`ImageDecoder.decode()`](/std/image-decoder/) or from `resize()`; `new Image()` here would be the HTML element, which this class is not, and the class itself has no constructor.",
      "The memory lives until `close()`, `toImageBitmap()` or until the image is collected. The collector is told the size, but it runs when it decides to: close a large image as soon as you are done with it.",
      "While `resize()` or [`ImageEncoder.encode()`](/std/image-encoder/) reads the pixels on a worker, `pixels` is detached, as after a transfer, and reading it throws `InvalidStateError`. Once the last of them settles, `pixels` is a new view of the same memory: take it again rather than keeping an old one.",
      "`pixels.buffer.transfer()` moves the memory out, and the image counts as closed.",
      SAME_EVERYWHERE,
    ],
    edgeCases: [
      "`resize()` rejects with a `RangeError` for a side that is not an integer from 1 to the image's own: it only makes images smaller. It rejects with `InvalidStateError` once the image is closed.",
      "`toImageBitmap()` throws `InvalidStateError` for a closed image or one a resize or an encode is reading.",
    ],
    examples: [
      {
        title: "A thumbnail from a picked file",
        lang: "ts",
        code: `import { ImageDecoder, ImageEncoder } from "skathi:image";

export async function thumbnail(file: File): Promise<Uint8Array> {
  const decoder = new ImageDecoder({ data: file.stream(), type: file.type });
  const { image } = await decoder.decode({ maxSide: 256 });
  decoder.close();
  try {
    return await ImageEncoder.encode(image, { type: "image/jpeg", quality: 0.8 });
  } finally {
    image.close();
  }
}`,
      },
      {
        title: "Read a pixel",
        lang: "ts",
        code: `const { image } = await decoder.decode();
const at = (y * image.width + x) * 4;
const [r, g, b, a] = image.pixels.subarray(at, at + 4);`,
      },
    ],
    related: ["image-decoder", "image-encoder", "native-buffer"],
  },
  {
    slug: "image-decoder",
    name: "ImageDecoder",
    kind: "class",
    summary:
      "Decodes PNG, JPEG, WebP, GIF and BMP files on worker threads, frame by frame, in the shape of WebCodecs' `ImageDecoder`.",
    syntax: `${importLine("skathi:image", "ImageDecoder")}

new ImageDecoder(init: { data: BufferSource | NativeBuffer | ReadableStream<Uint8Array>; type: string })
static ImageDecoder.isTypeSupported(type: string): Promise<boolean>

readonly decoder.type: string
readonly decoder.complete: boolean
readonly decoder.tracks: { readonly frameCount: number; readonly animated: boolean; readonly ready: Promise<void> }
decoder.decode(options?: { frameIndex?: number; maxSide?: number }): Promise<{ image: Image; complete: boolean }>
decoder.close(): void`,
    params: [
      {
        name: "init.data",
        type: "BufferSource | NativeBuffer | ReadableStream<Uint8Array>",
        description:
          "The file. Bytes are copied when the decoder is made, so the source can be reused or freed at once; a stream is read to its end, and decoding waits for it.",
      },
      {
        name: "init.type",
        type: "string",
        description:
          "The file's MIME type, which picks the decoder: `image/png`, `image/jpeg`, `image/webp`, `image/gif` or `image/bmp`. The bytes are not sniffed.",
      },
      {
        name: "options.frameIndex",
        type: "number",
        description: "The frame of an animation. A still image has frame 0 only.",
        optional: true,
        default: "0",
      },
      {
        name: "options.maxSide",
        type: "number",
        description:
          "Decodes a copy no larger than this on its longer side, with the aspect ratio kept. A smaller image is returned as it is.",
        optional: true,
      },
    ],
    values: [
      { value: "image/png", status: OK, note: "Including APNG animations." },
      { value: "image/jpeg", status: OK, note: "Baseline and progressive." },
      { value: "image/webp", status: OK, note: "Lossy, lossless and animated." },
      { value: "image/gif", status: OK, note: "Every frame, composited onto the canvas as a browser shows it." },
      { value: "image/bmp", status: OK },
      { value: "image/avif", status: NO, note: "Not decoded: `isTypeSupported()` resolves `false`." },
      {
        value: "tracks",
        status: OK,
        note: "Counting the frames starts on first access, on a worker. `frameCount` and `animated` are 0 and `false` until `ready` resolves.",
      },
      {
        value: "complete",
        status: OK,
        note: "Whether all of the data is in: at once for bytes, at the end of a stream.",
      },
      {
        value: "close()",
        status: OK,
        note: "Rejects every decode still waiting with `AbortError`, cancels a stream that is still being read and lets go of the data.",
      },
    ],
    notes: [
      "Decoding runs on worker threads, never on the thread that runs your code and draws the window, so even a large photo does not stall a frame. The promise settles in a later frame.",
      "A photo is turned upright by its EXIF orientation, as browsers draw it: `width` and `height` are those of the upright image.",
      "The decoded image is straight RGBA8 in native memory: a 16-bit PNG is reduced to 8 bits, and a grayscale or palette image is expanded.",
      "Decoding frame N of an animation decodes the frames before it too, since each one is drawn over the last. To show every frame, decode them in order.",
      "Unlike WebCodecs, `decode()` resolves with an [`Image`](/std/image/) instead of a `VideoFrame`, and `tracks` is the one track an image file has.",
      LIMITS,
      "An animation is decoded up to 4,294,967,296 composited pixels (frames times the pixels of the canvas); counting or decoding past that fails with `EncodingError`.",
      SAME_EVERYWHERE,
    ],
    edgeCases: [
      "The constructor throws a `TypeError` without an init object, a type or data, or for empty data; `NotSupportedError` for a type outside the five; a `RangeError` for more than 256 MiB of bytes; and `InvalidStateError` for a freed `NativeBuffer`.",
      "`decode()` rejects with `EncodingError` for a truncated or corrupt file, one that is not of its declared type, or one over the limits; with a `RangeError` for a `frameIndex` past the last frame or a `maxSide` of 0; with the stream's own error if the stream fails; and with `InvalidStateError` after `close()`.",
    ],
    examples: [
      {
        title: "Every frame of a GIF",
        lang: "ts",
        code: `import { ImageDecoder, type Image } from "skathi:image";

const bytes = new Uint8Array(await (await fetch("spinner.gif")).arrayBuffer());
const decoder = new ImageDecoder({ data: bytes, type: "image/gif" });
await decoder.tracks.ready;
const frames: Image[] = [];
for (let i = 0; i < decoder.tracks.frameCount; i++) {
  frames.push((await decoder.decode({ frameIndex: i })).image);
}
decoder.close();`,
      },
    ],
    related: ["image", "image-encoder"],
  },
  {
    slug: "image-encoder",
    name: "ImageEncoder",
    kind: "class",
    summary: "Encodes an `Image` or an `ImageBitmap` to a PNG, JPEG or WebP file on a worker thread.",
    syntax: `${importLine("skathi:image", "ImageEncoder")}

static ImageEncoder.encode(image: Image | ImageBitmap, options: { type: ImageType; quality?: number }): Promise<Uint8Array>
static ImageEncoder.isTypeSupported(type: string): boolean`,
    params: [
      { name: "image", type: "Image | ImageBitmap", description: "The pixels to write." },
      {
        name: "options.type",
        type: "ImageType",
        description: "`image/png`, `image/jpeg` or `image/webp`.",
      },
      {
        name: "options.quality",
        type: "number",
        description: "From 0 to 1, for JPEG only.",
        optional: true,
        default: "0.92",
      },
    ],
    values: [
      { value: "image/png", status: OK, note: "Lossless, with alpha." },
      {
        value: "image/jpeg",
        status: OK,
        note: "No alpha: translucent pixels are composited onto black first, as `canvas.toBlob()` does.",
      },
      { value: "image/webp", status: OK, note: "Lossless only, with alpha; `quality` is ignored." },
      { value: "image/gif", status: NO, note: "Decoded but not encoded: `encode()` rejects with `NotSupportedError`." },
      { value: "image/bmp", status: NO, note: "Decoded but not encoded." },
    ],
    notes: [
      "The encoder is the one `canvas.toBlob()` and `OffscreenCanvas.convertToBlob()` use, run on a worker thread and without a canvas.",
      "The file comes back as a `Uint8Array` over native memory, without a copy.",
      "An `Image` is encoded in place: its `pixels` are detached until the promise settles. An `ImageBitmap` is copied first and can be used meanwhile.",
      "There is no web standard for encoding an image without a canvas, so the name is Skathi's own.",
      SAME_EVERYWHERE,
    ],
    edgeCases: [
      "`encode()` rejects with `NotSupportedError` for a type it does not write, a `RangeError` for a `quality` outside 0 to 1, a `TypeError` for a source that is not an image, and `InvalidStateError` for a closed `Image` or `ImageBitmap`.",
    ],
    examples: [
      {
        title: "Save a screenshot of a canvas as PNG",
        lang: "ts",
        code: `import { ImageEncoder } from "skathi:image";

const bitmap = await createImageBitmap(canvas);
const png = await ImageEncoder.encode(bitmap, { type: "image/png" });
bitmap.close();`,
      },
    ],
    related: ["image", "image-decoder"],
  },
]);
