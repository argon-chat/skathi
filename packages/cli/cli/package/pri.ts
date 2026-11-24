// resources.pri, the package resource index Windows reads to pick a tile or taskbar image by scale, target size
// and plate. Written byte for byte as `makepri new` writes it (format "mrm_pri2") for the variant names
// `msixAssets` produces: scale-N, targetsize-N and altform-(light)unplated.

/** How each file-name qualifier ranks; the scores are makepri's against its defaults (scale-100, targetsize-256). */
const QUALIFIERS: Readonly<
  Record<string, { type: number; priority: number; scores: Readonly<Record<string, number>> }>
> = {
  scale: { type: 2, priority: 200, scores: { "100": 1000, "125": 937, "150": 875, "200": 750, "400": 437 } },
  targetsize: { type: 4, priority: 300, scores: { "16": 500, "24": 500, "32": 500, "48": 500, "256": 1000 } },
  altform: { type: 7, priority: 100, scores: { UNPLATED: 0, LIGHTUNPLATED: 0 } },
};

interface Qualifier {
  key: string;
  value: string;
  priority: number;
  score: number;
}

/** Splits `Name.q1-v1_q2-v2.ext` into the resource name and its qualifiers. */
function parseVariant(fileName: string): { name: string; qualifiers: Qualifier[] } {
  const parts = fileName.split(".");
  if (parts.length < 3) return { name: fileName, qualifiers: [] };
  const segment = parts[parts.length - 2]!;
  const qualifiers: Qualifier[] = [];
  for (const token of segment.split("_")) {
    const dash = token.indexOf("-");
    const key = token.slice(0, dash).toLowerCase();
    const q = dash > 0 ? QUALIFIERS[key] : undefined;
    if (!q) return { name: fileName, qualifiers: [] };
    const value = key === "altform" ? token.slice(dash + 1).toUpperCase() : token.slice(dash + 1);
    const score = q.scores[value];
    if (score === undefined) throw new Error(`${fileName}: ${token} is not a variant resources.pri is written for`);
    qualifiers.push({ key, value, priority: q.priority, score });
  }
  return {
    name: [...parts.slice(0, -2), parts[parts.length - 1]].join("."),
    qualifiers: qualifiers.sort(compareQualifier).reverse(),
  };
}

const upper = (a: string, b: string) => {
  const x = a.toUpperCase();
  const y = b.toUpperCase();
  return x < y ? -1 : x > y ? 1 : 0;
};

function compareQualifier(a: Qualifier, b: Qualifier): number {
  if (a.priority !== b.priority) return a.priority > b.priority ? 1 : -1;
  if (a.key !== b.key) return QUALIFIERS[a.key]!.type > QUALIFIERS[b.key]!.type ? 1 : -1;
  return upper(a.value, b.value) || Math.sign(a.score - b.score);
}

function compareSet(a: readonly Qualifier[], b: readonly Qualifier[]): number {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    const c = compareQualifier(a[i]!, b[i]!);
    if (c) return c;
  }
  return Math.sign(a.length - b.length);
}

class Writer {
  private bytes = new Uint8Array(1024);
  private view = new DataView(this.bytes.buffer);
  length = 0;

  private room(n: number) {
    if (this.length + n <= this.bytes.length) return;
    const next = new Uint8Array(Math.max(this.bytes.length * 2, this.length + n));
    next.set(this.bytes);
    this.bytes = next;
    this.view = new DataView(next.buffer);
  }
  u8(v: number) {
    this.room(1);
    this.view.setUint8(this.length, v);
    this.length += 1;
  }
  u16(v: number) {
    this.room(2);
    this.view.setUint16(this.length, v, true);
    this.length += 2;
  }
  u32(v: number) {
    this.room(4);
    this.view.setUint32(this.length, v >>> 0, true);
    this.length += 4;
  }
  raw(b: Uint8Array) {
    this.room(b.length);
    this.bytes.set(b, this.length);
    this.length += b.length;
  }
  ascii(s: string) {
    for (let i = 0; i < s.length; i++) this.u8(s.charCodeAt(i));
    this.u8(0);
  }
  utf16(s: string) {
    for (let i = 0; i < s.length; i++) this.u16(s.charCodeAt(i));
    this.u16(0);
  }
  pad(to: number) {
    while (this.length % to) this.u8(0);
  }
  done(): Uint8Array {
    return this.bytes.slice(0, this.length);
  }
}

