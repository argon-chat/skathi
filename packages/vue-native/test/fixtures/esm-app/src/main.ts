// Fixture entry — see test/build-graph.test.ts "a "type": "module" app".
// Not part of any shipped app. The package is "type": "module", which is what
// made Bun emit Node's interop (`__toESM(require("./App.vue"), 1)`).
import App from "./App.vue";
import * as util from "./util";

export const app = App;
export const ns = util;
export const lazy = () => import("./Lazy.vue");
// import.meta.glob, expanded by the dev builder as prod.ts expands it.
export const eagerNames = import.meta.glob("./kinds/*.ts", { eager: true, import: "name" });
export const lazyKinds = import.meta.glob(["./kinds/*.ts", "!./kinds/b.ts"]);
