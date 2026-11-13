import { appStorage, createApp, h, ref } from "vue-native";

const urls = ref<string[]>([]);
let started = 0;
let loads: number[] = [];
let errors: string[] = [];

(globalThis as any).__imgs = {
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