const tag = (id: string) => {
  const b = new Uint8Array(16);
  for (let i = 0; i < id.length; i++) b[i] = id.charCodeAt(i);
  return b;
};

function section(id: string, content: Uint8Array): Uint8Array {
  const w = new Writer();
  const length = 32 + content.length + 8;
  w.raw(tag(id));
  w.u32(0);
  w.u16(0);
  w.u16(0);
  w.u32(length);
  w.u32(0);
  w.raw(content);
  w.u32(0xdef5fade);
  w.u32(length);
  return w.done();
}

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(crc: number, bytes: Uint8Array): number {
  let c = ~crc >>> 0;
  for (const b of bytes) c = (CRC_TABLE[(c ^ b) & 0xff]! ^ (c >>> 8)) >>> 0;
  return ~c >>> 0;
}

const le32 = (v: number) => new Uint8Array(new Uint32Array([v]).buffer);
const crcData = (crc: number, data: Uint8Array | null) =>
  data ? crc32(crc32(crc, le32(data.length)), data) : crc32(crc32(crc, le32(0)), le32(0));
const crcString = (crc: number, s: string | null) => {
  if (s === null) return crcData(crc, null);
  const w = new Writer();
  w.utf16(s.toLowerCase());
  return crcData(crc, w.done());
};
const crcPool = (crc: number, names: readonly string[]) => {
  let c = crcString(crc, null);
  c = crc32(c, le32(1));
  c = crc32(c, le32(names.length));
  for (const n of names) c = crcString(c, n);
  return c;
};

interface Node {
  name: string;
  parent: Node | null;
  scope: boolean;
  /** Scope or item index, in the order they were added. */
  index: number;
  children: Node[];
  nameIndex: number;
}

const fullPath = (n: Node, sep: string): string =>
  !n.parent ? n.name : n.parent.parent ? fullPath(n.parent, sep) + sep + n.name : n.name;

/**
 * resources.pri for the package named `packageName` (its Identity/@Name), indexing `files`: package paths, `/`
 * separated, of images whose names may carry qualifiers. Deterministic: the same input gives the same bytes.
 */
