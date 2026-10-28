// File System Access, FileReader, the pickers, `<input type=file>` and
// `<a download>` — the JS half of docs/DESKTOP.md "Pickers and files".
//
// The Rust half (the name fence, the swap and atomic replace, the cursor, the
// dialogs, the scripted backend) is unit-tested in crates/vn-desktop, and
// packages/vn-test/specs/files.spec.ts drives both through a real host with
// VN_DIALOGS=scripted. What is pinned here is what only the runtime can get
// wrong: the spec's option checks and exception names, transient activation,
// the FileReader state machine and event order, that a handle holds no path,
// and the input element's events.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
type Call = { op: string; args: unknown[]; token: number | null };

const calls = (rt: Rt, op: string): Call[] => rt.host.desktop.fs.calls.filter((c: Call) => c.op === op);
const tick = () => new Promise((r) => setTimeout(r, 0));
/** Lets the reader's task chain (setTimeout 0, a promise, setTimeout 0) run out. */
const settleReader = async () => {
  for (let i = 0; i < 6; i++) await tick();
};
/** A user gesture, as the host delivers one (only these activate). */
const gesture = (rt: Rt) => rt.noteActivation("mousedown", undefined);

/** A handle the host granted (a launch path, a drop). */
const handle = (rt: Rt, h: number, name: string, kind: "file" | "directory") =>
  rt.desktopApi.handleFrom({ h, name, kind }) as Record<string, any>;

