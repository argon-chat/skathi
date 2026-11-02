// A fixture for packages/vn-test/specs/http-cache.spec.ts: a grid of http(s)
// <img>, every one of them on screen, loaded from a server the spec runs.
//
// Nothing loads until the spec calls `__imgs.start(urls)`; each image's `load`
// (or `error`) is timed from that call, so a round is "how long until the
// N-th picture was there". `__imgs.clear()` unmounts the grid, and
// `__imgs.stats()` reads the round back. `__imgs.get(url, init)` is a `fetch`
// whose outcome lands in `__imgs.got`, and `__imgs.encrypt(flag)` flips the
// storage's encryption, which the HTTP cache follows. Render functions rather
// than an SFC, for the reason fixtures/layout gives.
import { appStorage, createApp, h, ref } from "vue-native";

const urls = ref<string[]>([]);
let started = 0;
let loads: number[] = [];
let errors: string[] = [];

(globalThis as any).__imgs = {
  /** `fetch` from the page; the outcome lands in `__imgs.got` (JSON). */
  get(url: string, init?: RequestInit) {
    (globalThis as any).__imgs.got = "";
    fetch(url, init)
      .then(async (r) => {
        const n = (await r.arrayBuffer()).byteLength;
        return { status: r.status, bytes: n, url: r.url, redirected: r.redirected };
      })
      .catch((e) => ({ error: String((e && e.message) || e) }))
      .then((v) => ((globalThis as any).__imgs.got = JSON.stringify(v)));
    return 1;
  },
  got: "",
  /** The storage's encryption, which the HTTP cache follows (docs/STORAGE.md §12). */
  encrypt(on: boolean) {
    appStorage.setStorageFolder(appStorage.folder, { encrypted: on });
    return appStorage.encrypted;
  },
  start(list: string[]) {
    started = performance.now();
    loads = [];
    errors = [];
    urls.value = list;
    return list.length;
  },
  clear() {
    urls.value = [];
  },
  stats() {
    return { n: urls.value.length, loads: loads.slice(), errors: errors.slice() };
  },
};

createApp({
  render: () =>
    h(
      "div",
      {
        id: "root",
        style: {
          display: "flex",
          flexWrap: "wrap",
          alignContent: "flex-start",
          width: "100%",
          height: "100%",
          background: "#fff",
        },
      },
      urls.value.map((src) =>
        h("img", {
          key: src,
          src,
          alt: "",
          style: { width: "24px", height: "24px", margin: "1px" },
          onLoad: () => loads.push(performance.now() - started),
          onError: () => errors.push(src),
        }),
      ),
    ),
}).mount();