export function resourcesPri(packageName: string, files: readonly string[]): Uint8Array {
  const sorted = [...files].sort(upper);
  const root: Node = { name: "", parent: null, scope: true, index: 0, children: [], nameIndex: 0 };
  const files0: Node = { name: "Files", parent: root, scope: true, index: 1, children: [], nameIndex: 0 };
  root.children.push(files0);
  const scopes: Node[] = [root, files0];
  const items: Node[] = [];
  const candidates = new Map<Node, { qualifiers: Qualifier[]; path: string }[]>();

  for (const file of sorted) {
    const segments = file.split("/");
    let at = files0;
    for (const dir of segments.slice(0, -1)) {
      let next = at.children.find((c) => c.scope && upper(c.name, dir) === 0);
      if (!next) {
        next = { name: dir, parent: at, scope: true, index: scopes.length, children: [], nameIndex: 0 };
        scopes.push(next);
        at.children.push(next);
      }
      at = next;
    }
    const { name, qualifiers } = parseVariant(segments[segments.length - 1]!);
    let item = at.children.find((c) => !c.scope && upper(c.name, name) === 0);
    if (!item) {
      item = { name, parent: at, scope: false, index: items.length, children: [], nameIndex: 0 };
      items.push(item);
      at.children.push(item);
      candidates.set(item, []);
    }
    candidates.get(item)!.push({ qualifiers, path: segments.join("\\") });
  }

  const sortChildren = (n: Node) => {
    n.children.sort((a, b) => upper(a.name, b.name));
    n.children.forEach(sortChildren);
  };
  sortChildren(root);
  const byName: Node[] = [root];
  const assign = (scope: Node) => {
    for (const c of scope.children) {
      c.nameIndex = byName.length;
      byName.push(c);
    }
    for (const c of scope.children) if (c.scope) assign(c);
  };
  assign(root);

  // Decision info: qualifiers, qualifier sets and decisions, interned in the order makepri meets them.
  const literals: string[] = [""];
  let literalChars = 1;
  const literalOffset = (value: string): number => {
    let pos = 0;
    for (const s of literals) {
      for (let i = 0; i < s.length; i++) {
        const tail = s.slice(i);
        if (pos + i > 0 && tail[0] === value[0] && tail.toUpperCase() === value.toUpperCase()) return pos + i;
      }
      pos += s.length + 1;
    }
    literals.push(value);
    literalChars += value.length + 1;
    return literalChars - value.length - 1;
  };
  const baseQualifiers: { type: number; offset: number }[] = [];
  const qualifierPool: Qualifier[] = [];
  const references: number[] = [0];
  const sets: { first: number; count: number; key: string }[] = [{ first: 0, count: 0, key: "" }];
  const decisions: { first: number; count: number; key: string }[] = [
    { first: 0, count: 0, key: "-" },
    { first: 0, count: 1, key: "0" },
  ];
  const qualifierIndex = (q: Qualifier): number => {
    const i = qualifierPool.findIndex((p) => p.key === q.key && p.value === q.value);
    if (i >= 0) return i + 1;
    baseQualifiers.push({ type: QUALIFIERS[q.key]!.type, offset: literalOffset(q.value) });
    qualifierPool.push(q);
    return qualifierPool.length;
  };
  const setIndex = (qualifiers: readonly Qualifier[]): number => {
    const ids = qualifiers.map(qualifierIndex);
    const key = ids.join(",");
    const i = sets.findIndex((s) => s.key === key);
    if (i >= 0) return i;
    sets.push({ first: references.length, count: ids.length, key });
    references.push(...ids);
    return sets.length - 1;
  };
  for (const item of items) for (const c of candidates.get(item)!) setIndex(c.qualifiers);

  interface Placed {
    set: number;
    path: string;
    qualifiers: Qualifier[];
  }
  const ordered = new Map<Node, Placed[]>();
  const itemDecision = new Map<Node, number>();
  for (const item of items) {
    const list: Placed[] = [];
    for (const c of candidates.get(item)!) {
      const placed = { set: setIndex(c.qualifiers), path: c.path, qualifiers: c.qualifiers };
      const at = list.findIndex((p) => compareSet(placed.qualifiers, p.qualifiers) >= 0);
      if (at >= 0 && compareSet(placed.qualifiers, list[at]!.qualifiers) === 0)
        throw new Error(`${c.path} and ${list[at]!.path} are the same variant`);
      list.splice(at < 0 ? list.length : at, 0, placed);
    }
    ordered.set(item, list);
    const key = list.map((p) => p.set).join(",");
    let d = decisions.findIndex((x) => x.key === key);
    if (d < 0) {
      decisions.push({ first: references.length, count: list.length, key });
      references.push(...list.map((p) => p.set));
      d = decisions.length - 1;
    }
    itemDecision.set(item, d);
  }

  // One data section per qualifier set, in the order its first candidate is written.
  const dataSections: { set: number; strings: string[] }[] = [];
  const candidateRefs = new Map<Placed, { section: number; item: number }>();
  for (const item of items) {
    for (const c of candidates.get(item)!) {
      const placed = ordered.get(item)!.find((p) => p.path === c.path)!;
      let s = dataSections.findIndex((d) => d.set === placed.set);
      if (s < 0) {
        dataSections.push({ set: placed.set, strings: [] });
        s = dataSections.length - 1;
      }
      candidateRefs.set(placed, { section: s, item: dataSections[s]!.strings.length });
      dataSections[s]!.strings.push(c.path);
    }
  }
  const FIRST_DATA_SECTION = 4;

  const decisionInfo = new Writer();
  decisionInfo.u16(baseQualifiers.length + 1);
  decisionInfo.u16(qualifierPool.length + 1);
  decisionInfo.u16(sets.length);
  decisionInfo.u16(decisions.length);
  decisionInfo.u16(references.length);
  decisionInfo.u16(literalChars);
  for (const d of decisions) {
    decisionInfo.u16(d.first);
    decisionInfo.u16(d.count);
  }
  for (const s of sets) {
    decisionInfo.u16(s.first);
    decisionInfo.u16(s.count);
  }
  for (let i = 0; i <= qualifierPool.length; i++) {
    const q = qualifierPool[i - 1];
    decisionInfo.u16(i);
    decisionInfo.u16(q?.priority ?? 0);
    decisionInfo.u16(q?.score ?? 0);
    decisionInfo.u16(0);
  }
  decisionInfo.raw(new Uint8Array([0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0]));
  for (const b of baseQualifiers) {
    decisionInfo.u16(2);
    decisionInfo.u16(b.type);
    decisionInfo.u16(0);
    decisionInfo.u16(10);
    decisionInfo.u32(b.offset);
  }
  for (const r of references) decisionInfo.u16(r);
  for (const s of literals) decisionInfo.utf16(s);
  decisionInfo.pad(8);

  const descriptor = new Writer();
  descriptor.u16(2);
  descriptor.u16(0xffff);
  descriptor.u16(0);
  descriptor.u16(1);
  descriptor.u16(1);
  descriptor.u16(1);
  descriptor.u16(3);
  descriptor.u16(0);
  descriptor.u16(dataSections.length);
  descriptor.u16(0);
  descriptor.u16(2);
  descriptor.u16(0);
  descriptor.u16(3);
  dataSections.forEach((_, i) => descriptor.u16(FIRST_DATA_SECTION + i));
  descriptor.pad(8);

  const uniqueName = `ms-appx://${packageName}/`;
  const scopePaths = scopes.map((s) => fullPath(s, "/"));
  const itemPaths = items.map((i) => fullPath(i, "/"));
  let checksum = crcString(0, uniqueName);
  checksum = crcString(checksum, packageName);
  checksum = crc32(checksum, le32(1));
  checksum = crcPool(checksum, scopePaths);
  checksum = crcPool(checksum, itemPaths);

  const names = new Writer();
  const pathLength = (n: Node) => fullPath(n, "\\").length;
  const ascii: string[] = [];
  let asciiChars = 1;
  const nameOffset = new Map<Node, number>();
  for (const n of [...scopes.slice(1), ...items]) {
    if (/[^\x20-\x7e]/.test(n.name)) throw new Error(`${n.name}: a resource name must be ASCII`);
    nameOffset.set(n, asciiChars);
    ascii.push(n.name);
    asciiChars += n.name.length + 1;
  }
  names.u16(Math.max(...byName.map(pathLength)));
  names.u16(0);
  names.u32(byName.length);
  names.u32(scopes.length);
  names.u32(items.length);
  names.u32(0);
  const totalAt = names.length;
  names.u32(0);
  names.u32(asciiChars);
  for (const n of byName) {
    names.u16(n.parent?.nameIndex ?? 0);
    names.u16(pathLength(n));
    names.u16(n.name ? n.name[0]!.toUpperCase().charCodeAt(0) : 0);
    names.u8(n.name.length);
    names.u8((n.scope ? 0x10 : 0) | (n.parent ? 0x20 : 0));
    names.u16(nameOffset.get(n) ?? 0);
    names.u16(n.index);
  }
  for (const s of scopes) {
    names.u16(s.nameIndex);
    names.u16(s.children.length);
    names.u16(s.children[0]?.nameIndex ?? 0);
    names.u16(0);
  }
  for (const i of items) names.u16(i.nameIndex);
  names.u8(0);
  for (const s of ascii) names.ascii(s);
  names.pad(8);
  const namesBlock = names.done();
  new DataView(namesBlock.buffer).setUint32(totalAt, namesBlock.length, true);

  const schema = new Writer();
  schema.u16(1);
  schema.u16(uniqueName.length + 1);
  schema.u16(packageName.length + 1);
  schema.u16(0);
  schema.raw(tag("[def_hnamesx]  "));
  schema.u16(1);
  schema.u16(0);
  schema.u32(0);
  schema.u32(checksum);
  schema.u32(scopes.length);
  schema.u32(items.length);
  schema.utf16(uniqueName);
  schema.utf16(packageName);
  schema.pad(4);
  schema.raw(namesBlock);
  schema.pad(8);

  const map = new Writer();
  const candidateCount = items.reduce((n, i) => n + ordered.get(i)!.length, 0);
  map.u16(0);
  map.u16(0);
  map.u16(2);
  map.u16(0);
  map.u16(0);
  map.u16(7);
  map.u16(1);
  map.u16(1);
  map.u32(items.length);
  map.u32(candidateCount);
  map.u32(0);
  map.u32(0);
  for (let t = 0; t < 7; t++) {
    map.u32(4);
    map.u32(t);
  }
  map.u16(0);
  map.u16(0);
  map.u16(items.length);
  map.u16(0);
  let first = 0;
  for (const item of items) {
    map.u16(itemDecision.get(item)!);
    map.u16(first);
    first += ordered.get(item)!.length;
  }
  for (const item of items) {
    for (const p of ordered.get(item)!) {
      const ref = candidateRefs.get(p)!;
      map.u8(1);
      map.u8(5);
      map.u16(0);
      map.u16(ref.item);
      map.u16(FIRST_DATA_SECTION + ref.section);
    }
  }
  map.pad(8);

  const sections: Uint8Array[] = [
    section("[mrm_decn_info]", decisionInfo.done()),
    section("[mrm_pridescex]", descriptor.done()),
    section("[mrm_hschemaex] ", schema.done()),
    section("[mrm_res_map2_]", map.done()),
  ];
  for (const d of dataSections) {
    const data = new Writer();
    const offsets: number[] = [];
    for (const s of d.strings) {
      data.pad(4);
      offsets.push(data.length);
      data.ascii(s);
    }
    const head = 12 + 4 * d.strings.length;
    const padded = Math.ceil((head + data.length) / 8) * 8 - head;
    while (data.length < padded) data.u8(0);
    const w = new Writer();
    w.u32(0);
    w.u16(d.strings.length);
    w.u16(0);
    w.u32(padded);
    d.strings.forEach((s, i) => {
      w.u16(offsets[i]!);
      w.u16(s.length + 1);
    });
    w.raw(data.done());
    sections.push(section("[mrm_dataitem] ", w.done()));
  }

  const out = new Writer();
  const tocAt = 32;
  const start = tocAt + 32 * sections.length;
  const total = start + sections.reduce((n, s) => n + s.length, 0) + 16;
  out.raw(tag("mrm_pri2").subarray(0, 8));
  out.u16(0);
  out.u16(1);
  out.u32(total);
  out.u32(tocAt);
  out.u32(start);
  out.u16(sections.length);
  out.u16(0xffff);
  out.u32(0);
  let offset = 0;
  for (const s of sections) {
    out.raw(s.subarray(0, 16));
    out.u16(0);
    out.u16(0);
    out.u32(0);
    out.u32(offset);
    out.u32(s.length);
    offset += s.length;
  }
  for (const s of sections) out.raw(s);
  out.u32(0xdefffade);
  out.u32(total);
  out.raw(tag("mrm_pri2").subarray(0, 8));
  return out.done();
}
