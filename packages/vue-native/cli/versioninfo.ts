// `VS_VERSIONINFO`: what Explorer's Details tab, Task Manager and SmartScreen read out of an exe.
// A tree of length-prefixed nodes: WORD wLength; WORD wValueLength; WORD wType; WCHAR szKey[]; pad to 4;
// Value; pad to 4; Children. `wValueLength` counts BYTES for a binary value, WORDS for a text one.

export interface VersionStrings {
  FileDescription: string;
  ProductName: string;
  ProductVersion: string;
  FileVersion: string;
  InternalName: string;
  OriginalFilename: string;
  CompanyName?: string;
  LegalCopyright?: string;
  Comments?: string;
}

export interface VersionInfoInput {
  fileVersion: [number, number, number, number];
  productVersion?: [number, number, number, number];
  strings: VersionStrings;
  lang?: number;
  codepage?: number;
}

const utf16 = (s: string) => new Uint8Array(Buffer.from(s + "\0", "utf16le"));

function concat(parts: Uint8Array[]): Uint8Array {
  const n = parts.reduce((a, p) => a + p.length, 0);
  const out = new Uint8Array(n);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

const pad4 = (b: Uint8Array) => (b.length % 4 ? concat([b, new Uint8Array(4 - (b.length % 4))]) : b);

function node(
  key: string,
  type: 0 | 1,
  value: Uint8Array | null,
  valueLen: number,
  children: Uint8Array[],
): Uint8Array {
  let body = pad4(concat([new Uint8Array(6), utf16(key)]));
  if (value) body = pad4(concat([body, value]));
  for (const c of children) body = pad4(concat([body, c]));
  const dv = new DataView(body.buffer, body.byteOffset, body.byteLength);
  if (body.length > 0xffff) throw new Error(`VERSIONINFO node ${key} is ${body.length} bytes (> 64 KiB)`);
  dv.setUint16(0, body.length, true);
  dv.setUint16(2, value ? valueLen : 0, true);
  dv.setUint16(4, type, true);
  return body;
}

export function numericVersion(v: string): [number, number, number, number] {
  const m = /^\s*v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:\.(\d+))?(?:[-+].*)?\s*$/.exec(v);
  if (!m) throw new Error(`${JSON.stringify(v)} is not a version (expected a.b.c, semver)`);
  const parts = [m[1], m[2], m[3], m[4]].map((p) => (p === undefined ? 0 : Number(p))) as [
    number,
    number,
    number,
    number,
  ];
  for (const p of parts) {
    if (p > 0xffff) throw new Error(`${JSON.stringify(v)}: each version part must be <= 65535 for VERSIONINFO`);
  }
  return parts;
}

export function buildVersionInfo(input: VersionInfoInput): Uint8Array {
  const lang = input.lang ?? 0x0409;
  const codepage = input.codepage ?? 1200;
  const fv = input.fileVersion;
  const pv = input.productVersion ?? fv;

  const fixed = new Uint8Array(52);
  const dv = new DataView(fixed.buffer);
  const fields = [
    0xfeef04bd, // dwSignature
    0x00010000, // dwStrucVersion
    ((fv[0] << 16) | fv[1]) >>> 0,
    ((fv[2] << 16) | fv[3]) >>> 0,
    ((pv[0] << 16) | pv[1]) >>> 0,
    ((pv[2] << 16) | pv[3]) >>> 0,
    0x3f, // dwFileFlagsMask
    0, // dwFileFlags
    0x40004, // dwFileOS = VOS_NT_WINDOWS32
    1, // dwFileType = VFT_APP
    0, // dwFileSubtype
    0, // dwFileDateMS — no timestamps: two builds of one input are byte-identical
    0, // dwFileDateLS
  ];
  fields.forEach((x, i) => dv.setUint32(i * 4, x >>> 0, true));

  const table = `${lang.toString(16).padStart(4, "0")}${codepage.toString(16).padStart(4, "0")}`;
  const order: (keyof VersionStrings)[] = [
    "CompanyName",
    "FileDescription",
    "FileVersion",
    "InternalName",
    "LegalCopyright",
    "OriginalFilename",
    "ProductName",
    "ProductVersion",
    "Comments",
  ];
  const strings: Uint8Array[] = [];
  for (const key of order) {
    const value = input.strings[key];
    if (value === undefined || value === "") continue;
    strings.push(node(key, 1, utf16(value), value.length + 1, []));
  }
  const translation = new Uint8Array(4);
  new DataView(translation.buffer).setUint16(0, lang, true);
  new DataView(translation.buffer).setUint16(2, codepage, true);

  const stringFileInfo = node("StringFileInfo", 1, null, 0, [node(table, 1, null, 0, strings)]);
  const varFileInfo = node("VarFileInfo", 1, null, 0, [node("Translation", 0, translation, 4, [])]);
  return node("VS_VERSION_INFO", 0, fixed, 52, [stringFileInfo, varFileInfo]);
}
