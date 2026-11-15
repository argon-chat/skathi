// `<input type=file>`, `<label>` and `<a download>`: what the runtime decides before sk-desktop's `__SK_FS__` is asked
// (transient activation, the element's attributes and events). File System Access itself is sk-desktop's, tested there.

import { describe, expect, test } from "bun:test";
import { createRuntime, FakeFileList, type MirrorElement } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
type Call = { op: string; args: unknown[]; token: number | null };

const calls = (rt: Rt, op: string): Call[] => rt.host.desktop.fs.calls.filter((c: Call) => c.op === op);
const tick = () => new Promise((r) => setTimeout(r, 0));

interface DetachedElement {
  type: string;
  onchange: ((e: { target: { files: unknown } }) => void) | null;
  oncancel: (() => void) | null;
  href: string;
  download: string;
  dispatchEvent(e: Event): boolean;
  setAttribute(name: string, value: string): void;
}

interface DesktopGlobals {
  document: { createElement(tag: string): FileInput & DetachedElement; activeElement: unknown };
}

const win = (rt: Rt) => rt.global as unknown as DesktopGlobals;

type FileInput = MirrorElement & FileInputProps;

interface FileInputProps {
  get files(): FakeFileList;
  set files(value: unknown);
  value: string;
  accept: string;
  multiple: boolean;
  webkitdirectory: boolean;
  click(): void;
  showPicker(): void;
}

function fileWorld(attrs: Record<string, string> = {}) {
  const rt = createRuntime();
  const root = rt.nodeOps.createElement("root");
  const input = rt.nodeOps.createElement("input");
  rt.nodeOps.insert(input, root, null);
  rt.patchProp(input, "type", null, "file");
  for (const [k, v] of Object.entries(attrs)) rt.patchProp(input, k, null, v);
  const seen: string[] = [];
  for (const t of ["input", "change", "cancel"]) {
    rt.patchProp(root, `on${t[0]!.toUpperCase()}${t.slice(1)}`, null, (e: { bubbles: boolean; target: unknown }) =>
      seen.push(`${t}:${e.bubbles}:${e.target === input}`),
    );
  }
  const host = (node: { __skId: number }, name: "click" | "keydown" | "keyup" | "mousedown", key?: string) =>
    rt.host.dispatcher(node.__skId, rt.EV_CODES[name], 5, 6, 0, 0, key);
  const userClick = (node: { __skId: number }) => {
    host(node, "mousedown");
    host(node, "click");
  };
  const label = () =>
    rt.host.props.filter((p: { id: number; key: string }) => p.id === input.__skId && p.key === ":file-label").at(-1)
      ?.value;
  return { rt, root, input: input as unknown as FileInput, seen, host, userClick, label };
}

