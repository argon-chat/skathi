// `run` starts a scenario that needs no gesture (`nogesture`: every picker must reject).
import { createApp, h, onMounted, ref } from "vue-native";

type Handle = {
  kind: "file" | "directory";
  name: string;
  queryPermission(d?: { mode?: string }): Promise<string>;
  isSameEntry(other: unknown): Promise<boolean>;
  getFile(): Promise<File>;
  createWritable(o?: { keepExistingData?: boolean }): Promise<{ write(d: unknown): Promise<void>; close(): Promise<void> }>;
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
  __VN_TEST_FILES__: unknown;
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

async function noGesture() {
  const out: string[] = [];
  for (const call of [
    () => g.showOpenFilePicker(),
    () => g.showSaveFilePicker(),
    () => g.showDirectoryPicker(),
  ]) {
    try {
      await call();
      out.push("resolved");
    } catch (e) {
      out.push(nameOf(e));
    }
  }
  results.nogesture = out.join("|");
}

g.__VN_TEST_FILES__ = {
  events,
  results,
  run(name: string) {
    if (name === "nogesture") void noGesture();
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
      h("div", { id: "root", style: { padding: "12px", display: "flex", flexDirection: "column", gap: "8px", background: "#ffffff" } }, [
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
      ]);
  },
}).mount();
