// Fixture entry — see test/build-graph.test.ts "a "type": "module" app".
// Not part of any shipped app. The package is "type": "module", which is what
// made Bun emit Node's interop (`__toESM(require("./App.vue"), 1)`).
import App from "./App.vue";
import * as util from "./util";
import { state } from "./util";
import { viaAlias, aliasedJson } from "./alias";
import config, { title } from "./data/config.json";
import meta from "../meta.json";
import note from "./data/note.txt?raw";
import Work from "./work.ts?worker";

export const app = App;
export const ns = util;
export const lazy = () => import("./Lazy.vue");
// import.meta.glob, expanded by the dev builder as prod.ts expands it.
export const eagerNames = import.meta.glob("./kinds/*.ts", { eager: true, import: "name" });
export const lazyKinds = import.meta.glob(["./kinds/*.ts", "!./kinds/b.ts"]);
export const texts = import.meta.glob("./data/*.txt", { query: "?raw", import: "default", eager: true });

// The dev-build blockers the Argon client hit: a tsconfig alias, JSON in and
// outside src/, `?raw`, and `?worker` both static and through import().
export const sameInstance = viaAlias === state;
export const json = { config, title, meta, aliasedJson };
export const raw = note;
export const worker = Work;
export const lazyWorker = () => import("./work.ts?worker");