describe("<input type=file> (HTML)", () => {
  test("arms click, Enter and Space, and starts empty: a FileList, no value, 'No file chosen'", () => {
    const { rt, input, label } = fileWorld();
    for (const code of [rt.EV_CODES.click, rt.EV_CODES.keydown, rt.EV_CODES.keyup]) {
      expect(rt.flagsFor(code)).toEqual([{ id: input.__skId, code, on: true }]);
    }
    expect(label()).toBe("No file chosen");
    expect(input.files).toBeInstanceOf(FakeFileList);
    expect([input.files.length, input.value]).toEqual([0, ""]);
  });

  test("a click without a user activation opens nothing", () => {
    const { rt, input } = fileWorld();
    input.click();
    expect(calls(rt, "pickForInput")).toEqual([]);
  });

  test("a host click is a user's press even when no mousedown came first (nobody listened for one)", () => {
    // The host forwards only what is listened for; the input arms click, not mousedown.
    const { rt, input, host } = fileWorld();
    host(input, "click");
    expect(calls(rt, "pickForInput").length).toBe(1);
  });

  test("a user's click asks for files; the pick sets files, then fires input and change, both bubbling", async () => {
    const { rt, input, seen, userClick, label } = fileWorld({ accept: ".CSV, text/*, nope", multiple: "" });
    userClick(input);
    const c = calls(rt, "pickForInput")[0]!;
    expect(c.args[0]).toEqual({ accept: ".CSV, text/*, nope", multiple: true, directory: false });
    const list = new FakeFileList([new File(["a,b"], "data.csv"), new File(["x"], "two.csv")]);
    rt.host.desktop.settle(c.token!, true, { files: 2 }, list);
    await tick();
    expect(seen).toEqual(["input:true:true", "change:true:true"]);
    expect(input.files).toBe(list);
    expect(input.value).toBe("C:\\fakepath\\data.csv");
    expect(label()).toBe("2 files");
    userClick(input);
    expect(calls(rt, "pickForInput")[1]!.args[1]).toBe(list);
  });

  test("a dismissal (or the same selection) fires cancel; a busy window fires nothing", async () => {
    const { rt, input, seen, userClick } = fileWorld();
    userClick(input);
    rt.host.desktop.settle(calls(rt, "pickForInput")[0]!.token!, true, { cancelled: true });
    await tick();
    expect(seen).toEqual(["cancel:true:true"]);
    rt.host.desktop.fs.busy = true;
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
    let err: Error | undefined;
    try {
      input.value = "C:\\x.txt";
    } catch (e) {
      err = e as Error;
    }
    expect(err?.name).toBe("InvalidStateError");
    input.value = "";
    expect([input.files.length, input.value, label()]).toEqual([0, "", "No file chosen"]);
    expect(seen).toEqual([]);
  });

  test("webkitdirectory picks a folder; a disabled input opens nothing; IDL attributes reflect", () => {
    const { rt, input, userClick } = fileWorld({ webkitdirectory: "" });
    expect([input.webkitdirectory, input.multiple, input.accept]).toEqual([true, false, ""]);
    userClick(input);
    expect((calls(rt, "pickForInput")[0]!.args[0] as { directory: boolean }).directory).toBe(true);
    input.multiple = true;
    input.accept = "image/*";
    expect(
      rt.host.props
        .filter((p: { id: number; key: string }) => p.id === input.__skId && ["multiple", "accept"].includes(p.key))
        .map((p: { key: string; value: unknown }) => [p.key, p.value]),
    ).toEqual([
      ["multiple", ""],
      ["accept", "image/*"],
    ]);
    rt.patchProp(input, "disabled", null, "");
    userClick(input);
    input.click();
    expect(calls(rt, "pickForInput").length).toBe(1);
  });

  test("Enter (keydown) and Space (keyup) click it; a cancelled keydown does not", () => {
    const { rt, input, host } = fileWorld();
    host(input, "keydown", "Enter");
    expect(calls(rt, "pickForInput").length).toBe(1);
    host(input, "keydown", " ");
    expect(calls(rt, "pickForInput").length).toBe(1);
    host(input, "keyup", " ");
    expect(calls(rt, "pickForInput").length).toBe(2);
    rt.patchProp(input, "onKeydown", null, (e: Event) => e.preventDefault());
    host(input, "keydown", "Enter");
    expect(calls(rt, "pickForInput").length).toBe(2);
  });

  test("showPicker(): immutable is InvalidStateError, no gesture is NotAllowedError", () => {
    const { rt, input, host } = fileWorld();
    const thrown = (el: { showPicker(): void }) => {
      try {
        el.showPicker();
        return "no throw";
      } catch (e) {
        return (e as Error).name;
      }
    };
    expect(thrown(input)).toBe("NotAllowedError");
    host(input, "mousedown");
    expect(thrown(input)).toBe("no throw");
    expect(calls(rt, "pickForInput").length).toBe(1);
    rt.patchProp(input, "disabled", null, "");
    expect(thrown(input)).toBe("InvalidStateError");
    const text = rt.nodeOps.createElement("input");
    expect(thrown(text)).toBe("no throw");
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
    expect(rt.flagsFor(rt.EV_CODES.click).some((f) => f.id === label.__skId && f.on)).toBe(true);
    rt.patchProp(label, "for", null, "pick");
    userClick(span);
    expect(calls(rt, "pickForInput").length).toBe(1);
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
    const user = (n: { __skId: number }) => {
      rt.host.dispatcher(n.__skId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
      rt.host.dispatcher(n.__skId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    };
    user(text);
    expect([clicks, calls(rt, "pickForInput").length]).toEqual([1, 1]);
    user(input);
    expect([clicks, calls(rt, "pickForInput").length]).toEqual([2, 2]);
  });

  test("a label for a text field focuses it and clicks it", () => {
    const rt = createRuntime();
    rt.host.focus.answer = (_id: number, op: number) => (op === 0 ? "moved:0" : "");
    const label = rt.nodeOps.createElement("label");
    const input = rt.nodeOps.createElement("input");
    rt.nodeOps.insert(input, label, null);
    let clicked = 0;
    rt.patchProp(input, "onClick", null, () => clicked++);
    rt.host.dispatcher(label.__skId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    rt.host.dispatcher(label.__skId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    expect(clicked).toBe(2);
    expect(rt.host.focus.calls).toEqual([
      { id: input.__skId, op: 0 },
      { id: input.__skId, op: 0 },
    ]);
    expect(win(rt).document.activeElement).toBe(input);
  });
});

describe("detached elements (redjs's document.createElement)", () => {
  test("VueUse's useFileDialog: an input never in the tree picks, and onchange sees event.target.files", async () => {
    const rt = createRuntime();
    const input = win(rt).document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = "image/*";
    expect(input.files.length).toBe(0);
    let got: unknown = null;
    let cancelled = 0;
    input.onchange = (e: { target: { files: unknown } }) => (got = e.target.files);
    input.oncancel = () => cancelled++;
    rt.noteActivation("mousedown", undefined);
    input.click();
    const c = calls(rt, "pickForInput")[0]!;
    expect(c.args[0]).toEqual({ accept: "image/*", multiple: true, directory: false });
    const list = new FakeFileList([new File(["p"], "p.png")]);
    rt.host.desktop.settle(c.token!, true, { files: 1 }, list);
    await tick();
    expect(got).toBe(list);
    expect(input.value).toBe("C:\\fakepath\\p.png");
    input.click();
    rt.host.desktop.settle(calls(rt, "pickForInput")[1]!.token!, true, { cancelled: true });
    await tick();
    expect(cancelled).toBe(1);
  });

  test("FileSaver.js: a detached <a download> clicked or dispatched saves; setAttribute works too", () => {
    const rt = createRuntime();
    const a = win(rt).document.createElement("a");
    a.href = "blob:sk/42";
    a.download = "notes.txt";
    a.dispatchEvent(new Event("click"));
    const b = win(rt).document.createElement("a");
    b.setAttribute("href", "data:text/plain,hi");
    b.setAttribute("download", "");
    expect(b.getAttribute("download")).toBe("");
    b.click();
    expect(calls(rt, "download").map((c) => c.args)).toEqual([
      ["blob:sk/42", "notes.txt"],
      ["data:text/plain,hi", ""],
    ]);
  });
});
