// A promise does not survive the eval round trip, so each run records how it SETTLED into
// `results`, and the spec polls that.
import { createApp, h, onMounted, ref } from "vue-native";

type Note = { id?: number; title: string; done: boolean };

const req = <T>(q: VnIDBRequest<T>): Promise<T> =>
  new Promise((resolve, reject) => {
    q.onsuccess = () => resolve(q.result);
    q.onerror = () => reject(q.error);
  });
const done = (tx: VnIDBTransaction): Promise<void> =>
  new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error);
  });
/** (`!`: engine.d.ts types rAF as optional; under vn-app it never is.) */
const frame = () => new Promise<void>((r) => requestAnimationFrame!(() => r()));

const status = ref("opening");
const notes = ref<Note[]>([]);
let db: VnIDBDatabase | null = null;

async function openNotes(): Promise<VnIDBDatabase> {
  const open = indexedDB.open("vn-test-notes", 1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore("notes", { keyPath: "id", autoIncrement: true });
    store.createIndex("byDone", "done");
  };
  return req(open);
}

async function reload() {
  const tx = db!.transaction("notes");
  notes.value = (await req(tx.objectStore("notes").getAll())) as Note[];
}

async function addNote() {
  const tx = db!.transaction("notes", "readwrite");
  const store = tx.objectStore("notes");
  const n = (await req(store.count())) + 1;
  store.add({ title: `note ${n}`, done: n % 2 === 0 });
  await done(tx);
  await reload();
}

async function clearNotes() {
  const tx = db!.transaction("notes", "readwrite");
  tx.objectStore("notes").clear();
  await done(tx);
  await reload();
}

const results: Record<string, string> = {};

const scenarios: Record<string, () => Promise<string>> = {
  /** The surface a feature-detecting library probes before it picks a driver. */
  async surface() {
    return [
      typeof indexedDB,
      typeof IDBKeyRange,
      String(Object.prototype.toString.call(indexedDB)),
      typeof navigator!.storage.estimate,
    ].join("|");
  },

  /**
   * Frames are counted on the host's own clock; picked up once per pump, this would be ~500
   * frames.
   */
  async chain() {
    const open = indexedDB.open("vn-test-chain", 1);
    open.onupgradeneeded = () => {
      const s = open.result.createObjectStore("s");
      for (let i = 0; i < 500; i++) s.put({ i }, i);
    };
    const cdb = await req(open);
    let frames = 0;
    let counting = true;
    const tick = () => {
      if (!counting) return;
      frames++;
      requestAnimationFrame!(tick);
    };
    requestAnimationFrame!(tick);
    const t0 = performance.now();
    let sum = 0;
    for (let i = 0; i < 500; i++) sum += ((await req(cdb.transaction("s").objectStore("s").get(i))) as { i: number }).i;
    const ms = performance.now() - t0;
    counting = false;
    cdb.close();
    return `${sum}|${frames}|${Math.round(ms)}`;
  },

  async versionchange() {
    const first = await req(indexedDB.open("vn-test-vc", 1));
    const seen: string[] = [];
    first.onversionchange = (e) => {
      const ev = e as VnIDBVersionChangeEvent;
      seen.push(`vc ${ev.oldVersion}->${ev.newVersion}`);
      first.close();
    };
    const open = indexedDB.open("vn-test-vc", 2);
    open.onblocked = () => seen.push("blocked");
    open.onupgradeneeded = (e) => {
      seen.push(`upgrade ${e.oldVersion}->${e.newVersion}`);
      open.result.createObjectStore("added");
    };
    const second = await req(open);
    const stores = [...second.objectStoreNames].join(",");
    second.close();
    await req(indexedDB.deleteDatabase("vn-test-vc"));
    return `${seen.join(",")}|v${second.version}|${stores}`;
  },

  async constraint() {
    const open = indexedDB.open("vn-test-unique", 1);
    open.onupgradeneeded = () =>
      open.result.createObjectStore("u", { keyPath: "k" }).createIndex("byMail", "mail", { unique: true });
    const udb = await req(open);
    const tx = udb.transaction("u", "readwrite");
    const s = tx.objectStore("u");
    s.add({ k: 1, mail: "a@x" });
    const dup = s.add({ k: 2, mail: "a@x" });
    const failed = req(dup).then(
      () => "no error",
      (e: VnDOMException) => e.name,
    );
    const ended = done(tx).then(
      () => "committed",
      (e: VnDOMException) => `aborted ${e.name}`,
    );
    const out = `${await failed}|${await ended}`;
    const left = await req(udb.transaction("u").objectStore("u").count());
    udb.close();
    return `${out}|${left}`;
  },

  async structured() {
    const open = indexedDB.open("vn-test-clone", 1);
    open.onupgradeneeded = () => open.result.createObjectStore("v");
    const vdb = await req(open);
    const value = {
      when: new Date(0),
      map: new Map([["a", 1]]),
      set: new Set([1, 2]),
      bytes: new Uint8Array([1, 2, 3]),
      blob: new Blob(["hi"], { type: "text/plain" }),
      nested: { list: [1, "two", null] },
    };
    const tx = vdb.transaction("v", "readwrite");
    tx.objectStore("v").put(value, "x");
    await done(tx);
    const back = (await req(vdb.transaction("v").objectStore("v").get("x"))) as typeof value;
    vdb.close();
    return [
      back.when instanceof Date && back.when.getTime(),
      back.map instanceof Map && back.map.get("a"),
      back.set instanceof Set && back.set.size,
      back.bytes instanceof Uint8Array && back.bytes.join(","),
      back.blob instanceof Blob && (await back.blob.text()),
      JSON.stringify(back.nested),
      back === value ? "same" : "copy",
    ].join("|");
  },

  async storage() {
    const names = (await indexedDB.databases()).map((d) => d.name).sort();
    const est = await navigator!.storage.estimate();
    return `${names.includes("vn-test-notes")}|${est.usage > 0}|${est.quota}|${await navigator!.storage.persisted()}`;
  },
};

const g = globalThis as unknown as Record<string, unknown>;
g.__VN_TEST_IDB__ = {
  results,
  run(name: string) {
    delete results[name];
    const f = scenarios[name];
    if (!f) {
      results[name] = `ERR no scenario ${name}`;
      return;
    }
    void frame()
      .then(frame)
      .then(f)
      .then(
        (v) => (results[name] = v),
        (e: { name?: string; message?: string }) => (results[name] = `ERR ${e?.name}: ${e?.message}`),
      );
  },
};

createApp({
  setup() {
    onMounted(async () => {
      try {
        db = await openNotes();
        await reload();
        status.value = "ready";
      } catch (e) {
        status.value = `failed: ${(e as VnDOMException).name}`;
      }
    });
    return () =>
      h("div", { id: "root", style: { padding: "12px", display: "flex", flexDirection: "column", gap: "6px" } }, [
        h("div", { id: "status" }, status.value),
        h("div", { id: "count" }, `count=${notes.value.length}`),
        h("div", { style: { display: "flex", gap: "8px" } }, [
          h("button", { id: "add", onClick: () => void addNote() }, "add"),
          h("button", { id: "clear", onClick: () => void clearNotes() }, "clear"),
        ]),
        h(
          "div",
          { id: "notes" },
          notes.value.map((n) =>
            h("div", { class: "note", key: n.id! }, `${n.id}:${n.title}${n.done ? " (done)" : ""}`),
          ),
        ),
      ]);
  },
}).mount();
