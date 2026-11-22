// `run` starts a scenario that needs no gesture (`nogesture`: every picker must reject).
import { createApp, h, onMounted, ref } from "@skathi-native/runtime";
import { TOML } from "skathi:toml";
import settings from "./settings.toml";

type Handle = {
  kind: "file" | "directory";
  name: string;
  queryPermission(d?: { mode?: string }): Promise<string>;
  isSameEntry(other: unknown): Promise<boolean>;
  getFile(): Promise<File>;
  createWritable(o?: {
    keepExistingData?: boolean;
  }): Promise<{ write(d: unknown): Promise<void>; close(): Promise<void> }>;
  entries(): AsyncIterable<[string, Handle]>;
};
type FileReaderLike = {
  onload: (() => void) | null;
  onerror: (() => void) | null;
  result: unknown;
  readAsText(b: Blob): void;
};
const g = globalThis as unknown as {
  showOpenFilePicker(o?: unknown): Promise<Handle[]>;
  showSaveFilePicker(o?: unknown): Promise<Handle>;
  showDirectoryPicker(o?: unknown): Promise<Handle>;
  FileReader: new () => FileReaderLike;
  structuredClone<T>(v: T): T;
  __SK_TEST_FILES__: unknown;
};

const events: string[] = [];
const results: Record<string, unknown> = {};
const nameOf = (e: unknown) => (e as { name?: string })?.name ?? String(e);

function readAsText(b: Blob): Promise<string> {
  return new Promise((done, fail) => {
    const r = new g.FileReader();
    r.onload = () => done(String(r.result));
    r.onerror = () => fail(new Error("FileReader failed"));
    r.readAsText(b);
  });
}

async function onChange(e: Event) {
  const input = e.target as HTMLInputElement;
  const list = input.files;
  const files: File[] = [];
  for (let i = 0; list && i < list.length; i++) files.push(list.item(i)!);
  const out = [];
  for (const f of files) {
    out.push({ name: f.name, size: f.size, type: f.type, reader: await readAsText(f), text: await f.text() });
  }
  results.change = { value: input.value, count: files.length, files: out };
}

async function openOne() {
  try {
    const [h] = await g.showOpenFilePicker({
      types: [{ description: "Text", accept: { "text/plain": [".txt"] } }],
    });
    const file = await h!.getFile();
    const before = await file.text();
    const w = await h!.createWritable();
    await w.write("changed");
    await w.close();
    const clone = g.structuredClone(h!);
    results.open = {
      kind: h!.kind,
      name: h!.name,
      permission: await h!.queryPermission({ mode: "readwrite" }),
      before,
      after: await (await h!.getFile()).text(),
      cloneIsHandle: clone !== h && clone.name === h!.name,
      cloneSame: await clone.isSameEntry(h),
      clonePermission: await clone.queryPermission(),
      cloneText: await (await clone.getFile()).text(),
      keys: Object.keys(h!),
      json: JSON.stringify(h),
    };
  } catch (e) {
    results.open = `ERR ${nameOf(e)}: ${(e as Error).message}`;
  }
}

async function openDir() {
  try {
    const d = await g.showDirectoryPicker();
    const entries: string[] = [];
    for await (const [name, h] of d.entries()) entries.push(`${h.kind}:${name}`);
    results.dir = { kind: d.kind, name: d.name, entries: entries.sort() };
  } catch (e) {
    results.dir = `ERR ${nameOf(e)}`;
  }
}

// VueUse's `useFileDialog` and FileSaver.js: elements made by document.createElement and clicked, never inserted.
function loosePick() {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".txt";
  input.addEventListener("change", () => {
    results.loosePick = Array.from(input.files ?? [], (f) => f.name);
  });
  input.click();
}

function looseSave() {
  const a = document.createElement("a");
  const url = URL.createObjectURL(new Blob(["loose"], { type: "text/plain" }));
  a.setAttribute("href", url);
  a.setAttribute("download", "loose.txt");
  a.click();
  URL.revokeObjectURL(url);
}

async function noGesture() {
  const out: string[] = [];
  for (const call of [() => g.showOpenFilePicker(), () => g.showSaveFilePicker(), () => g.showDirectoryPicker()]) {
    try {
      await call();
      out.push("resolved");
    } catch (e) {
      out.push(nameOf(e));
    }
  }
  results.nogesture = out.join("|");
}

const tag = (v: unknown) => Object.prototype.toString.call(v).slice(8, -1);
function same(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null || tag(a) !== tag(b)) return false;
  if (a instanceof Date) return Object.is(a.getTime(), (b as Date).getTime());
  if (tag(a).startsWith("Temporal.")) return String(a) === String(b);
  const ka = Object.keys(a);
  const kb = Object.keys(b);
  const ra = a as Record<string, unknown>;
  const rb = b as Record<string, unknown>;
  return ka.length === kb.length && ka.every((k, i) => k === kb[i] && same(ra[k], rb[k]));
}
function shape(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(shape);
  if (tag(v) === "Object") return Object.fromEntries(Object.entries(v as object).map(([k, x]) => [k, shape(x)]));
  return typeof v === "object" ? tag(v) : typeof v;
}

g.__SK_TEST_FILES__ = {
  events,
  results,
  run(name: string) {
    if (name === "nogesture") void noGesture();
  },
  toml(text: string) {
    return JSON.stringify({ same: same(settings, TOML.parse(text)), shape: shape(settings) });
  },
};

const csv = URL.createObjectURL(new Blob(["a,b\n1,2\n"], { type: "text/csv" }));
const shown = ref(false);
const button = (id: string, label: string, onClick: () => void) =>
  h("button", { id, onClick, style: { padding: "4px 10px" } }, label);

createApp({
  setup() {
    onMounted(() => {
      shown.value = true;
    });
    return () =>
      h(
        "div",
        {
          id: "root",
          style: { padding: "12px", display: "flex", flexDirection: "column", gap: "8px", background: "#ffffff" },
        },
        [
          h("div", { id: "status" }, shown.value ? "ready" : "loading"),
          h("input", {
            id: "pick",
            type: "file",
            multiple: true,
            accept: ".txt,text/plain",
            onInput: () => events.push("input"),
            onChange: (e: Event) => {
              events.push("change");
              void onChange(e);
            },
            onCancel: () => events.push("cancel"),
          }),
          h("label", { id: "label", for: "pick" }, "Choose text files"),
          h("div", { style: { display: "flex", gap: "8px" } }, [
            button("open", "Open", () => void openOne()),
            button("dir", "Folder", () => void openDir()),
            h("a", { id: "dl", href: csv, download: "report.csv" }, "Download"),
          ]),
          h("div", { style: { display: "flex", gap: "8px" } }, [
            button("loose-pick", "Loose pick", loosePick),
            button("loose-save", "Loose save", looseSave),
          ]),
        ],
      );
  },
}).mount();
