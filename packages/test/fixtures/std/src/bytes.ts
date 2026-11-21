import { BinaryReader, BinaryWriter, BitReader, BitWriter, CRC32, CRC64, Endian, VarInt } from "skathi:binary";
import { Base16, Base32, Base64, Encoding, Hex, Mime } from "skathi:encoding";

const attempt = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

const utf8 = (text: string) => Encoding.encode(text, "utf-8");
const HEAD = { magic: "u32", size: "u16", id: "i64" } as const;

export function binary() {
  const writer = new BinaryWriter({ endian: "big", initialCapacity: 4 });
  writer.writeStruct(HEAD, { magic: 0xcafebabe, size: 0, id: -2n });
  writer
    .writeAll([1.5, -2], "f32")
    .writeAll(new Uint16Array([0x0102]), "u16")
    .bytes(new Uint8Array([0xe5, 0x8e, 0x26, 0x7f]))
    .bytes(utf8("héllo"));
  const written = writer.length;
  writer.view.setUint16(4, written);
  const bytes = writer.finish();

  const reader = new BinaryReader(bytes, { endian: "big" });
  const head = reader.readStruct(HEAD);
  const floats = reader.readArray(Float32Array, 2);
  const word = reader.readArray(Uint16Array, 1);
  const vars = reader.varUints(2);
  const text = reader.string(6);

  const source = new Uint8Array([9, 8, 7, 6, 5]);
  const part = new BinaryReader(source.subarray(1));
  part.position = 1;
  const window = part.bytes(2);

  const bits = new BitWriter("lsb").write(5, 3).writeMany([1, 2, 3], 2).finish();
  const bitReader = new BitReader(bits, "lsb");
  const firstBits = bitReader.read(3);
  const nextBits = [...bitReader.readMany(2, 3)];
  bitReader.align();

  const swapped = new Uint16Array([0x1234]);
  Endian.swap16(swapped);
  const stream = CRC32.create();
  stream.update(utf8("1234"));
  stream.update(utf8("56789"));

  return {
    written,
    emptied: writer.length,
    head: { magic: head.magic, size: head.size, id: String(head.id), idType: typeof head.id },
    floats: [...floats],
    word: word[0],
    vars: [...vars],
    text,
    remaining: reader.remaining,
    past: attempt(() => reader.bytes(1)),
    window: [...window],
    shared: window.buffer === source.buffer && window.byteOffset === 2,
    bits: [...bits],
    firstBits,
    nextBits,
    bitsLeft: attempt(() => bitReader.read(1)),
    swapped: swapped[0],
    native: Endian.native,
    varInts: [...VarInt.decodeAll(VarInt.encodeAll([0, 300, -1, 2 ** 40], true), true)],
    varSize: VarInt.size(300),
    negative: attempt(() => VarInt.encodeAll([-1])),
    crc32: CRC32.compute(utf8("123456789")),
    crc32Chained: CRC32.compute(utf8("56789"), CRC32.compute(utf8("1234"))),
    crc32Stream: stream.digest(),
    crc64: CRC64.compute(utf8("123456789")).toString(16),
    crc64Nvme: CRC64.compute(utf8("123456789"), "nvme").toString(16),
    badType: attempt(() => reader.readStruct({ a: "u24" as "u32" })),
    globals: ["BinaryReader", "BitWriter", "Runtime", "App", "OS"].filter((name) => name in globalThis),
  };
}

export function encoding() {
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
  const cyrillic = Encoding.encode("Да€日", "windows-1251");
  return {
    hex: Hex.encode(new Uint8Array([0, 255, 16])),
    unhex: [...Base16.decode("00FF10")],
    oddHex: attempt(() => Hex.decode("abc")),
    format: [Hex.format(255, 4), Hex.format(-1n), Hex.format(2n ** 64n)],
    dump: Hex.dump(utf8("Hi!")),
    base32: Base32.encode(utf8("foobar")),
    base32Back: Encoding.decode(Base32.decode("mzxw6ytboi"), "utf-8"),
    crockford: Base32.encode(new Uint8Array([255, 0, 16]), { alphabet: "crockford" }),
    base64: Base64.encode(new Uint8Array([0xfb, 0xff])),
    base64Url: Base64.encode(new Uint8Array([0xfb, 0xff]), { alphabet: "base64url", omitPadding: true }),
    base64Back: [...Base64.decode("-_8")],
    base64Strict: attempt(() => Base64.decode("-_8", { alphabet: "base64" })),
    mime: [Mime.fromName("dir/photo.PNG"), Mime.fromName("noext")],
    extensions: Mime.extensions("image/jpeg"),
    sniff: [Mime.sniff(png), Mime.sniff(utf8("plain"))],
    matches: [Mime.matches("image/png", "text/html, image/*"), Mime.matches("image/png", ".gif")],
    cyrillic: [...cyrillic],
    decoded: Encoding.decode(cyrillic, "windows-1251"),
    supported: [Encoding.isSupported("shift_jis"), Encoding.isSupported("nope")],
    detect: [Encoding.detect(new Uint8Array([0xff, 0xfe, 0x41, 0])), Encoding.detect(utf8("a"))],
    unknown: attempt(() => Encoding.decode(png, "nope")),
  };
}
