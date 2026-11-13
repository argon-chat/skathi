import { appStorage, createApp, h, ref } from "vue-native";

const urls = ref<string[]>([]);
let started = 0;
let loads: number[] = [];
let errors: string[] = [];

interface Imgs {
  get(url: string, init?: RequestInit): number;
  got: string;
  encrypt(on: boolean): boolean;
  start(list: string[]): number;
  clear(): void;
  stats(): { n: number; loads: number[]; errors: string[] };
}
const imgs: Imgs = {
  get(url: string, init?: RequestInit) {
    imgs.got = "";
    fetch!(url, init)
      .then(async (r) => {
        const n = (await r.arrayBuffer()).byteLength;
        return { status: r.status, bytes: n, url: r.url, redirected: r.redirected };
      })
      .catch((e: unknown) => ({ error: String((e as { message?: unknown } | null)?.message || e) }))
      .then((v) => (imgs.got = JSON.stringify(v)));
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

(globalThis as unknown as { __imgs: Imgs }).__imgs = imgs;

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
