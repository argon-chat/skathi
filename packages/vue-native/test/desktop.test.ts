// File System Access, the pickers, `<input type=file>` and `<a download>`: the JS half. Pinned here is what only the
// runtime can get wrong: option checks and exception names, transient activation, that a handle holds no path, and
// the input element's events.

import { describe, expect, test } from "bun:test";
import { createRuntime, FakeFileList } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
type Call = { op: string; args: unknown[]; token: number | null };

const calls = (rt: Rt, op: string): Call[] => rt.host.desktop.fs.calls.filter((c: Call) => c.op === op);
const tick = () => new Promise((r) => setTimeout(r, 0));
const gesture = (rt: Rt) => rt.noteActivation("mousedown", undefined);

const handle = (rt: Rt, h: number, name: string, kind: "file" | "directory") =>
  rt.desktopApi.handleFrom({ h, name, kind }) as Record<string, any>;

test("desktop adapter preserves the engine's native FileReader and ProgressEvent", () => {
  class NativeReader {}
  class NativeProgress {}
  const rt = createRuntime({ globals: { FileReader: NativeReader, ProgressEvent: NativeProgress } });
  expect(rt.global.FileReader).toBe(NativeReader);
  expect(rt.global.ProgressEvent).toBe(NativeProgress);
  expect(rt.host.desktop.fs.refused.some((entry: { name: string }) => entry.name === "FileReader")).toBe(false);
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

  test("what the user chose is granted in both modes; an invalid mode is a TypeError", async () => {
    const rt = createRuntime();
    const f = handle(rt, 3, "notes.txt", "file");
    expect(await f.queryPermission()).toBe("granted");
    expect(await f.requestPermission({ mode: "readwrite" })).toBe("granted");
    expect(await f.queryPermission({ mode: "write" }).catch((e: Error) => e)).toBeInstanceOf(TypeError);
    expect(rt.host.desktop.fs.permissionCalls.every((c: { request: boolean }) => !c.request)).toBe(true);
  });

  test("a restored handle is 'prompt'; requestPermission needs a gesture, then grants once-advised", async () => {
    const rt = createRuntime();
    rt.host.desktop.fs.prompt.add(8);
    const f = handle(rt, 8, "restored.txt", "file");
    expect(await f.queryPermission({ mode: "readwrite" })).toBe("prompt");
    const denied = await f.requestPermission().catch((e: Error) => e);
    expect(denied).toBeInstanceOf(DOMException);
    expect([denied.name, denied.message]).toEqual([
      "SecurityError",
      "User activation is required to request permissions.",
    ]);
    expect(rt.host.desktop.fs.prompt.has(8)).toBe(true);
    gesture(rt);
    expect(await f.requestPermission()).toBe("granted");
    expect(rt.activation.isActive()).toBe(true);
    expect(await f.queryPermission()).toBe("granted");
    rt.host.desktop.fs.prompt.add(9);
    await handle(rt, 9, "other.txt", "file").requestPermission();
    expect(rt.advisories().filter((l: string) => l.includes("restored from storage")).length).toBe(1);
  });

  test("a write through a 'prompt' handle asks on a gesture, and is the host's NotAllowedError without one", async () => {
    const rt = createRuntime();
    rt.host.desktop.fs.prompt.add(8);
    const dir = handle(rt, 8, "restored", "directory");
    rt.host.desktop.fs.answers.set("remove", (d: number) =>
      rt.host.desktop.fs.prompt.has(d)
        ? {
            sync: {
              error: "The request is not allowed by the user agent or the platform in the current context.",
              name: "NotAllowedError",
            },
          }
        : { payload: {} },
    );
    const refused = await dir.removeEntry("x").catch((e: Error) => e);
    expect(refused).toBeInstanceOf(DOMException);
    expect(refused.name).toBe("NotAllowedError");
    gesture(rt);
    rt.host.desktop.fs.answers.set("child", () => ({ payload: { h: 10, name: "a", kind: "file" } }));
    await dir.getFileHandle("a");
    expect(rt.host.desktop.fs.prompt.has(8)).toBe(true);
    await dir.removeEntry("x");
    expect(rt.host.desktop.fs.prompt.has(8)).toBe(false);
  });

  test("a child is looked up by the parent's id; failures are the spec's DOMExceptions", async () => {
    const rt = createRuntime();
    const dir = handle(rt, 5, "proj", "directory");
    rt.host.desktop.fs.answers.set("child", (_d: number, name: string, isDir: boolean) =>
      name === "missing.txt"
        ? {
            ok: false,
            payload: {
              name: "NotFoundError",
              message: "A requested file or directory could not be found at the time an operation was processed.",
            },
          }
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
        ? {
            payload: {
              entries: [
                { h: 20, name: "a.txt", kind: "file" },
                { h: 21, name: "sub", kind: "directory" },
              ],
              done: false,
            },
          }
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

  test("handles are [Serializable] through their base, by id; the writable stream is refused by this adapter", () => {
    const rt = createRuntime();
    const { proto, idOf } = rt.host.desktop.fs.serializer;
    expect(proto).toBe(rt.global.FileSystemHandle.prototype);
    expect(idOf(handle(rt, 31, "a", "file"))).toBe(31);
    expect(idOf(handle(rt, 32, "d", "directory"))).toBe(32);
    expect(idOf(Object.create(rt.global.FileSystemFileHandle.prototype))).toBeUndefined();
    const names = rt.host.desktop.fs.refused.map((r: { name: string }) => r.name);
    expect(names).toEqual(["FileSystemWritableFileStream"]);
    const clone = rt.host.desktop.fs.factory(33, "a", "file");
    expect(clone).toBeInstanceOf(rt.global.FileSystemFileHandle);
    expect(rt.advisories()).toEqual([]);
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

function fileWorld(attrs: Record<string, string> = {}) {
  const rt = createRuntime();
  const root = rt.nodeOps.createElement("root");
  const input = rt.nodeOps.createElement("input");
  rt.nodeOps.insert(input, root, null);
  rt.patchProp(input, "type", null, "file");
  for (const [k, v] of Object.entries(attrs)) rt.patchProp(input, k, null, v);
  const seen: string[] = [];
  for (const t of ["input", "change", "cancel"]) {
    rt.patchProp(root, `on${t[0].toUpperCase()}${t.slice(1)}`, null, (e: any) =>
      seen.push(`${t}:${e.bubbles}:${e.target === input}`),
    );
  }
  const host = (node: { id: number }, name: "click" | "keydown" | "keyup" | "mousedown", key?: string) =>
    rt.host.dispatcher(node.__vnId, rt.EV_CODES[name], 5, 6, 0, 0, key);
  const userClick = (node: { id: number }) => {
    host(node, "mousedown");
    host(node, "click");
  };
  const label = () =>
    rt.host.props.filter((p: { id: number; key: string }) => p.id === input.__vnId && p.key === ":file-label").at(-1)
      ?.value;
  return { rt, root, input: input as Record<string, any>, seen, host, userClick, label };
}

describe("<input type=file> (HTML)", () => {
  test("arms click, Enter and Space, and starts empty: a FileList, no value, 'No file chosen'", () => {
    const { rt, input, label } = fileWorld();
    for (const code of [rt.EV_CODES.click, rt.EV_CODES.keydown, rt.EV_CODES.keyup]) {
      expect(rt.flagsFor(code)).toEqual([{ id: input.__vnId, code, on: true }]);
    }
    expect(label()).toBe("No file chosen");
    expect(input.files).toBeInstanceOf(FakeFileList);
    expect([input.files.length, input.value]).toEqual([0, ""]);
    expect(rt.advisories().filter((l: string) => l.includes("approximation of Chromium")).length).toBe(1);
  });

  test("a click without a user activation opens nothing, and says so in Chromium's words", () => {
    const { rt, input } = fileWorld();
    input.click();
    expect(calls(rt, "dialogOpen")).toEqual([]);
    expect(rt.host.logs).toContain("File chooser dialog can only be shown with a user activation.");
  });

  test("a host click is a user's press even when no mousedown came first (nobody listened for one)", () => {
    // The host forwards only what is listened for; the input arms click, not mousedown.
    const { rt, input, host } = fileWorld();
    host(input, "click");
    expect(calls(rt, "dialogOpen").length).toBe(1);
  });

  test("a user's click asks for files; the pick sets files, then fires input and change, both bubbling", async () => {
    const { rt, input, seen, userClick, label } = fileWorld({ accept: ".CSV, text/*, nope", multiple: "" });
    userClick(input);
    const [c] = calls(rt, "dialogOpen");
    expect(c.args[0]).toEqual({ kind: "open", multiple: true, accept: [".csv", "text/*"], result: "files" });
    expect(rt.advisories().some((l: string) => l.includes('"nope"') && l.includes("ignored"))).toBe(true);
    const list = new FakeFileList([new File(["a,b"], "data.csv"), new File(["x"], "two.csv")]);
    rt.host.desktop.settle(c.token, true, { files: 2 }, list);
    await tick();
    expect(seen).toEqual(["input:true:true", "change:true:true"]);
    expect(input.files).toBe(list);
    expect(input.value).toBe("C:\\fakepath\\data.csv");
    expect(label()).toBe("2 files");
    userClick(input);
    expect(calls(rt, "dialogOpen")[1].args[1]).toBe(list);
  });

  test("a dismissal (or the same selection) fires cancel; a busy window fires nothing", async () => {
    const { rt, input, seen, userClick } = fileWorld();
    userClick(input);
    rt.host.desktop.settle(calls(rt, "dialogOpen")[0].token, true, { cancelled: true });
    await tick();
    expect(seen).toEqual(["cancel:true:true"]);
    rt.host.desktop.fs.answers.set("dialogOpen", () => ({
      sync: { error: "File picker already active.", name: "NotAllowedError", code: "EBUSY" },
    }));
    userClick(input);
    await tick();
    expect(seen).toEqual(["cancel:true:true"]);
  });

  test("value takes only ''; files takes a FileList; neither fires anything", () => {
    const { input, seen, label } = fileWorld();
    const list = new FakeFileList([new File(["x"], "a.txt")]);
    input.files = list;
    expect([input.value, label()]).toEqual(["C:\\fakepath\\a.txt", "a.txt"]);
    input.files = null;
    expect(input.files).toBe(list);
    expect(() => (input.files = [new File(["x"], "b.txt")])).toThrow(TypeError);
    let err: any;
    try {
      input.value = "C:\\x.txt";
    } catch (e) {
      err = e;
    }
    expect([err?.name, err?.message]).toEqual([
      "InvalidStateError",
      "Failed to set the 'value' property on 'HTMLInputElement': This input element accepts a filename, which may only be programmatically set to the empty string.",
    ]);
    input.value = "";
    expect([input.files.length, input.value, label()]).toEqual([0, "", "No file chosen"]);
    expect(seen).toEqual([]);
  });

  test("webkitdirectory picks a folder; a disabled input opens nothing; IDL attributes reflect", () => {
    const { rt, input, userClick } = fileWorld({ webkitdirectory: "" });
    expect([input.webkitdirectory, input.multiple, input.accept]).toEqual([true, false, ""]);
    userClick(input);
    expect((calls(rt, "dialogOpen")[0].args[0] as any).kind).toBe("folder");
    input.multiple = true;
    input.accept = "image/*";
    expect(
      rt.host.props
        .filter((p: any) => p.id === input.__vnId && ["multiple", "accept"].includes(p.key))
        .map((p: any) => [p.key, p.value]),
    ).toEqual([
      ["multiple", ""],
      ["accept", "image/*"],
    ]);
    rt.patchProp(input, "disabled", null, "");
    userClick(input);
    input.click();
    expect(calls(rt, "dialogOpen").length).toBe(1);
  });

  test("Enter (keydown) and Space (keyup) click it; a cancelled keydown does not", () => {
    const { rt, input, host } = fileWorld();
    host(input, "keydown", "Enter");
    expect(calls(rt, "dialogOpen").length).toBe(1);
    host(input, "keydown", " ");
    expect(calls(rt, "dialogOpen").length).toBe(1);
    host(input, "keyup", " ");
    expect(calls(rt, "dialogOpen").length).toBe(2);
    rt.patchProp(input, "onKeydown", null, (e: any) => e.preventDefault());
    host(input, "keydown", "Enter");
    expect(calls(rt, "dialogOpen").length).toBe(2);
  });

  test("showPicker(): immutable is InvalidStateError, no gesture is NotAllowedError, a text input advises", () => {
    const { rt, input, host } = fileWorld();
    const thrown = (el: any) => {
      try {
        el.showPicker();
        return "no throw";
      } catch (e: any) {
        return `${e.name}: ${e.message}`;
      }
    };
    expect(thrown(input)).toBe(
      "NotAllowedError: Failed to execute 'showPicker' on 'HTMLInputElement': HTMLInputElement::showPicker() requires a user gesture.",
    );
    host(input, "mousedown");
    expect(thrown(input)).toBe("no throw");
    expect(calls(rt, "dialogOpen").length).toBe(1);
    rt.patchProp(input, "disabled", null, "");
    expect(thrown(input)).toContain(
      "InvalidStateError: Failed to execute 'showPicker' on 'HTMLInputElement': HTMLInputElement::showPicker() cannot be used on immutable controls.",
    );
    const text = rt.nodeOps.createElement("input");
    expect(thrown(text)).toBe("no throw");
    expect(rt.advisories().some((l: string) => l.includes("no-op"))).toBe(true);
  });

  test("@change on an element that neither is nor wraps a file input still advises; on a wrapper it does not", async () => {
    const { rt } = fileWorld();
    await tick();
    expect(rt.advisories().filter((l: string) => l.includes("@change") || l.includes("@cancel"))).toEqual([]);
    const div = rt.nodeOps.createElement("div");
    rt.patchProp(div, "onChange", null, () => {});
    await tick();
    expect(rt.advisories().filter((l: string) => l.includes("@change")).length).toBe(1);
  });
});

describe("<label> activation (HTML)", () => {
  function labelWorld() {
    const w = fileWorld({ id: "pick" });
    const label = w.rt.nodeOps.createElement("label");
    const span = w.rt.nodeOps.createElement("span");
    w.rt.nodeOps.insert(label, w.root, null);
    w.rt.nodeOps.insert(span, label, null);
    return { ...w, label, span };
  }

  test("is armed for clicks at creation, and clicks the control its `for` names", () => {
    const { rt, label, span, userClick } = labelWorld();
    expect(rt.flagsFor(rt.EV_CODES.click).some((f: any) => f.id === label.__vnId && f.on)).toBe(true);
    rt.patchProp(label, "for", null, "pick");
    userClick(span);
    expect(calls(rt, "dialogOpen").length).toBe(1);
  });

  test("without `for`, clicks its first labelable descendant, and a click inside the control is not doubled", () => {
    const rt = createRuntime();
    const label = rt.nodeOps.createElement("label");
    const text = rt.nodeOps.createElement("span");
    const input = rt.nodeOps.createElement("input");
    rt.nodeOps.insert(text, label, null);
    rt.nodeOps.insert(input, label, null);
    rt.patchProp(input, "type", null, "file");
    let clicks = 0;
    rt.patchProp(input, "onClick", null, () => clicks++);
    const user = (n: { id: number }) => {
      rt.host.dispatcher(n.__vnId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
      rt.host.dispatcher(n.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    };
    user(text);
    expect([clicks, calls(rt, "dialogOpen").length]).toEqual([1, 1]);
    user(input);
    expect([clicks, calls(rt, "dialogOpen").length]).toEqual([2, 2]);
  });

  test("a label for a text field focuses it and clicks it", () => {
    const rt = createRuntime();
    rt.host.focus.answer = (_id: number, op: number) => (op === 0 ? "moved:0" : "");
    const label = rt.nodeOps.createElement("label");
    const input = rt.nodeOps.createElement("input");
    rt.nodeOps.insert(input, label, null);
    let clicked = 0;
    rt.patchProp(input, "onClick", null, () => clicked++);
    rt.host.dispatcher(label.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    rt.host.dispatcher(label.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    expect(clicked).toBe(2);
    expect(rt.host.focus.calls).toEqual([
      { id: input.__vnId, op: 0 },
      { id: input.__vnId, op: 0 },
    ]);
    expect(rt.global.document.activeElement).toBe(input);
  });
});

describe("detached elements (redjs's document.createElement)", () => {
  test("VueUse's useFileDialog: an input never in the tree picks, and onchange sees event.target.files", async () => {
    const rt = createRuntime();
    const input = rt.global.document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = "image/*";
    expect(input.files.length).toBe(0);
    let got: any = null;
    let cancelled = 0;
    input.onchange = (e: any) => (got = e.target.files);
    input.oncancel = () => cancelled++;
    rt.noteActivation("mousedown", undefined);
    input.click();
    const [c] = calls(rt, "dialogOpen");
    expect(c.args[0]).toEqual({ kind: "open", multiple: true, accept: ["image/*"], result: "files" });
    const list = new FakeFileList([new File(["p"], "p.png")]);
    rt.host.desktop.settle(c.token, true, { files: 1 }, list);
    await tick();
    expect(got).toBe(list);
    expect(input.value).toBe("C:\\fakepath\\p.png");
    input.click();
    rt.host.desktop.settle(calls(rt, "dialogOpen")[1].token, true, { cancelled: true });
    await tick();
    expect(cancelled).toBe(1);
  });

  test("FileSaver.js: a detached <a download> clicked or dispatched saves; setAttribute works too", () => {
    const rt = createRuntime();
    const a = rt.global.document.createElement("a");
    a.href = "blob:vn/42";
    a.download = "notes.txt";
    a.dispatchEvent(new Event("click"));
    const b = rt.global.document.createElement("a");
    b.setAttribute("href", "data:text/plain,hi");
    b.setAttribute("download", "");
    expect(b.getAttribute("download")).toBe("");
    b.click();
    expect(calls(rt, "download").map((c) => c.args)).toEqual([
      ["blob:vn/42", "notes.txt"],
      ["data:text/plain,hi", ""],
    ]);
  });
});
