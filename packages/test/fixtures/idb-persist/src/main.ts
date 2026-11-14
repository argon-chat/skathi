import { appLaunch, appStorage, createApp, h, onMounted, ref } from "@skathi-native/runtime";

const req = <T>(q: IDBRequest<T>): Promise<T> =>
  new Promise((resolve, reject) => {
    q.onsuccess = () => resolve(q.result);
    q.onerror = () => reject(q.error);
  });
const done = (tx: IDBTransaction): Promise<void> =>
  new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error);
  });

const argv = appLaunch.argv;
const asked = argv.includes("encrypt") ? true : argv.includes("decrypt") ? false : null;
if (asked !== null) appStorage.setStorageFolder(appStorage.folder, { encrypted: asked });
else if (argv.includes("keep")) appStorage.setStorageFolder(appStorage.folder);

const status = ref("opening");
let db: IDBDatabase | null = null;

function openKv(): Promise<IDBDatabase> {
  const open = indexedDB.open("sk-test-persist", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("kv");
  return req(open);
}

const results: Record<string, string> = {};

const scenarios: Record<string, (...args: string[]) => Promise<string>> = {
  async put(key: string, value: string) {
    const tx = db!.transaction("kv", "readwrite");
    tx.objectStore("kv").put({ value, at: Date.now() }, key);
    await done(tx);
    return "ok";
  },
  async get(key: string) {
    const v = (await req(db!.transaction("kv").objectStore("kv").get(key))) as { value: string } | undefined;
    return v ? v.value : "<missing>";
  },
  async open() {
    db = await openKv();
    status.value = "ready";
    return "ok";
  },
  async databases() {
    return (await indexedDB.databases()).map((d) => `${d.name}@${d.version}`).join(",");
  },
  async estimate() {
    const e = await navigator.storage.estimate();
    return `${e.usage}|${e.quota}`;
  },
  async info() {
    return JSON.stringify(appStorage.info().indexedDB);
  },
  async late(encrypted: string) {
    if (encrypted === "keep") appStorage.setStorageFolder(appStorage.folder);
    else appStorage.setStorageFolder(appStorage.folder, { encrypted: encrypted === "true" });
    return JSON.stringify(appStorage.info().indexedDB);
  },
};

const g = globalThis as unknown as Record<string, unknown>;
g.__SK_TEST_IDB__ = {
  results,
  run(name: string, ...args: string[]) {
    delete results[name];
    const f = scenarios[name];
    if (!f) {
      results[name] = `ERR no scenario ${name}`;
      return;
    }
    f(...args).then(
      (v) => (results[name] = v),
      (e: { name?: string; message?: string }) => (results[name] = `ERR ${e?.name}: ${e?.message}`),
    );
  },
};

createApp({
  setup() {
    onMounted(async () => {
      try {
        db = await openKv();
        status.value = "ready";
      } catch (e) {
        const err = e as DOMException;
        status.value = `failed: ${err.name}`;
        results.boot = `${err.name}: ${err.message}`;
      }
    });
    return () => h("div", { id: "root", style: { padding: "12px" } }, [h("div", { id: "status" }, status.value)]);
  },
}).mount();
