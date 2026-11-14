// Every Vite import shape skathi's prod builder handles (vitecompat.ts).
import { createApp } from "vue";
import App from "./App.vue";
import notes from "./data/notes.txt?raw";
import logoUrl from "./assets/logo.png";
import iconUrl from "./assets/icon.svg?url";
import Icon from "./assets/icon.svg";
import FixWorker from "./work.ts?worker";
import wasmUrl from "./mod.wasm?url";

declare const __FIX_BUILD__: { branch: string; n: number };
declare const __FIX_FLAG__: boolean;

const kinds = import.meta.glob("./kinds/*.ts", { eager: true, import: "name" });
const lazy = import.meta.glob(["./kinds/*.ts", "!./kinds/b.ts"]);

(globalThis as Record<string, unknown>).__fixture = {
  notes,
  logoUrl,
  iconUrl,
  Icon,
  FixWorker,
  wasmUrl,
  kinds,
  lazyKeys: Object.keys(lazy),
  build: __FIX_BUILD__,
  flag: __FIX_FLAG__,
  greeting: import.meta.env.VITE_GREETING,
  spawn: () => new Worker(new URL("./work.ts", import.meta.url), { type: "module" }),
};

createApp(App).mount("#app");
