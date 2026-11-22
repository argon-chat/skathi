import { ImageDecoder, ImageEncoder } from "skathi:image";
import { NativeBuffer } from "skathi:memory";

// Values a premultiplied canvas gives back unchanged.
const PIXELS = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 0, 0, 0, 0];
const CHECKER = Array.from({ length: 8 }, (_, i) => PIXELS.slice((i % 4) * 4, (i % 4) * 4 + 4)).flat();

const rejection = (promise: Promise<unknown>): Promise<string> =>
  promise.then(
    () => "ok",
    (e: unknown) => (e instanceof Error || e instanceof DOMException ? e.name : String(e)),
  );

export async function image() {
  const painted = new OffscreenCanvas(4, 2);
  const paint = painted.getContext("2d");
  if (!paint) throw new Error("no 2D context");
  const data = paint.createImageData(4, 2);
  data.data.set(CHECKER);
  paint.putImageData(data, 0, 0);
  const source = await createImageBitmap(painted);
  const png = await ImageEncoder.encode(source, { type: "image/png" });

  const streamed = new ImageDecoder({ data: new Blob([png]).stream(), type: "image/png" });
  const completeBefore = streamed.complete;
  await streamed.tracks.ready;
  const { image: decoded, complete } = await streamed.decode();
  const pixels = [...decoded.pixels];

  const file = NativeBuffer.alloc(png.byteLength);
  new Uint8Array(file.buffer).set(png);
  const fromNative = new ImageDecoder({ data: file, type: "IMAGE/PNG" });
  file.free();
  const thumb = (await fromNative.decode({ maxSide: 2 })).image;

  const small = await decoded.resize(2, 1);
  const jpeg = await ImageEncoder.encode(small, { type: "image/jpeg", quality: 0.5 });
  const fromJpeg = (await new ImageDecoder({ data: jpeg, type: "image/jpeg" }).decode()).image;

  const bitmap = decoded.toImageBitmap();
  const canvas = new OffscreenCanvas(4, 2);
  const context = canvas.getContext("2d");
  context?.drawImage(bitmap, 0, 0);
  const drawn = context ? [...context.getImageData(0, 0, 1, 1).data] : [];

  const unread = new ImageDecoder({ data: new Blob([png]).stream(), type: "image/png" });
  const waiting = rejection(unread.decode());
  unread.close();

  return {
    streamed: [completeBefore, streamed.complete, streamed.type, streamed.tracks.frameCount, streamed.tracks.animated],
    decoded: [decoded.width === 0, complete, pixels.length, pixels.join() === CHECKER.join()],
    thumb: [thumb.width, thumb.height, fromNative.type],
    small: [small.width, small.height, small.pixels.length],
    jpeg: [jpeg[0], jpeg[1], fromJpeg.width, fromJpeg.height],
    bitmap: [bitmap.width, bitmap.height, drawn.join()],
    supported: [
      await ImageDecoder.isTypeSupported("image/gif"),
      await ImageDecoder.isTypeSupported("image/avif"),
      ImageEncoder.isTypeSupported("image/webp"),
      ImageEncoder.isTypeSupported("image/gif"),
    ],
    errors: {
      truncated: await rejection(new ImageDecoder({ data: png.slice(0, 40), type: "image/png" }).decode()),
      wrongType: await rejection(new ImageDecoder({ data: png, type: "image/jpeg" }).decode()),
      noFrame: await rejection(new ImageDecoder({ data: png, type: "image/png" }).decode({ frameIndex: 1 })),
      enlarge: await rejection(small.resize(3, 1)),
      gif: await rejection(ImageEncoder.encode(small, { type: "image/gif" })),
      closed: await rejection(decoded.resize(1, 1)),
      aborted: await waiting,
      freed: await rejection(Promise.resolve().then(() => new ImageDecoder({ data: file, type: "image/png" }))),
    },
  };
}
