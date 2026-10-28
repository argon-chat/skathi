// A fixture for packages/vn-test/specs/indexeddb-disk.spec.ts (m25 P2,
// research/reports/m25/indexeddb-p2-impl.md): IndexedDB ON DISK, across real
// restarts of the host.
//
// Boot, in this order, because IndexedDB latches its folder and key at its
// first use:
//   1. the launch's own argv picks the encryption — `encrypt` calls
//      `appStorage.setStorageFolder(<the current folder>, { encrypted: true })`,
//      `decrypt` the same with `{ encrypted: false }`, neither calls nothing
//      (the sticky rule decides);
//   2. the database opens. A failure is RENDERED (`#status` = `failed: <name>`)
//      rather than thrown: the second-instance test reads it.
//
// `__VN_TEST_IDB__.run(name, ...args)` starts a scenario over the devtools
// `eval` and records how it settled into `results[name]` (a promise does not
// survive the round trip): `put key value`, `get key`, `open`, `databases`,
// `info`, `late encrypted`.
import { appLaunch, appStorage, createApp, h, onMounted, ref } from "vue-native";

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

const argv = appLaunch.argv;
const asked = argv.includes("encrypt") ? true : argv.includes("decrypt") ? false : null;
if (asked !== null) appStorage.setStorageFolder(appStorage.folder, { encrypted: asked });

const status = ref("opening");
let db: VnIDBDatabase | null = null;

function openKv(): Promise<VnIDBDatabase> {
  const open = indexedDB.open("vn-test-persist", 1);
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
  /** Opens the database now (the second instance retries after the first exits). */
  async open() {
    db = await openKv();
    status.value = "ready";
    return "ok";
  },
  async databases() {
    return (await indexedDB.databases()).map((d) => `${d.name}@${d.version}`).join(",");
  },
  async estimate() {
    const e = await navigator!.storage.estimate();
    return `${e.usage}|${e.quota}`;
  },
  async info() {
    return JSON.stringify(appStorage.info().indexedDB);
  },
  /** A setStorageFolder AFTER IndexedDB started — for the next start. */
  async late(encrypted: string) {
    appStorage.setStorageFolder(appStorage.folder, { encrypted: encrypted === "true" });
    return JSON.stringify(appStorage.info().indexedDB);
  },
};

const g = globalThis as unknown as Record<string, unknown>;
g.__VN_TEST_IDB__ = {
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
        const err = e as VnDOMException;
        status.value = `failed: ${err.name}`;
        results.boot = `${err.name}: ${err.message}`;
      }
    });
    return () => h("div", { id: "root", style: { padding: "12px" } }, [h("div", { id: "status" }, status.value)]);
  },
}).mount();
