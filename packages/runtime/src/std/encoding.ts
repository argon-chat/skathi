// `skathi:encoding`: bytes as text and back, and MIME types. The host installs the objects.

export interface HexCodec {
  /** Lowercase hex, two digits a byte. */
  encode(bytes: BufferSource): string;
  /** Either case. Throws a `SyntaxError` for an odd length or a character that is not a digit. */
  decode(text: string): Uint8Array;
}

export interface HexOps extends HexCodec {
  /** Lines of offset, hex pairs and ASCII, as `xxd` prints them. `width` is bytes a line, 16 by default. */
  dump(bytes: BufferSource, options?: { width?: number; offset?: number }): string;
  /** `0x` and the digits, padded with zeros to `width` digits: `Hex.format(255, 4)` is `"0x00ff"`. */
  format(value: number | bigint, width?: number): string;
}

export type Base32Alphabet = "rfc4648" | "hex" | "crockford";
export type Base64Alphabet = "base64" | "base64url";

export interface Base32Ops {
  /** Padded with `=` unless the alphabet is Crockford's, which has no padding. */
  encode(bytes: BufferSource, options?: { alphabet?: Base32Alphabet; padding?: boolean }): string;
  /**
   * Either case, padding optional, whitespace skipped. Crockford also reads `O` as `0`, `I` and `L`
   * as `1`, and skips hyphens. Throws a `SyntaxError` for any other character.
   */
  decode(text: string, options?: { alphabet?: Base32Alphabet }): Uint8Array;
}

export interface Base64Ops {
  encode(bytes: BufferSource, options?: { alphabet?: Base64Alphabet; omitPadding?: boolean }): string;
  /**
   * Padding optional, whitespace skipped. Without `alphabet` the characters of both are taken;
   * with it, the other one's throw a `SyntaxError`.
   */
  decode(text: string, options?: { alphabet?: Base64Alphabet }): Uint8Array;
}

export interface MimeOps {
  /** The type for a file name or path by its extension, as `File.type` gives it; `""` when unknown. */
  fromName(name: string): string;
  /** The extensions of a type, without the dot, the usual one first. `image/*` lists them all. */
  extensions(type: string): string[];
  /** The type a file's first bytes show, or `null`. A few dozen bytes are enough for most formats. */
  sniff(bytes: BufferSource): string | null;
  /** Whether `type` fits an accept list such as `"image/*, .pdf"`. */
  matches(type: string, accept: string): boolean;
}

export interface EncodingOps {
  /** `label` is any `TextDecoder` label. A byte order mark of that encoding is dropped. */
  decode(bytes: BufferSource, label: string): string;
  /** A character the encoding has no bytes for becomes `?`. UTF-16 is written without a byte order mark. */
  encode(text: string, label: string): Uint8Array;
  isSupported(label: string): boolean;
  /** `"utf-8"`, `"utf-16le"` or `"utf-16be"` by the byte order mark, or `null` without one. */
  detect(bytes: BufferSource): string | null;
}

declare const __sk_std: {
  encoding: {
    Base16: HexCodec;
    Base32: Base32Ops;
    Base64: Base64Ops;
    Hex: HexOps;
    Mime: MimeOps;
    Encoding: EncodingOps;
  };
};

export const Base16: HexCodec = __sk_std.encoding.Base16;
export const Base32: Base32Ops = __sk_std.encoding.Base32;
export const Base64: Base64Ops = __sk_std.encoding.Base64;
export const Hex: HexOps = __sk_std.encoding.Hex;
export const Mime: MimeOps = __sk_std.encoding.Mime;
export const Encoding: EncodingOps = __sk_std.encoding.Encoding;