describe("FileReader (File API)", () => {
  const reader = (rt: Rt) => new rt.global.FileReader();

  test("is installed with its constants, on the class and on instances", () => {
    const rt = createRuntime();
    const FR = rt.global.FileReader;
    expect([FR.EMPTY, FR.LOADING, FR.DONE]).toEqual([0, 1, 2]);
    const r = reader(rt);
    expect([r.EMPTY, r.LOADING, r.DONE, r.readyState, r.result, r.error]).toEqual([0, 1, 2, 0, null, null]);
    expect(r).toBeInstanceOf(EventTarget);
    expect(typeof rt.global.ProgressEvent).toBe("function");
  });

  test("events come in the spec's order, with readyState and result as each one sees them", async () => {
    const rt = createRuntime();
    const r = reader(rt);
    const seen: string[] = [];
    for (const t of ["loadstart", "progress", "load", "loadend"]) {
      r.addEventListener(t, (e: any) => seen.push(`${t}:${r.readyState}:${r.result === null ? "null" : "set"}:${e.loaded}`));
    }
    r.readAsText(new Blob(["héllo"]));
    // Synchronously: LOADING, and nothing has fired yet (every event is a task).
    expect([r.readyState, seen.length]).toEqual([1, 0]);
    await settleReader();
    expect(seen).toEqual(["loadstart:1:null:0", "progress:1:null:6", "load:2:set:6", "loadend:2:set:6"]);
    expect(r.result).toBe("héllo");
  });

  test("on<type> attributes fire once each, beside listeners", async () => {
    const rt = createRuntime();
    const r = reader(rt);
    let n = 0;
    r.onload = () => n++;
    r.onload = () => (n += 10); // replaced, not added
    r.readAsArrayBuffer(new Blob([new Uint8Array([1, 2, 3])]));
    await settleReader();
    expect(n).toBe(10);
    expect(r.onload).toBeInstanceOf(Function);
    expect(new Uint8Array(r.result)).toEqual(new Uint8Array([1, 2, 3]));
  });

  test("readAsText honours the label, the Blob's charset and a BOM, in that order of priority", async () => {
    const rt = createRuntime();
    const text = async (blob: Blob, enc?: string) => {
      const r = reader(rt);
      r.readAsText(blob, enc);
      await settleReader();
      return r.result;
    };
    const cp1251 = new Uint8Array([0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2]); // "Привет"
    expect(await text(new Blob([cp1251]), "windows-1251")).toBe("Привет");
    expect(await text(new Blob([cp1251], { type: "text/plain;charset=windows-1251" }))).toBe("Привет");
    // An unknown label falls back to the type's charset, then UTF-8.
    expect(await text(new Blob([cp1251], { type: "text/plain;charset=windows-1251" }), "no-such-label")).toBe("Привет");
    // A BOM wins over everything.
    expect(await text(new Blob([new Uint8Array([0xff, 0xfe, 0x41, 0x00])]), "windows-1251")).toBe("A");
    expect(await text(new Blob([new Uint8Array([0xef, 0xbb, 0xbf, 0x41])]))).toBe("A");
  });

  test("readAsDataURL and readAsBinaryString package as Chromium does", async () => {
    const rt = createRuntime();
    const r = reader(rt);
    // (Not text/plain: bun's Blob appends `;charset=utf-8` to that one.)
    r.readAsDataURL(new Blob(["hi"], { type: "image/png" }));
    await settleReader();
    expect(r.result).toBe("data:image/png;base64,aGk=");
    r.readAsDataURL(new Blob(["hi"]));
    await settleReader();
    expect(r.result).toBe("data:application/octet-stream;base64,aGk=");
    r.readAsBinaryString(new Blob([new Uint8Array([0, 255])]));
    await settleReader();
    expect(r.result).toBe("\u0000ÿ");
  });

  test("a second read while loading is InvalidStateError; a non-Blob is TypeError", () => {
    const rt = createRuntime();
    const r = reader(rt);
    expect(() => r.readAsText("not a blob")).toThrow(TypeError);
    r.readAsText(new Blob(["x"]));
    let err: any;
    try {
      r.readAsArrayBuffer(new Blob(["y"]));
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(DOMException);
    expect(err.name).toBe("InvalidStateError");
    expect(err.message).toContain("The object is already busy reading Blobs.");
  });

  test("abort() while loading fires abort then loadend, and no load", async () => {
    const rt = createRuntime();
    const r = reader(rt);
    const seen: string[] = [];
    for (const t of ["loadstart", "load", "abort", "loadend"]) r.addEventListener(t, () => seen.push(t));
    r.readAsText(new Blob(["x"]));
    r.abort();
    expect([r.readyState, r.result]).toEqual([2, null]);
    await settleReader();
    expect(seen).toEqual(["abort", "loadend"]);
    // On a DONE reader, abort() only clears the result.
    r.readAsText(new Blob(["y"]));
    await settleReader();
    r.abort();
    expect([r.readyState, r.result]).toEqual([2, null]);
  });

  test("a read that fails is error then loadend, with the read's DOMException", async () => {
    const rt = createRuntime();
    class Unreadable extends Blob {
      override arrayBuffer(): Promise<ArrayBuffer> {
        return Promise.reject(new DOMException("The requested file could not be read.", "NotReadableError"));
      }
    }
    const r = reader(rt);
    const seen: string[] = [];
    for (const t of ["load", "error", "loadend"]) r.addEventListener(t, () => seen.push(t));
    r.readAsText(new Unreadable(["x"]));
    await settleReader();
    expect(seen).toEqual(["error", "loadend"]);
    expect([r.readyState, r.result, r.error?.name]).toEqual([2, null, "NotReadableError"]);
  });

  test("structuredClone refuses it, as Chromium does (the host marks the prototype)", () => {
    const rt = createRuntime();
    expect(rt.host.desktop.fs.refused.some((r: { name: string }) => r.name === "FileReader")).toBe(true);
  });
});

describe("the pickers (WICG File System Access)", () => {
  test("need a user gesture: SecurityError with Chromium's message", async () => {
    const rt = createRuntime();
    const err = await rt.global.showOpenFilePicker().catch((e: Error) => e);
    expect(err).toBeInstanceOf(DOMException);
    expect(err.name).toBe("SecurityError");
    expect(err.message).toBe(
      "Failed to execute 'showOpenFilePicker' on 'Window': Must be handling a user gesture to show a file picker.",
    );
    expect(calls(rt, "dialogOpen")).toEqual([]);
  });

  test("el.click() is not a gesture; a host keydown is, but not Escape or a lone modifier", () => {
    const rt = createRuntime();
    rt.noteActivation("keydown", "Escape");
    rt.noteActivation("keydown", "Shift");
    expect(rt.activation.isActive()).toBe(false);
    rt.noteActivation("keydown", "a");
    expect(rt.activation.isActive()).toBe(true);
  });

  test("a gesture sends the request, and is consumed by it", async () => {
    const rt = createRuntime();
    gesture(rt);
    const p = rt.global.showOpenFilePicker({
      multiple: true,
      id: "imports",
      startIn: "documents",
      types: [{ description: "Images", accept: { "image/*": [".png", ".gif"] } }],
    });
    const [c] = calls(rt, "dialogOpen");
    expect(c.args[0]).toEqual({
      id: "imports",
      startIn: "documents",
      types: [{ description: "Images", accept: { "image/*": [".png", ".gif"] } }],
      excludeAcceptAllOption: false,
      kind: "open",
      result: "handles",
      multiple: true,
    });
    expect(rt.activation.isActive()).toBe(false);
    const again = await rt.global.showSaveFilePicker().catch((e: Error) => e);
    expect(again.name).toBe("SecurityError");
    rt.host.desktop.settle(c.token, true, {
      handles: [
        { h: 1, name: "a.png", kind: "file" },
        { h: 2, name: "b.gif", kind: "file" },
      ],
    });
    const hs = await p;
    expect(hs.map((h: any) => h.name)).toEqual(["a.png", "b.gif"]);
    expect(hs[0]).toBeInstanceOf(rt.global.FileSystemFileHandle);
    expect(JSON.stringify(hs[0])).toBe("{}");
  });

  test("a dismissal is AbortError; a second dialog on the window is NotAllowedError", async () => {
    const rt = createRuntime();
    gesture(rt);
    const p = rt.global.showDirectoryPicker();
    const [c] = calls(rt, "dialogOpen");
    expect((c.args[0] as any).kind).toBe("folder");
    rt.host.desktop.settle(c.token, true, { cancelled: true });
    const err = await p.catch((e: Error) => e);
    expect([err.name, err.message]).toEqual(["AbortError", "The user aborted a request."]);
    expect(err).toBeInstanceOf(DOMException);

    gesture(rt);
    rt.host.desktop.fs.answers.set("dialogOpen", () => ({
      sync: { error: "File picker already active.", name: "NotAllowedError", code: "EBUSY" },
    }));
    const busy = await rt.global.showSaveFilePicker().catch((e: Error) => e);
    expect([busy.name, busy.message]).toEqual([
      "NotAllowedError",
      "Failed to execute 'showSaveFilePicker' on 'Window': File picker already active.",
    ]);
  });

  test("option checks reject with TypeErrors before anything is asked", async () => {
    const rt = createRuntime();
    const bad = async (o: unknown) => {
      gesture(rt);
      const e = await rt.global.showOpenFilePicker(o).catch((x: Error) => x);
      return e instanceof TypeError ? e.message : `not a TypeError: ${e}`;
    };
    expect(await bad({ id: "x".repeat(33) })).toContain("ID too long");
    expect(await bad({ id: "a b" })).toContain("ID contains invalid characters");
    expect(await bad({ startIn: "home" })).toContain("not a valid enum value of type WellKnownDirectory");
    expect(await bad({ types: [{ accept: { "not a mime": [".x"] } }] })).toContain("Invalid type: not a mime");
    expect(await bad({ types: [{ accept: { "text/plain": ["txt"] } }] })).toContain("must start with '.'");
    expect(await bad({ types: [{ accept: { "text/plain": [".t?t"] } }] })).toContain("contains invalid characters");
    expect(await bad({ types: [{ accept: { "text/plain": [".txt."] } }] })).toContain("must not end with '.'");
    expect(await bad({ types: [{ accept: { "text/plain": [".abcdefghijklmnopq"] } }] })).toContain("longer than 16");
    expect(await bad({ excludeAcceptAllOption: true })).toContain("Need at least one accepted type");
    expect(calls(rt, "dialogOpen")).toEqual([]);
  });

  test("startIn may be a handle; the request carries its id, never a path", async () => {
    const rt = createRuntime();
    const dir = handle(rt, 41, "proj", "directory");
    gesture(rt);
    rt.global.showSaveFilePicker({ startIn: dir, suggestedName: "out.csv" });
    const [c] = calls(rt, "dialogOpen");
    expect(c.args[0]).toMatchObject({ kind: "save", startIn: { h: 41 }, suggestedName: "out.csv" });
  });
});

describe("FileSystemHandle and its two kinds", () => {
  test("cannot be constructed by script, and holds no path anywhere", () => {
    const rt = createRuntime();
    expect(() => new rt.global.FileSystemHandle()).toThrow(TypeError);
    expect(() => new rt.global.FileSystemFileHandle()).toThrow(TypeError);
    const f = handle(rt, 3, "notes.txt", "file");
    expect([f.kind, f.name]).toEqual(["file", "notes.txt"]);
    expect(Object.keys(f)).toEqual([]);
    expect(Object.prototype.toString.call(f)).toBe("[object FileSystemFileHandle]");
    expect(typeof f.createSyncAccessHandle).toBe("undefined");
    expect(typeof f.move).toBe("undefined");
  });

  test("permissions are always granted (D-F2); an invalid mode is a TypeError", async () => {
    const rt = createRuntime();
    const f = handle(rt, 3, "notes.txt", "file");
    expect(await f.queryPermission()).toBe("granted");
    expect(await f.requestPermission({ mode: "readwrite" })).toBe("granted");
    expect(await f.queryPermission({ mode: "write" }).catch((e: Error) => e)).toBeInstanceOf(TypeError);
  });

  test("a child is looked up by the parent's id; failures are the spec's DOMExceptions", async () => {
    const rt = createRuntime();
    const dir = handle(rt, 5, "proj", "directory");
    rt.host.desktop.fs.answers.set("child", (_d: number, name: string, isDir: boolean) =>
      name === "missing.txt"
        ? { ok: false, payload: { name: "NotFoundError", message: "A requested file or directory could not be found at the time an operation was processed." } }
        : name === ".."
          ? { sync: { error: "Name is not allowed.", name: "TypeError" } }
          : { payload: { h: 9, name, kind: isDir ? "directory" : "file" } },
    );
    const f = await dir.getFileHandle("a.txt", { create: true });
    expect(f).toBeInstanceOf(rt.global.FileSystemFileHandle);
    expect(calls(rt, "child")[0].args).toEqual([5, "a.txt", false, true]);
    const d = await dir.getDirectoryHandle("sub");
    expect(d).toBeInstanceOf(rt.global.FileSystemDirectoryHandle);
    const nf = await dir.getFileHandle("missing.txt").catch((e: Error) => e);
    expect(nf).toBeInstanceOf(DOMException);
    expect(nf.name).toBe("NotFoundError");
    const bad = await dir.getFileHandle("..").catch((e: Error) => e);
    expect(bad).toBeInstanceOf(TypeError);
    expect(bad.message).toBe("Name is not allowed.");
  });

  test("entries() drains the host cursor batch by batch, and a break closes it", async () => {
    const rt = createRuntime();
    const dir = handle(rt, 5, "proj", "directory");
    rt.host.desktop.fs.answers.set("dirOpen", () => ({ payload: { cursor: 77 } }));
    let batch = 0;
    rt.host.desktop.fs.answers.set("dirNext", () => {
      batch++;
      return batch === 1
        ? { payload: { entries: [{ h: 20, name: "a.txt", kind: "file" }, { h: 21, name: "sub", kind: "directory" }], done: false } }
        : { payload: { entries: [{ h: 22, name: "z.txt", kind: "file" }], done: true } };
    });
    const names: string[] = [];
    for await (const [name, h] of dir) names.push(`${name}:${h.kind}`);
    expect(names).toEqual(["a.txt:file", "sub:directory", "z.txt:file"]);
    expect(calls(rt, "dirClose").map((c) => c.args[0])).toEqual([77]);
    batch = 0;
    for await (const k of dir.keys()) {
      expect(k).toBe("a.txt");
      break;
    }
    expect(calls(rt, "dirClose").length).toBe(2);
  });

  test("isSameEntry and resolve go by id; different kinds are never the same entry", async () => {
    const rt = createRuntime();
    const dir = handle(rt, 5, "proj", "directory");
    const f = handle(rt, 6, "a.txt", "file");
    expect(await dir.isSameEntry(f)).toBe(false);
    expect(calls(rt, "same")).toEqual([]);
    rt.host.desktop.fs.answers.set("same", () => ({ payload: { same: true } }));
    expect(await f.isSameEntry(handle(rt, 7, "a.txt", "file"))).toBe(true);
    rt.host.desktop.fs.answers.set("resolve", () => ({ payload: { path: ["src", "a.txt"] } }));
    expect(await dir.resolve(f)).toEqual(["src", "a.txt"]);
    expect(await dir.resolve({}).catch((e: Error) => e)).toBeInstanceOf(TypeError);
  });

  test("every handle class is refused by structuredClone (D-F23), most specific first, and advises once", () => {
    const rt = createRuntime();
    const names = rt.host.desktop.fs.refused.map((r: { name: string }) => r.name);
    expect(names).toEqual([
      "FileReader",
      "FileSystemFileHandle",
      "FileSystemDirectoryHandle",
      "FileSystemHandle",
      "FileSystemWritableFileStream",
    ]);
    handle(rt, 1, "a", "file");
    handle(rt, 2, "b", "file");
    expect(rt.advisories().filter((l: string) => l.includes("D-F23")).length).toBe(1);
  });
});

describe("FileSystemWritableFileStream", () => {
  async function stream(rt: Rt) {
    const f = handle(rt, 3, "out.txt", "file");
    for (const op of ["writableWrite", "writableSeek", "writableTruncate", "writableClose", "writableAbort"]) {
      rt.host.desktop.fs.answers.set(op, () => ({ payload: {} }));
    }
    rt.host.desktop.fs.answers.set("writableOpen", () => ({ payload: { w: 12 } }));
    return f.createWritable({ keepExistingData: true });
  }

  test("is a WritableStream; write/seek/truncate/close cross with the stream's id", async () => {
    const rt = createRuntime();
    const w = await stream(rt);
    expect(w).toBeInstanceOf(WritableStream);
    expect(w).toBeInstanceOf(rt.global.FileSystemWritableFileStream);
    expect(calls(rt, "writableOpen")[0].args).toEqual([3, true]);
    await w.write("hé");
    await w.write(new Uint8Array([1, 2, 3]).buffer);
    await w.write(new Uint16Array([0x0201]));
    await w.write({ type: "write", position: 5, data: "x" });
    await w.seek(2);
    await w.truncate(4);
    await w.close();
    const writes = calls(rt, "writableWrite").map((c) => [c.args[0], c.args[1], c.args[2]]);
    expect(writes[0]).toEqual([12, -1, "hé"]);
    expect(writes[1][2]).toEqual(new Uint8Array([1, 2, 3]));
    expect(writes[2][2]).toEqual(new Uint8Array([1, 2]));
    expect(writes[3]).toEqual([12, 5, "x"]);
    expect(calls(rt, "writableSeek")[0].args).toEqual([12, 2]);
    expect(calls(rt, "writableTruncate")[0].args).toEqual([12, 4]);
    expect(calls(rt, "writableClose")[0].args).toEqual([12]);
  });

  test("a Blob crosses as the Blob (the host copies it on the pool)", async () => {
    const rt = createRuntime();
    const w = await stream(rt);
    const b = new Blob(["abc"]);
    await w.write(b);
    expect(calls(rt, "writableWrite")[0].args[2]).toBe(b);
  });

  test("bad chunks error the stream with the spec's exceptions", async () => {
    const rt = createRuntime();
    const missing = await (await stream(rt)).write({ type: "write" }).catch((e: Error) => e);
    expect(missing.name).toBe("SyntaxError");
    const kind = await (await stream(rt)).write({ type: "rename" }).catch((e: Error) => e);
    expect(kind).toBeInstanceOf(TypeError);
    const num = await (await stream(rt)).write(42).catch((e: Error) => e);
    expect(num).toBeInstanceOf(TypeError);
  });

  test("abort() deletes the swap through the host", async () => {
    const rt = createRuntime();
    const w = await stream(rt);
    await w.abort();
    expect(calls(rt, "writableAbort")[0].args).toEqual([12]);
  });
});
