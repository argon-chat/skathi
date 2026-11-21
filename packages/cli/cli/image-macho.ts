export const MACHO_TEMPLATE_NAME = "libskimage.dylib";

const PAGE = 16 * 1024;
const HEADER_SIZE = 32;
const SEGMENT = 0x19;
const SEGMENT_SIZE = 72;
const SECTION_SIZE = 80;
const UUID = 0x1b;
const SIGNATURE = 0x1d;
const align = (value: number, alignment: number) => Math.ceil(value / alignment) * alignment;

function view(bytes: Uint8Array) {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function readName(bytes: Uint8Array, offset: number) {
  const name = bytes.subarray(offset, offset + 16);
  const end = name.indexOf(0);
  return new TextDecoder().decode(end < 0 ? name : name.subarray(0, end));
}

function writeName(bytes: Uint8Array, offset: number, name: string) {
  bytes.set(new TextEncoder().encode(name), offset);
}

function layout(image: Uint8Array) {
  if (image.length < 332) throw new Error("the native AOT image is missing its header");
  const header = view(image);
  const size = Number(header.getBigUint64(32, true));
  const code = Number(header.getBigUint64(40, true));
  const executable = Number(header.getBigUint64(64, true));
  if (
    header.getBigUint64(0, true) !== 0x3130544f414e5542n ||
    header.getUint32(72, true) !== 5 ||
    header.getUint8(90) !== 0 ||
    header.getUint8(93) !== 1 ||
    size !== image.length ||
    size % PAGE ||
    !code ||
    code % PAGE ||
    !executable ||
    executable % PAGE ||
    code + executable > size
  ) {
    throw new Error("the native AOT image is not a page-aligned position-independent ARM64 image from this sk-aot");
  }
  return { code, executable };
}

function commands(bytes: Uint8Array): Uint8Array[] {
  const header = view(bytes);
  if (
    bytes.length < HEADER_SIZE ||
    header.getUint32(0, true) !== 0xfeedfacf ||
    header.getUint32(4, true) !== 0x0100000c ||
    header.getUint32(8, true) !== 0 ||
    header.getUint32(12, true) !== 6
  ) {
    throw new Error("the native AOT container is not an arm64 Mach-O dylib");
  }
  const end = HEADER_SIZE + header.getUint32(20, true);
  if (end > bytes.length || end > PAGE) throw new Error("Mach-O load commands run past the header");
  const count = header.getUint32(16, true);
  const result: Uint8Array[] = [];
  let offset = HEADER_SIZE;
  for (let index = 0; index < count; index++) {
    if (offset + 8 > end) throw new Error("truncated Mach-O load command");
    const length = header.getUint32(offset + 4, true);
    if (length < 8 || length % 8 || offset + length > end) throw new Error("invalid Mach-O load command size");
    const command = bytes.slice(offset, offset + length);
    if (view(command).getUint32(0, true) === SEGMENT && length < SEGMENT_SIZE) {
      throw new Error("truncated Mach-O segment command");
    }
    result.push(command);
    offset += length;
  }
  if (offset !== end) throw new Error("Mach-O load commands do not fill the header");
  return result;
}

function section(segment: string, name: string, offset: number, size: number, alignment: number, flags = 0) {
  const bytes = new Uint8Array(SECTION_SIZE);
  const fields = view(bytes);
  writeName(bytes, 0, name);
  writeName(bytes, 16, segment);
  fields.setBigUint64(32, BigInt(offset), true);
  fields.setBigUint64(40, BigInt(size), true);
  fields.setUint32(48, offset, true);
  fields.setUint32(52, alignment, true);
  fields.setUint32(64, flags, true);
  return bytes;
}

function imageSegment(name: string, offset: number, size: number, executable = false) {
  const bytes = new Uint8Array(SEGMENT_SIZE + SECTION_SIZE);
  const fields = view(bytes);
  fields.setUint32(0, SEGMENT, true);
  fields.setUint32(4, bytes.length, true);
  writeName(bytes, 8, name);
  fields.setBigUint64(24, BigInt(offset), true);
  fields.setBigUint64(32, BigInt(size), true);
  fields.setBigUint64(40, BigInt(offset), true);
  fields.setBigUint64(48, BigInt(size), true);
  fields.setUint32(56, executable ? 5 : 1, true);
  fields.setUint32(60, executable ? 5 : 1, true);
  fields.setUint32(64, 1, true);
  bytes.set(
    section(name, executable ? "__aot_code" : "__aot_data", offset, size, 14, executable ? 0x80000400 : 0),
    SEGMENT_SIZE,
  );
  return bytes;
}

function linkeditOffsets(type: number): readonly number[] {
  switch (type) {
    case 0x2: // LC_SYMTAB
      return [8, 16];
    case 0xb: // LC_DYSYMTAB
      return [32, 40, 48, 56, 64, 72];
    case 0x22: // LC_DYLD_INFO
    case 0x80000022:
      return [8, 16, 24, 32, 40];
    case 0x26: // LC_FUNCTION_STARTS
    case 0x29: // LC_DATA_IN_CODE
    case 0x2e: // LC_LINKER_OPTIMIZATION_HINT
    case 0x80000033: // LC_DYLD_EXPORTS_TRIE
    case 0x80000034: // LC_DYLD_CHAINED_FIXUPS
      return [8];
    case 0xc: // LC_LOAD_DYLIB
    case 0xd: // LC_ID_DYLIB
    case UUID:
    case 0x24: // LC_VERSION_MIN_MACOSX
    case 0x2a: // LC_SOURCE_VERSION
    case 0x32: // LC_BUILD_VERSION
      return [];
    default:
      throw new Error(`unexpected Mach-O template load command 0x${type.toString(16)}`);
  }
}

function functionStarts(image: DataView, codeSize: number): Set<number> {
  // ImageHeader.sections[FunctionStarts] and ImageHeader.counts.functions (format 5).
  const offset = image.getUint32(120, true);
  const size = image.getUint32(124, true);
  const count = image.getUint32(328, true);
  const groups = offset + Math.floor((count + 2) / 2) * 4;
  const end = offset + size;
  if (end > image.byteLength || groups + 4 * Math.ceil(count / 32) > end) throw new Error("invalid AOT function table");
  const starts = new Set<number>();
  for (let index = 0; index < count; index++) {
    const group = image.getUint32(groups + Math.floor(index / 32) * 4, true);
    let start: number;
    if (group >= 0x80000000) {
      const entry = groups + (group - 0x80000000 + (index % 32)) * 4;
      if (entry + 4 > end) throw new Error("invalid AOT function group");
      start = image.getUint32(entry, true);
    } else {
      start = group + image.getUint16(offset + index * 2, true);
    }
    if (start > codeSize || start % 4) throw new Error("invalid AOT function start");
    starts.add(start);
  }
  return starts;
}

function arm64Unwind(bytes: Uint8Array, code: number, executable: number) {
  const image = view(bytes);
  const starts = functionStarts(image, executable);
  const frame = 0x04000000;
  const frameless = 0x02000000;
  const rows: Array<[number, number]> = [];
  const emit = (offset: number, encoding: number) => {
    const previous = rows.at(-1);
    if (previous?.[0] === offset) previous[1] = encoding;
    else if (previous?.[1] !== encoding) rows.push([offset, encoding]);
  };
  // Match the backend's x29/x30 record. Other callee saves have frame-specific offsets.
  // Mark a restored frame through the return or tail branch, then resume its out-of-line code.
  let hasFrame = false;
  let leavingFrame = false;
  for (let offset = 0; offset < executable; offset += 4) {
    if (starts.has(offset)) {
      hasFrame = false;
      leavingFrame = false;
    }
    emit(offset, hasFrame && !leavingFrame ? frame : frameless);
    const instruction = image.getUint32(code + offset, true);
    if (
      instruction === 0xa9bf7bfd &&
      offset + 8 < executable &&
      image.getUint32(code + offset + 4, true) === 0x910003fd
    ) {
      hasFrame = true;
      leavingFrame = false;
      offset += 4;
    } else if (hasFrame && instruction === 0xa8c17bfd) {
      leavingFrame = true;
    } else if (
      leavingFrame &&
      ((instruction & 0x7c000000) === 0x14000000 ||
        (instruction & 0xffff0000) >>> 0 === 0xd61f0000 ||
        (instruction & 0xffff0000) >>> 0 === 0xd65f0000)
    ) {
      leavingFrame = false;
    }
  }
  // __unwind_info version 1, with regular second-level pages and no personalities or LSDA.
  const perPage = 511;
  const pages = Math.ceil(rows.length / perPage);
  const indexStart = 28;
  const pagesStart = indexStart + (pages + 1) * 12;
  const table = new Uint8Array(pagesStart + pages * 8 + rows.length * 8);
  const fields = view(table);
  const addresses: number[] = [];
  fields.setUint32(0, 1, true);
  fields.setUint32(4, indexStart, true);
  fields.setUint32(12, indexStart, true);
  fields.setUint32(20, indexStart, true);
  fields.setUint32(24, pages + 1, true);
  let pageStart = pagesStart;
  for (let page = 0; page < pages; page++) {
    const index = indexStart + page * 12;
    const entries = rows.slice(page * perPage, (page + 1) * perPage);
    fields.setUint32(index, entries[0]![0], true);
    addresses.push(index);
    fields.setUint32(index + 4, pageStart, true);
    fields.setUint32(index + 8, pagesStart, true);
    fields.setUint32(pageStart, 2, true);
    fields.setUint16(pageStart + 4, 8, true);
    fields.setUint16(pageStart + 6, entries.length, true);
    pageStart += 8;
    for (const [offset, encoding] of entries) {
      fields.setUint32(pageStart, offset, true);
      addresses.push(pageStart);
      fields.setUint32(pageStart + 4, encoding, true);
      pageStart += 8;
    }
  }
  const sentinel = indexStart + pages * 12;
  fields.setUint32(sentinel, executable, true);
  addresses.push(sentinel);
  fields.setUint32(sentinel + 8, pagesStart, true);
  return { bytes: table, addresses, length: table.length };
}

/** Returns a deterministic dylib. Sign the result after writing it. */
export function appendMachOImage(template: Uint8Array, image: Uint8Array): Uint8Array {
  const { code, executable } = layout(image);
  const loadCommands = commands(template);
  const segments = loadCommands.filter((c) => view(c).getUint32(0, true) === SEGMENT);
  const [text, linkedit] = segments;
  if (
    segments.length !== 2 ||
    !text ||
    readName(text, 8) !== "__TEXT" ||
    !linkedit ||
    readName(linkedit, 8) !== "__LINKEDIT"
  ) {
    throw new Error("the Mach-O template must contain only __TEXT and __LINKEDIT");
  }
  const textFields = view(text);
  const linkFields = view(linkedit);
  const textSize = Number(textFields.getBigUint64(48, true));
  const linkOffset = Number(linkFields.getBigUint64(40, true));
  let linkEnd = linkOffset + Number(linkFields.getBigUint64(48, true));
  if (
    textFields.getBigUint64(24, true) !== 0n ||
    textFields.getBigUint64(40, true) !== 0n ||
    !textSize ||
    textSize % PAGE ||
    textSize !== Number(textFields.getBigUint64(32, true)) ||
    linkOffset !== textSize ||
    linkOffset !== Number(linkFields.getBigUint64(24, true)) ||
    linkEnd !== template.length
  ) {
    throw new Error("the Mach-O template's segments are not contiguous and page-aligned");
  }
  const count = textFields.getUint32(64, true);
  if (text.length !== SEGMENT_SIZE + count * SECTION_SIZE) throw new Error("invalid Mach-O template section table");
  let sectionOffset = textSize;
  for (let i = 0; i < count; i++) {
    const at = SEGMENT_SIZE + i * SECTION_SIZE;
    if (textFields.getUint32(at + 64, true) & 0xff) throw new Error("unsupported Mach-O template section type");
    if (textFields.getBigUint64(at + 40, true))
      sectionOffset = Math.min(sectionOffset, textFields.getUint32(at + 48, true));
  }
  const unwind = arm64Unwind(image, code, executable);
  const imageOffset = align(textSize + unwind.length, PAGE);
  const codeOffset = imageOffset + code;
  if (imageOffset + image.length > 0xffffffff)
    throw new Error("the native AOT image exceeds the Mach-O container size");
  // Unwind offsets are relative to the Mach-O header. Reserve output space before adjusting them.
  const unwindFields = view(unwind.bytes);
  for (const offset of unwind.addresses) {
    unwindFields.setUint32(offset, unwindFields.getUint32(offset, true) + codeOffset, true);
  }
  const unwindText = new Uint8Array(text.length + SECTION_SIZE);
  unwindText.set(text);
  const unwindTextFields = view(unwindText);
  unwindTextFields.setUint32(4, unwindText.length, true);
  unwindTextFields.setBigUint64(32, BigInt(imageOffset), true);
  unwindTextFields.setBigUint64(48, BigInt(imageOffset), true);
  unwindTextFields.setUint32(64, count + 1, true);
  unwindText.set(section("__TEXT", "__unwind_info", textSize, unwind.length, 2), text.length);
  const dataOffset = code + executable;
  const imageCommands = [
    imageSegment("__SKHDR", imageOffset, code),
    imageSegment("__SKCODE", codeOffset, executable, true),
  ];
  if (dataOffset < image.length)
    imageCommands.push(imageSegment("__SKDATA", imageOffset + dataOffset, image.length - dataOffset));
  const newLinkOffset = imageOffset + image.length;
  const delta = newLinkOffset - linkOffset;
  const rewritten: Uint8Array[] = [];
  for (const command of loadCommands) {
    const fields = view(command);
    const type = fields.getUint32(0, true);
    if (command === text) {
      rewritten.push(unwindText, ...imageCommands);
      continue;
    }
    if (command === linkedit) {
      fields.setBigUint64(24, BigInt(newLinkOffset), true);
      fields.setBigUint64(40, BigInt(newLinkOffset), true);
    } else if (type === SIGNATURE) {
      const at = fields.getUint32(8, true);
      if (at < linkOffset || at + fields.getUint32(12, true) !== linkEnd)
        throw new Error("invalid Mach-O template signature");
      linkEnd = at;
      continue;
    } else {
      for (const field of linkeditOffsets(type)) {
        const offset = fields.getUint32(field, true);
        if (offset) {
          if (offset < linkOffset || offset > linkEnd || offset + delta > 0xffffffff)
            throw new Error("invalid Mach-O template linkedit offset");
          fields.setUint32(field, offset + delta, true);
        }
      }
      if (type === UUID) {
        const hash = new Bun.CryptoHasher("sha256").update(template).update(image).digest();
        command.set(hash.subarray(0, 16), 8);
        command[14] = (command[14]! & 0x0f) | 0x40;
        command[16] = (command[16]! & 0x3f) | 0x80;
      }
    }
    rewritten.push(command);
  }
  linkFields.setBigUint64(48, BigInt(linkEnd - linkOffset), true);
  const commandsLength = rewritten.reduce((size, command) => size + command.length, 0);
  if (HEADER_SIZE + commandsLength > sectionOffset)
    throw new Error("the Mach-O template lacks space for native AOT load commands");

  const bytes = new Uint8Array(newLinkOffset + linkEnd - linkOffset);
  bytes.set(template.subarray(0, textSize));
  bytes.fill(0, HEADER_SIZE, sectionOffset);
  const header = view(bytes);
  header.setUint32(16, rewritten.length, true);
  header.setUint32(20, commandsLength, true);
  let offset = HEADER_SIZE;
  for (const command of rewritten) {
    bytes.set(command, offset);
    offset += command.length;
  }
  bytes.set(unwind.bytes, textSize);
  bytes.set(image, imageOffset);
  bytes.set(template.subarray(linkOffset, linkEnd), newLinkOffset);
  return bytes;
}

/** Reads the image bytes from an unsigned or signed dylib. */
export function readMachOImage(bytes: Uint8Array): Uint8Array | null {
  const header = commands(bytes).find((c) => view(c).getUint32(0, true) === SEGMENT && readName(c, 8) === "__SKHDR");
  if (!header) return null;
  const offset = Number(view(header).getBigUint64(40, true));
  if (!offset || offset % PAGE || offset + 40 > bytes.length) throw new Error("invalid Mach-O native AOT image offset");
  const size = Number(view(bytes).getBigUint64(offset + 32, true));
  if (offset + size > bytes.length) throw new Error("the Mach-O native AOT image exceeds the container size");
  const image = bytes.subarray(offset, offset + size);
  layout(image);
  return image;
}
