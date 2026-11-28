import { OK, section } from "./shared";

const SAME_EVERYWHERE = "The same on every platform.";
const NOT_BYTES =
  "A `bytes` argument that is not an `ArrayBuffer`, a typed array or a `DataView` throws a `TypeError`.";
const NOT_STRING = "A `text` argument that is not a string throws a `TypeError`.";

export const encoding = section("skathi:encoding", [
  {
    slug: "hex",
    name: "Hex",
    kind: "namespace",
    summary: "Converts bytes to hex text and back, prints a hex dump, and formats a number as `0x…`.",
    declaration: true,
    params: [
      { name: "bytes", type: "BufferSource", description: "An `ArrayBuffer`, a typed array or a `DataView`." },
      { name: "text", type: "string", description: "`decode`: hex digits, two a byte, in either case." },
      {
        name: "options.width",
        type: "number",
        description: "`dump`: bytes a line, from 1 to 256.",
        optional: true,
        default: "16",
      },
      {
        name: "options.offset",
        type: "number",
        description: "`dump`: the offset printed for the first byte.",
        optional: true,
        default: "0",
      },
      {
        name: "width",
        type: "number",
        description: "`format`: the least number of digits. Shorter values are padded with zeros. At most 64.",
        optional: true,
        default: "no padding",
      },
    ],
    notes: [
      "`encode` writes lowercase, two digits a byte. `decode` reads either case.",
      "`dump` prints lines of offset, hex pairs and ASCII, the way `xxd` does. A byte that is not printable ASCII shows as `.`. Every line ends with a newline, and the last line is padded so the ASCII column lines up.",
      '`format` writes `0x` and the digits: `Hex.format(255, 4)` is `"0x00ff"`. It takes a `bigint` of any size. A negative value has the sign first: `Hex.format(-1n)` is `"-0x1"`.',
      "[Base16](/std/base16/) is `encode` and `decode` under the RFC 4648 name.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Bytes to hex and back",
        lang: "ts",
        code: `import { Hex } from "skathi:encoding";

Hex.encode(new Uint8Array([0xca, 0xfe, 0x01])); // "cafe01"
Hex.decode("00FF10"); // Uint8Array [0, 255, 16]

Hex.format(255, 4); // "0x00ff"
Hex.format(-1n); // "-0x1"
Hex.format(2n ** 64n); // "0x10000000000000000"`,
      },
      {
        title: "Log a packet",
        lang: "ts",
        code: `import { Hex } from "skathi:encoding";

const packet = new Uint8Array([0x48, 0x69, 0x21, 0x00]);
console.log(Hex.dump(packet, { width: 3, offset: 0x10 }));
// 00000010: 4869 21  Hi!
// 00000013: 00       .`,
      },
    ],
    edgeCases: [
      "`decode` throws a `SyntaxError` for an odd number of digits (`Hex.decode: 3 hex digits is not a whole number of bytes`) and for a character that is not a hex digit. Whitespace and a `0x` prefix are not skipped.",
      "`dump` throws a `RangeError` for a `width` outside 1 to 256, and for a `width` or an `offset` that is negative or not an integer.",
      "`format` throws a `RangeError` for a `number` that is not an integer or is beyond ±2^53. Pass a `bigint` for larger values.",
      NOT_BYTES,
      NOT_STRING,
    ],
    related: ["base16", "base32", "base64"],
    aliases: [
      "Hex.encode",
      "Hex.decode",
      "Hex.dump",
      "Hex.format",
      "hexadecimal",
      "hex dump",
      "xxd",
      "hexdump",
      "toHex",
      "fromHex",
      "0x",
    ],
  },
  {
    slug: "base16",
    name: "Base16",
    kind: "namespace",
    summary: "The `encode` and `decode` of `Hex`, under the RFC 4648 name.",
    declaration: true,
    notes: [
      "`Base16.encode` and `Base16.decode` are the same functions as `Hex.encode` and `Hex.decode`. `Base16` has no `dump` and no `format`.",
      "`encode` writes lowercase, two digits a byte. `decode` reads either case.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Decode an uppercase digest",
        lang: "ts",
        code: `import { Base16 } from "skathi:encoding";

Base16.decode("00FF10"); // Uint8Array [0, 255, 16]
Base16.encode(new Uint8Array([0, 255, 16])); // "00ff10"`,
      },
    ],
    edgeCases: [
      "`decode` throws a `SyntaxError` for an odd number of digits and for a character that is not a hex digit.",
      NOT_BYTES,
      NOT_STRING,
    ],
    related: ["hex", "base32", "base64"],
    aliases: ["Base16.encode", "Base16.decode", "RFC 4648", "hexadecimal"],
  },
  {
    slug: "base32",
    name: "Base32",
    kind: "namespace",
    summary: "Encodes bytes as Base32 text and back, in the RFC 4648, base32hex or Crockford alphabet.",
    declaration: true,
    params: [
      { name: "bytes", type: "BufferSource", description: "An `ArrayBuffer`, a typed array or a `DataView`." },
      {
        name: "options.alphabet",
        type: '"rfc4648" | "hex" | "crockford"',
        description: "The alphabet to write or to read.",
        optional: true,
        default: '"rfc4648"',
      },
      {
        name: "options.padding",
        type: "boolean",
        description: "`encode`: pad the text with `=` to a multiple of eight characters.",
        optional: true,
        default: "true; false for Crockford's alphabet",
      },
    ],
    values: [
      { value: '"rfc4648"', status: OK, note: "`A` to `Z` and `2` to `7`. The default." },
      { value: '"hex"', status: OK, note: "base32hex: `0` to `9` and `A` to `V`." },
      {
        value: '"crockford"',
        status: OK,
        note: "`0` to `9` and `A` to `Z` without `I`, `L`, `O` and `U`. No padding by default.",
      },
    ],
    notes: [
      "`encode` writes uppercase.",
      "`decode` takes either case and optional padding, and skips whitespace.",
      "With Crockford's alphabet, `decode` also reads `O` as `0`, `I` and `L` as `1`, and skips hyphens.",
      "Pass the same `alphabet` to `decode` as to `encode`: it is not detected.",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "The three alphabets",
        lang: "ts",
        code: `import { Base32 } from "skathi:encoding";

const utf8 = (text: string) => new TextEncoder().encode(text);

Base32.encode(utf8("foobar")); // "MZXW6YTBOI======"
Base32.encode(utf8("fo"), { padding: false }); // "MZXQ"
Base32.encode(utf8("fooba"), { alphabet: "hex" }); // "CPNMUOJ1"
Base32.encode(new Uint8Array([255, 0, 16]), { alphabet: "crockford" }); // "ZW010"

Base32.decode("mzxw6ytboi"); // the bytes of "foobar"
Base32.decode("zw-o1o", { alphabet: "crockford" }); // Uint8Array [255, 0, 16]`,
      },
    ],
    edgeCases: [
      "`decode` throws a `SyntaxError` for a character outside the alphabet: `Base32.decode: character 1 is not in the alphabet`.",
      "An `alphabet` outside the three names throws a `TypeError`.",
      NOT_BYTES,
      NOT_STRING,
    ],
    related: ["base64", "hex", "base16"],
    aliases: ["Base32.encode", "Base32.decode", "base32hex", "Crockford", "RFC 4648", "TOTP secret", "padding"],
  },
  {
    slug: "base64",
    name: "Base64",
    kind: "namespace",
    summary: "Encodes bytes as Base64 or base64url text and back.",
    declaration: true,
    params: [
      { name: "bytes", type: "BufferSource", description: "An `ArrayBuffer`, a typed array or a `DataView`." },
      {
        name: "options.alphabet",
        type: '"base64" | "base64url"',
        description: "`encode`: the alphabet to write. `decode`: the only alphabet to accept.",
        optional: true,
        default: '"base64" for encode; both for decode',
      },
      {
        name: "options.omitPadding",
        type: "boolean",
        description: "`encode`: leave out the `=` at the end.",
        optional: true,
        default: "false",
      },
    ],
    values: [
      {
        value: '"base64"',
        status: OK,
        note: "`A` to `Z`, `a` to `z`, `0` to `9`, `+` and `/`. The default of `encode`.",
      },
      { value: '"base64url"', status: OK, note: "The same with `-` and `_` in place of `+` and `/`." },
    ],
    notes: [
      "`encode` pads with `=` in both alphabets unless `omitPadding` is set.",
      "`decode` without `alphabet` takes the characters of both alphabets. With `alphabet`, the other alphabet's characters throw.",
      "`decode` skips whitespace, and padding is optional.",
      "`Base64` works on bytes. To encode text, turn it into bytes first with `TextEncoder` or [Encoding](/std/encoding/).",
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Both alphabets",
        lang: "ts",
        code: `import { Base64 } from "skathi:encoding";

const bytes = new Uint8Array([0xfb, 0xff]);

Base64.encode(bytes); // "+/8="
Base64.encode(bytes, { alphabet: "base64url", omitPadding: true }); // "-_8"

Base64.decode("-_8"); // Uint8Array [251, 255]
Base64.decode("+/8=\\n"); // Uint8Array [251, 255]`,
      },
      {
        title: "Read the payload of a JWT",
        lang: "ts",
        code: `import { Base64 } from "skathi:encoding";

export function jwtPayload(token: string): unknown {
  const part = token.split(".")[1];
  if (part === undefined) throw new Error("Not a JWT.");
  const bytes = Base64.decode(part, { alphabet: "base64url" });
  return JSON.parse(new TextDecoder().decode(bytes));
}`,
      },
    ],
    edgeCases: [
      "`decode` throws a `SyntaxError` for a character outside the alphabet: `Base64.decode: character 0 is not in the alphabet`.",
      '`decode` with `alphabet` throws a `SyntaxError` for the other alphabet\'s characters: `Base64.decode("-_8", { alphabet: "base64" })` throws.',
      "`decode` throws a `SyntaxError` when the last group has a single character, which is less than a byte.",
      'An `alphabet` other than `"base64"` or `"base64url"` throws a `TypeError`.',
      NOT_BYTES,
      NOT_STRING,
    ],
    related: ["base32", "hex", "encoding"],
    aliases: [
      "Base64.encode",
      "Base64.decode",
      "base64url",
      "btoa",
      "atob",
      "omitPadding",
      "JWT",
      "data URL",
      "RFC 4648",
    ],
  },
  {
    slug: "mime",
    name: "Mime",
    kind: "namespace",
    summary:
      "Finds a MIME type from a file name or from a file's first bytes, lists a type's extensions, and tests a type against an accept list.",
    declaration: true,
    params: [
      { name: "name", type: "string", description: "`fromName`: a file name or a path. Only the extension is read." },
      {
        name: "type",
        type: "string",
        description: "A MIME type such as `image/png`. `extensions` also takes a `type/*` pattern.",
      },
      { name: "bytes", type: "BufferSource", description: "`sniff`: the start of a file's content." },
      {
        name: "accept",
        type: "string",
        description:
          '`matches`: a comma-separated list of types, `type/*` patterns, `*/*` and extensions with a dot, such as `"image/*, .pdf"`.',
      },
    ],
    values: [
      {
        value: "fromName(name)",
        status: OK,
        note: 'The type for the extension, as `File.type` gives it. `""` when unknown.',
      },
      {
        value: "extensions(type)",
        status: OK,
        note: "The extensions of a type, without the dot, the usual one first.",
      },
      { value: "sniff(bytes)", status: OK, note: "The type the first bytes show, or `null`." },
      { value: "matches(type, accept)", status: OK, note: "Whether `type` fits the accept list." },
    ],
    notes: [
      "`fromName` answers from the table behind `File.type` and local `fetch()`, so the three agree. The extension is matched in any case: `photo.PNG` is `image/png`.",
      '`extensions("image/*")` lists the extensions of every `image/` type. An unknown type, a malformed one and `*/*` give an empty array.',
      "`sniff` reads the signature in a file's first bytes. A few dozen bytes are enough for most formats. It does not look at a file name.",
      "`sniff` knows these signatures. Images: PNG, JPEG, GIF, BMP, ICO, WebP, AVIF, HEIC, SVG. Audio: MP3, FLAC, WAV, AIFF, MIDI, M4A, Ogg (as `application/ogg`). Video: MP4, QuickTime, WebM, Matroska, AVI. Fonts: WOFF, WOFF2, TTF, OTF, TTC. Archives: ZIP, gzip, RAR, 7z, xz, Zstandard, bzip2. Others: PDF, PostScript, WebAssembly, SQLite 3, XML, HTML.",
      "`sniff` reports a format built on ZIP, such as `.docx`, as `application/zip`. Plain text is `text/plain` only when it starts with a byte order mark; without one it is `null`.",
      "`matches` ignores case and a type's parameters: `image/PNG; q=1` fits `image/*`. An extension entry fits when it is one of the type's extensions.",
      "`Mime.extensions` on Windows also asks the registry for a type the table does not know. The rest is the same on every platform.",
    ],
    examples: [
      {
        title: "Name, content and accept list",
        lang: "ts",
        code: `import { Mime } from "skathi:encoding";

Mime.fromName("dir/photo.PNG"); // "image/png"
Mime.fromName("noext"); // ""
Mime.extensions("image/jpeg").includes("jpg"); // true

const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
Mime.sniff(png); // "image/png"
Mime.sniff(new TextEncoder().encode("plain")); // null

Mime.matches("image/png", "text/html, image/*"); // true
Mime.matches("image/png", "image/*, .pdf"); // true
Mime.matches("image/png", ".gif"); // false`,
      },
      {
        title: "Check a file by its content, then by its name",
        lang: "ts",
        code: `import { Mime } from "skathi:encoding";

export function isAcceptedImage(name: string, content: Uint8Array): boolean {
  const type = Mime.sniff(content.subarray(0, 64)) ?? Mime.fromName(name);
  return Mime.matches(type, "image/png, image/jpeg, image/webp");
}`,
      },
    ],
    edgeCases: [
      "`matches` returns `false` for a `type` that is not a MIME type, the empty string included.",
      "A `name`, `type` or `accept` that is not a string throws a `TypeError`.",
      NOT_BYTES,
    ],
    related: ["encoding", "hex"],
    aliases: [
      "Mime.fromName",
      "Mime.extensions",
      "Mime.sniff",
      "Mime.matches",
      "MIME type",
      "content type",
      "media type",
      "file signature",
      "magic bytes",
      "accept",
      "File.type",
      "file extension",
    ],
  },
  {
    slug: "encoding",
    name: "Encoding",
    kind: "namespace",
    summary:
      "Decodes and encodes text in any encoding `TextDecoder` knows, the legacy ones included, and detects an encoding from a byte order mark.",
    declaration: true,
    params: [
      { name: "bytes", type: "BufferSource", description: "An `ArrayBuffer`, a typed array or a `DataView`." },
      { name: "text", type: "string", description: "`encode`: the text to write." },
      {
        name: "label",
        type: "string",
        description: 'Any `TextDecoder` label: `"utf-8"`, `"windows-1251"`, `"shift_jis"`, `"gbk"`, `"utf-16le"`, …',
      },
    ],
    notes: [
      "`Encoding` takes any `TextDecoder` label.",
      "`TextEncoder` only writes UTF-8. `Encoding.encode` writes the legacy encodings as well (`windows-1251`, `shift_jis`, `gbk`, …).",
      "`encode` turns a character the encoding has no bytes for into `?`. It writes UTF-16 without a byte order mark.",
      "`decode` drops a byte order mark of the encoding it decodes.",
      "`isSupported(label)` tells whether `decode` accepts the label. It never throws.",
      '`detect` goes by the byte order mark only. It returns `"utf-8"`, `"utf-16le"` or `"utf-16be"`, and `null` without a mark. It does not guess a legacy encoding from the content.',
      SAME_EVERYWHERE,
    ],
    examples: [
      {
        title: "Write and read a legacy encoding",
        lang: "ts",
        code: `import { Encoding } from "skathi:encoding";

const bytes = Encoding.encode("Да€日", "windows-1251"); // Uint8Array [0xc4, 0xe0, 0x88, 0x3f]
Encoding.decode(bytes, "windows-1251"); // "Да€?"

Encoding.isSupported("shift_jis"); // true
Encoding.isSupported("nope"); // false`,
      },
      {
        title: "Decode a text file by its byte order mark",
        lang: "ts",
        code: `import { Encoding } from "skathi:encoding";

export function readText(bytes: Uint8Array, fallback = "windows-1252"): string {
  return Encoding.decode(bytes, Encoding.detect(bytes) ?? fallback);
}

Encoding.detect(new Uint8Array([0xff, 0xfe, 0x41, 0])); // "utf-16le"
Encoding.detect(new TextEncoder().encode("a")); // null`,
      },
    ],
    edgeCases: [
      'An unknown `label` throws a `RangeError` from `decode` and `encode`: `Encoding.decode: "nope" is not an encoding this host knows`.',
      "`encode` throws a `RangeError` for a label that names an encoding that can only be decoded.",
      "A `label` that is not a string throws a `TypeError` from `decode` and `encode`. `isSupported` returns `false` for it.",
      NOT_BYTES,
      NOT_STRING,
    ],
    related: ["mime", "base64", "hex", "binary-reader"],
    aliases: [
      "Encoding.decode",
      "Encoding.encode",
      "Encoding.isSupported",
      "Encoding.detect",
      "TextDecoder",
      "TextEncoder",
      "charset",
      "code page",
      "windows-1251",
      "windows-1252",
      "shift_jis",
      "gbk",
      "utf-16",
      "BOM",
      "byte order mark",
      "legacy encoding",
    ],
  },
]);
