// Native images as PE sections, written into a copy of a signed system exe and of the container template, and checked
// the way Windows reads them: MapFileAndCheckSumW for the checksum, the loader's own mapping for where each part lands
// and how it is protected, and the unwinder's lookup for a container's code.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType, ptr, toArrayBuffer } from "bun:ffi";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  APP_SECTIONS,
  appendImageSections,
  imageLayout,
  INSPECTOR_SECTIONS,
  PAGE,
  readImageSections,
  withUnwindTable,
} from "../cli/image-sections";

const WHOAMI = "C:\\Windows\\System32\\whoami.exe";
/** crates/sk-image's skimage.dll as built for x64: FileAlignment 0x200, room in its headers for few sections. */
const TEMPLATE =
  "TVqQAAMAAAAEAAAA//8AALgAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAA4fug4AtAnNIbgBTM0hVGhpcyBwcm9ncmFtIGNhbm5vdCBiZSBydW4gaW4gRE9TIG1vZGUuDQ0KJAAAAAAAAAAlAzvfYWJVjGFiVYxhYlWMYWJVjGNiVYzx6VWNYGJVjPHpV41gYlWMUmljaGFiVYwAAAAAAAAAAAAAAAAAAAAAUEUAAGSGAQCgVRFWAAAAAAAAAADwACIgCwIOMgAAAAAAAgAAAAAAAAAAAAAAEAAAAAAAgAEAAAAAEAAAAAIAAAYAAAAAAAAABgAAAAAAAAAAIAAAAAIAAAAAAAACAGABAACAAAAAAAAAABAAAAAAAAAAEAAAAAAAABAAAAAAAAAAAAAAEAAAAEAQAAA0AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAA4AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAALnJkYXRhAAA0AQAAABAAAAACAAAAAgAAAAAAAAAAAAAAAAAAQAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAoFURVgAAAAANAAAAVAAAAIwQAACMAgAAAAAAAKBVEVYAAAAAEAAAACQAAAAQEQAAEAMAAAAAAAAAAAAAAAAAAKBVEVYAAAAAaBAAAAEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHNraW1hZ2UuZGxsABgAAAAAgACAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEAAAQAAAAC5yZGF0YQAAQBAAADQAAAAuZWRhdGEAAHQQAAAYAAAALnJkYXRhJHZvbHRtZAAAAIwQAACoAAAALnJkYXRhJHp6emRiZwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAAACmP7Q9o7uMe5+XBJw9wvaRUp458HXrJnBdVHBMoFURVgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
const isWindows = process.platform === "win32";
const dir = isWindows ? mkdtempSync(join(tmpdir(), "sk-image-sections-")) : "";
afterAll(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
});

/** An image the header describes: `pages` pages, code from page 1 for `code` pages. */
function image(pages: number, code: number, fill: number): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(pages * PAGE).map((_, i) => (i * 7 + fill) & 0xff);
  const view = new DataView(bytes.buffer);
  view.setBigUint64(32, BigInt(bytes.length), true);
  view.setBigUint64(40, BigInt(PAGE), true);
  view.setBigUint64(64, BigInt(code * PAGE), true);
  return bytes;
}

const wide = (s: string) => new Uint8Array(Buffer.from(s + "\0", "utf16le"));

