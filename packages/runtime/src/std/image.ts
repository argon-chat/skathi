// `skathi:image`: image files decoded and encoded on worker threads, without a canvas. The host installs the classes.

import type { NativeBuffer } from "./memory";

/** What `ImageEncoder` writes; an alias is answered with the first type of its group. */
export type EncodableImageType =
  | "image/png"
  | "image/jpeg"
  | "image/webp"
  | "image/gif"
  | "image/bmp"
  | "image/x-icon"
  | "image/vnd.microsoft.icon"
  | "image/tiff"
  | "image/x-portable-anymap"
  | "image/x-portable-bitmap"
  | "image/x-portable-graymap"
  | "image/x-portable-pixmap"
  | "image/qoi"
  | "image/x-tga"
  | "image/x-targa"
  | "image/vnd.radiance"
  | "image/x-farbfeld";

/** What `ImageDecoder` reads: everything `ImageEncoder` writes, plus SVG and DDS. */
export type ImageType = EncodableImageType | "image/svg+xml" | "image/vnd-ms.dds" | "image/vnd.ms-dds";

/** Decoded pixels: straight RGBA8 in native memory, `width × height × 4` bytes. */
export interface Image {
  /** 0 once closed. */
  readonly width: number;
  /** 0 once closed. */
  readonly height: number;
  /** A view of the native memory, not a copy. Throws `InvalidStateError` while a resize or an encode reads it. */
  readonly pixels: Uint8Array;
  /** A smaller copy, filtered on a worker. Neither side may be larger than the image's own. */
  resize(width: number, height: number): Promise<Image>;
  /** Hands the pixels to a new `ImageBitmap` without a copy; the image is closed afterwards. */
  toImageBitmap(): ImageBitmap;
  /** Frees the pixels and detaches `pixels`. A second call does nothing. */
  close(): void;
}

export interface ImageStatics {
  readonly prototype: Image;
}

export interface ImageDecoderInit {
  /** The file's bytes, copied when the decoder is made; a stream is read to its end. */
  data: BufferSource | NativeBuffer | ReadableStream<Uint8Array>;
  /** The file's type, which picks the decoder: one of `ImageType`. */
  type: string;
}

export interface ImageTrack {
  /** 0 until `ready` resolves. */
  readonly frameCount: number;
  /** `false` until `ready` resolves. */
  readonly animated: boolean;
  /** Resolves once the frames are counted; rejects with `EncodingError` for a file that does not decode. */
  readonly ready: Promise<void>;
}

export interface ImageDecodeOptions {
  /** 0 by default. */
  frameIndex?: number;
  /** Decodes a copy no larger than this on its longer side, keeping the aspect ratio. */
  maxSide?: number;
}

export interface ImageDecodeResult {
  image: Image;
  complete: boolean;
}

export interface ImageInfo {
  /** Full-resolution display dimensions with the file's orientation applied. */
  readonly width: number;
  readonly height: number;
  /** EXIF orientation, or 1 when no orientation is specified. */
  readonly orientation: 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8;
}

/** The shape of WebCodecs' `ImageDecoder`, with an `Image` in place of a `VideoFrame`. */
export interface ImageDecoder {
  readonly type: string;
  /** Whether all of the data is in: always for bytes, at the end of a stream. */
  readonly complete: boolean;
  readonly tracks: ImageTrack;
  /** Reads canvas dimensions and orientation on a worker, without decoding frames. */
  info(): Promise<ImageInfo>;
  decode(options?: ImageDecodeOptions): Promise<ImageDecodeResult>;
  /** Rejects what is still waiting with `AbortError` and lets go of the data. */
  close(): void;
}

export interface ImageDecoderConstructor {
  readonly prototype: ImageDecoder;
  new (init: ImageDecoderInit): ImageDecoder;
  isTypeSupported(type: string): Promise<boolean>;
}

export interface ImageEncoderStatics {
  readonly prototype: object;
  /** The file's bytes. `quality` (0 to 1, 0.92 by default) is used by JPEG only; WebP is lossless. */
  encode(image: Image | ImageBitmap, options: { type: EncodableImageType; quality?: number }): Promise<Uint8Array>;
  isTypeSupported(type: string): boolean;
}

declare const __sk_std: {
  image: {
    Image: ImageStatics;
    ImageDecoder: ImageDecoderConstructor;
    ImageEncoder: ImageEncoderStatics;
  };
};

export const Image: ImageStatics = __sk_std.image.Image;
export const ImageDecoder: ImageDecoderConstructor = __sk_std.image.ImageDecoder;
export const ImageEncoder: ImageEncoderStatics = __sk_std.image.ImageEncoder;
