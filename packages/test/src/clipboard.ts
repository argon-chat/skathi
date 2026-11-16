import { dlopen, FFIType, ptr, toArrayBuffer } from "bun:ffi";

const open = () => ({
  user32: dlopen("user32.dll", {
    OpenClipboard: { args: [FFIType.ptr], returns: FFIType.i32 },
    CloseClipboard: { args: [], returns: FFIType.i32 },
    EmptyClipboard: { args: [], returns: FFIType.i32 },
    EnumClipboardFormats: { args: [FFIType.u32], returns: FFIType.u32 },
    GetClipboardData: { args: [FFIType.u32], returns: FFIType.ptr },
    SetClipboardData: { args: [FFIType.u32, FFIType.ptr], returns: FFIType.ptr },
    IsClipboardFormatAvailable: { args: [FFIType.u32], returns: FFIType.i32 },
    RegisterClipboardFormatW: { args: [FFIType.ptr], returns: FFIType.u32 },
  }),
  kernel32: dlopen("kernel32.dll", {
    GlobalAlloc: { args: [FFIType.u32, FFIType.u64], returns: FFIType.ptr },
    GlobalLock: { args: [FFIType.ptr], returns: FFIType.ptr },
    GlobalUnlock: { args: [FFIType.ptr], returns: FFIType.i32 },
    GlobalSize: { args: [FFIType.ptr], returns: FFIType.u64 },
    GlobalFree: { args: [FFIType.ptr], returns: FFIType.ptr },
    RtlMoveMemory: { args: [FFIType.ptr, FFIType.ptr, FFIType.u64], returns: FFIType.void },
  }),
});
// Opened on first use: importing the harness must work where there is no Win32 clipboard.
let libs: ReturnType<typeof open> | null = null;
const lib = () => (libs ??= open());

const GMEM_MOVEABLE = 0x0002;
/** Formats whose data is a GDI handle, not a memory block. */
const HANDLE_FORMATS = new Set([
  2 /* BITMAP */, 3 /* METAFILEPICT */, 9 /* PALETTE */, 14 /* ENHMETAFILE */, 0x80, 0x82, 0x83, 0x8e,
]);
const isHandleFormat = (f: number) => HANDLE_FORMATS.has(f) || (f >= 0x200 && f <= 0x3ff);

export const CF_UNICODETEXT = 13;
export const CF_DIB = 8;
export const CF_DIBV5 = 17;

export type ClipboardSnapshot = { format: number; bytes: Uint8Array }[];

function wide(s: string): Uint16Array {
  const out = new Uint16Array(s.length + 1);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

/** A registered format's id by name ("PNG", "HTML Format"). */
export function clipboardFormatId(name: string): number {
  return lib().user32.symbols.RegisterClipboardFormatW(ptr(wide(name)));
}

/** Runs `f` with the clipboard open; another process may hold it for a moment. */
function withOpen<T>(f: () => T): T {
  for (let i = 0; i < 50; i++) {
    if (lib().user32.symbols.OpenClipboard(null)) {
      try {
        return f();
      } finally {
        lib().user32.symbols.CloseClipboard();
      }
    }
    Bun.sleepSync(10);
  }
  throw new Error("the clipboard stayed held by another process");
}

function readOpen(format: number): Uint8Array | null {
  if (!lib().user32.symbols.IsClipboardFormatAvailable(format)) return null;
  const h = lib().user32.symbols.GetClipboardData(format);
  if (!h) return null;
  const size = Number(lib().kernel32.symbols.GlobalSize(h));
  const p = lib().kernel32.symbols.GlobalLock(h);
  if (!p) return null;
  try {
    return size > 0 ? new Uint8Array(toArrayBuffer(p, 0, size)).slice() : new Uint8Array(0);
  } finally {
    lib().kernel32.symbols.GlobalUnlock(h);
  }
}

/** One format's bytes as the clipboard has them now, by id or registered name; `null` if absent. */
export function clipboardFormat(format: number | string): Uint8Array | null {
  const id = typeof format === "string" ? clipboardFormatId(format) : format;
  return withOpen(() => readOpen(id));
}

/** Everything on the clipboard that is a memory block, to put back with `clipboardRestore`. */
export function clipboardSnapshot(): ClipboardSnapshot {
  return withOpen(() => {
    const out: ClipboardSnapshot = [];
    for (
      let f = lib().user32.symbols.EnumClipboardFormats(0);
      f !== 0;
      f = lib().user32.symbols.EnumClipboardFormats(f)
    ) {
      if (isHandleFormat(f)) continue;
      const bytes = readOpen(f);
      if (bytes) out.push({ format: f, bytes });
    }
    return out;
  });
}

/** Replaces the clipboard with a snapshot (an empty one empties it). */
export function clipboardRestore(snapshot: ClipboardSnapshot): void {
  withOpen(() => {
    lib().user32.symbols.EmptyClipboard();
    for (const { format, bytes } of snapshot) {
      const h = lib().kernel32.symbols.GlobalAlloc(GMEM_MOVEABLE, Math.max(1, bytes.length));
      if (!h) continue;
      const p = lib().kernel32.symbols.GlobalLock(h);
      if (p && bytes.length) lib().kernel32.symbols.RtlMoveMemory(p, ptr(bytes), bytes.length);
      lib().kernel32.symbols.GlobalUnlock(h);
      if (!lib().user32.symbols.SetClipboardData(format, h)) lib().kernel32.symbols.GlobalFree(h);
    }
  });
}

/** A DIB's header, as another app reads it: size, depth, and the compression. */
export function dibHeader(dib: Uint8Array): {
  headerSize: number;
  width: number;
  height: number;
  bits: number;
  compression: number;
} {
  const v = new DataView(dib.buffer, dib.byteOffset, dib.byteLength);
  return {
    headerSize: v.getUint32(0, true),
    width: v.getInt32(4, true),
    height: v.getInt32(8, true),
    bits: v.getUint16(14, true),
    compression: v.getUint32(16, true),
  };
}