describe.skipIf(!isWindows)("image sections", () => {
  const kernel = isWindows
    ? dlopen("kernel32.dll", {
        LoadLibraryExW: { args: [FFIType.ptr, FFIType.ptr, FFIType.u32], returns: FFIType.ptr },
        FreeLibrary: { args: [FFIType.ptr], returns: FFIType.i32 },
        VirtualQuery: { args: [FFIType.ptr, FFIType.ptr, FFIType.u64], returns: FFIType.u64 },
      })
    : null;
  const ntdll = isWindows
    ? dlopen("ntdll.dll", {
        RtlLookupFunctionEntry: { args: [FFIType.u64, FFIType.ptr, FFIType.ptr], returns: FFIType.ptr },
      })
    : null;
  const imagehlp = isWindows
    ? dlopen("imagehlp.dll", {
        MapFileAndCheckSumW: { args: [FFIType.ptr, FFIType.ptr, FFIType.ptr], returns: FFIType.u32 },
      })
    : null;

  test("two images land page-aligned, each part protected as the engine maps it, in a file Windows checks and maps", () => {
    const app = image(4, 2, 1);
    const inspector = image(3, 2, 9);
    const original = new Uint8Array(readFileSync(WHOAMI));
    const bytes = appendImageSections(appendImageSections(original, app, APP_SECTIONS), inspector, INSPECTOR_SECTIONS);
    const path = join(dir, "whoami-images.exe");
    writeFileSync(path, bytes);

    expect(readImageSections(bytes, APP_SECTIONS)).toEqual(app);
    expect(readImageSections(bytes, INSPECTOR_SECTIONS)).toEqual(inspector);
    expect(readImageSections(original, APP_SECTIONS)).toBeNull();

    const header = new Uint32Array(1);
    const computed = new Uint32Array(1);
    expect(imagehlp!.symbols.MapFileAndCheckSumW(ptr(wide(path)), ptr(header), ptr(computed))).toBe(0);
    expect(header[0]).toBe(computed[0]!);

    const module = kernel!.symbols.LoadLibraryExW(ptr(wide(path)), null, 0x20); // LOAD_LIBRARY_AS_IMAGE_RESOURCE
    expect(module).toBeTruthy();
    try {
      const base = Number(module) - (Number(module) % 4); // the low bits flag a resource mapping
      const view = new DataView(bytes.buffer, bytes.byteOffset);
      const pe = view.getUint32(0x3c, true);
      const table = pe + 24 + view.getUint16(pe + 20, true);
      const sections = Array.from({ length: view.getUint16(pe + 6, true) }, (_, i) => {
        const at = table + i * 40;
        return {
          name: new TextDecoder().decode(bytes.subarray(at, at + 8)).replace(/\0+$/, ""),
          address: view.getUint32(at + 12, true),
          size: view.getUint32(at + 8, true),
          raw: view.getUint32(at + 20, true),
        };
      });
      const find = (name: string) => sections.find((s) => s.name === name)!;
      const protect = (address: number) => {
        const info = new BigUint64Array(6);
        kernel!.symbols.VirtualQuery(base + address, ptr(info), 48);
        return Number(info[4]! >> 32n);
      };
      for (const [names, bytesOf] of [
        [APP_SECTIONS, app],
        [INSPECTOR_SECTIONS, inspector],
      ] as const) {
        const first = find(names.header);
        expect(first.raw % PAGE).toBe(0);
        expect(first.address % PAGE).toBe(0);
        const layout = imageLayout(bytesOf);
        expect(find(names.code).address).toBe(first.address + layout.codeOffset);
        expect(find(names.code).raw).toBe(first.raw + layout.codeOffset);
        const mapped = new Uint8Array(toArrayBuffer(base + first.address, 0, layout.size));
        expect(mapped).toEqual(bytesOf);
        // A resource mapping keeps the image's layout but not its execute right: read-only all through.
        expect(protect(first.address)).toBe(0x02);
      }
    } finally {
      kernel!.symbols.FreeLibrary(module);
    }
  });

  test("a full section table grows the headers and moves every file offset behind them", () => {
    let bytes: Uint8Array = new Uint8Array(Buffer.from(TEMPLATE, "base64"));
    // Enough images that the section table outgrows the template's 512 bytes of headers twice.
    for (let i = 0; i < 6; i++) {
      const names = { header: `.ska${i}`, code: `.ska${i}x`, data: `.ska${i}d` };
      bytes = appendImageSections(bytes, image(3, 1, i), names);
      expect(readImageSections(bytes, names)).toEqual(image(3, 1, i));
    }
    // The engine maps an image from its file offset, so growing the headers must not shift one off a page.
    const view = new DataView(bytes.buffer, bytes.byteOffset);
    const pe = view.getUint32(0x3c, true);
    const table = pe + 24 + view.getUint16(pe + 20, true);
    for (let i = 0; i < view.getUint16(pe + 6, true); i++) {
      const at = table + i * 40;
      if (new TextDecoder().decode(bytes.subarray(at, at + 4)) === ".ska")
        expect(view.getUint32(at + 20, true) % PAGE).toBe(0);
    }
    const path = join(dir, "template-grown.dll");
    writeFileSync(path, bytes);
    const header = new Uint32Array(1);
    const computed = new Uint32Array(1);
    expect(imagehlp!.symbols.MapFileAndCheckSumW(ptr(wide(path)), ptr(header), ptr(computed))).toBe(0);
    expect(header[0]).toBe(computed[0]!);
    const module = kernel!.symbols.LoadLibraryExW(ptr(wide(path)), null, 0x20);
    expect(module).toBeTruthy();
    kernel!.symbols.FreeLibrary(module);
    expect(() => appendImageSections(bytes, image(3, 1, 0), { header: ".ska0", code: ".x", data: ".y" })).toThrow(
      /already has/,
    );
  });

  test("an image its header does not describe is refused", () => {
    const short = image(3, 1, 0).subarray(0, 2 * PAGE);
    expect(() => appendImageSections(new Uint8Array(readFileSync(WHOAMI)), short, APP_SECTIONS)).toThrow(
      /does not describe/,
    );
  });

  test("a container the loader maps keeps each part's protection and tells the unwinder about its code", () => {
    const app = image(4, 2, 3);
    const bytes = withUnwindTable(
      appendImageSections(new Uint8Array(Buffer.from(TEMPLATE, "base64")), app, APP_SECTIONS),
      [APP_SECTIONS],
    );
    const path = join(dir, "container.dll");
    writeFileSync(path, bytes);
    const header = new Uint32Array(1);
    const computed = new Uint32Array(1);
    expect(imagehlp!.symbols.MapFileAndCheckSumW(ptr(wide(path)), ptr(header), ptr(computed))).toBe(0);
    expect(header[0]).toBe(computed[0]!);
    // A real load, as the host does it: the container has no entry point and no imports.
    const module = kernel!.symbols.LoadLibraryExW(ptr(wide(path)), null, 0x800); // LOAD_LIBRARY_SEARCH_SYSTEM32
    expect(module).toBeTruthy();
    try {
      const base = Number(module);
      const pe = new DataView(bytes.buffer, bytes.byteOffset).getUint32(0x3c, true);
      const table = pe + 24 + new DataView(bytes.buffer, bytes.byteOffset).getUint16(pe + 20, true);
      const view = new DataView(bytes.buffer, bytes.byteOffset);
      const address = (name: string) => {
        for (let i = 0; i < view.getUint16(pe + 6, true); i++) {
          const at = table + i * 40;
          if (new TextDecoder().decode(bytes.subarray(at, at + 8)).replace(/ +$/, "") === name)
            return view.getUint32(at + 12, true);
        }
        throw new Error(name);
      };
      const protect = (rva: number) => {
        const info = new BigUint64Array(6);
        kernel!.symbols.VirtualQuery(base + rva, ptr(info), 48);
        return Number(info[4]! >> 32n);
      };
      expect(protect(address(APP_SECTIONS.header))).toBe(0x02);
      expect(protect(address(APP_SECTIONS.code))).toBe(0x20);
      expect(protect(address(APP_SECTIONS.data))).toBe(0x02);
      expect(new Uint8Array(toArrayBuffer(base + address(APP_SECTIONS.header), 0, app.length))).toEqual(app);
      const imageBase = new BigUint64Array(1);
      const entry = ntdll!.symbols.RtlLookupFunctionEntry(
        BigInt(base + address(APP_SECTIONS.code) + 100),
        ptr(imageBase),
        null,
      );
      expect(entry).toBeTruthy();
      expect(Number(imageBase[0])).toBe(base);
      expect(new Uint32Array(toArrayBuffer(entry!, 0, 12))[0]).toBe(address(APP_SECTIONS.code));
      expect(
        ntdll!.symbols.RtlLookupFunctionEntry(BigInt(base + address(APP_SECTIONS.header)), ptr(imageBase), null),
      ).toBeNull();
    } finally {
      kernel!.symbols.FreeLibrary(module);
    }
  });

  test("an exe keeps its own unwind entries and gains the images' code", () => {
    const original = new Uint8Array(readFileSync(WHOAMI));
    const images: [typeof APP_SECTIONS, Uint8Array][] = [
      [APP_SECTIONS, image(4, 2, 5)],
      [INSPECTOR_SECTIONS, image(3, 1, 6)],
    ];
    let bytes: Uint8Array = original;
    for (const [names, bytesOf] of images) bytes = appendImageSections(bytes, bytesOf, names);
    bytes = withUnwindTable(
      bytes,
      images.map(([names]) => names),
    );
    const path = join(dir, "whoami-unwind.exe");
    writeFileSync(path, bytes);
    const header = new Uint32Array(1);
    const computed = new Uint32Array(1);
    expect(imagehlp!.symbols.MapFileAndCheckSumW(ptr(wide(path)), ptr(header), ptr(computed))).toBe(0);
    expect(header[0]).toBe(computed[0]!);
    const view = new DataView(bytes.buffer, bytes.byteOffset);
    const pe = view.getUint32(0x3c, true);
    const table = pe + 24 + view.getUint16(pe + 20, true);
    const address = (name: string) => {
      for (let i = 0; i < view.getUint16(pe + 6, true); i++) {
        const at = table + i * 40;
        if (new TextDecoder().decode(bytes.subarray(at, at + 8)).replace(/\0+$/, "") === name)
          return view.getUint32(at + 12, true);
      }
      throw new Error(name);
    };
    expect(view.getUint32(pe + 24 + 112 + 3 * 8, true)).toBe(address(".skpdata"));
    const module = kernel!.symbols.LoadLibraryExW(ptr(wide(path)), null, 0x1); // DONT_RESOLVE_DLL_REFERENCES
    expect(module).toBeTruthy();
    try {
      const base = Number(module);
      const imageBase = new BigUint64Array(1);
      const begin = (rva: number) => {
        const entry = ntdll!.symbols.RtlLookupFunctionEntry(BigInt(base + rva), ptr(imageBase), null);
        return entry ? new Uint32Array(toArrayBuffer(entry, 0, 12))[0] : null;
      };
      for (const [names] of images) expect(begin(address(names.code) + 100)).toBe(address(names.code));
      const pdata = address(".pdata");
      const own = new Uint32Array(toArrayBuffer(base + pdata, 0, 12));
      expect(begin(own[0]!)).toBe(own[0]!);
    } finally {
      kernel!.symbols.FreeLibrary(module);
    }
  });
});
