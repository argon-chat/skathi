// A census of the `__vn_*` host protocol: how many calls, of which kind, does each thing an app does cost?
//
//   bun scripts/host-calls.ts [--json]
//
// Runs the real renderer against a host that only counts. An instrument, not a test.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  BaseTransition,
  BaseTransitionPropsValidators,
  createRenderer,
  createVNode,
  defineComponent,
  Fragment,
  getCurrentInstance,
  getTransitionRawChildren,
  h,
  nextTick,
  resolveTransitionHooks,
  setTransitionHooks,
  toRaw,
  useTransitionState,
  version,
} from "@vue/runtime-core";
import { reactive, ref } from "@vue/runtime-core";

const HERE = dirname(fileURLToPath(import.meta.url));
const RUNTIME_PATH = join(HERE, "../src/runtime.ts");

const IMPORT_LINE = /^\s*import\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
const STAR_REEXPORT = /^\s*export\s+\*\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;

const HOST_FNS = [
  "__vn_intern",
  "__vn_create_element",
  "__vn_create_text",
  "__vn_create_anchor",
  "__vn_create_root",
  "__vn_set_text",
  "__vn_set_element_text",
  "__vn_insert",
  "__vn_remove",
  "__vn_set_style",
  "__vn_set_class",
  "__vn_set_prop",
  "__vn_add_scope",
  "__vn_set_stylesheet",
  "__vn_set_listener",
  "__vn_set_dispatcher",
  "__vn_query",
  "__vn_bounds",
  "__vn_computed_style",
  "__vn_media_state",
  "__vn_media_call",
  "__vn_media_can_play",
  "__vn_log",
] as const;

type HostFn = (typeof HOST_FNS)[number];
type Counts = Partial<Record<HostFn, number>>;

interface World {
  createApp: (component: unknown) => { mount: (container?: unknown) => unknown; unmount(): void };
  counts: Counts;
  take(): Counts;
  reset(): void;
  global: Record<string, unknown>;
}

function makeWorld(): World {
  const transpiler = new Bun.Transpiler({
    loader: "ts",
    tsconfig: JSON.stringify({ compilerOptions: { target: "esnext", useDefineForClassFields: false } }),
  });
  const body = transpiler
    .transformSync(readFileSync(RUNTIME_PATH, "utf8"))
    .replace(IMPORT_LINE, "")
    .replace(STAR_REEXPORT, "")
    .replace(/^export\s+(function|const|let)\s/gm, "$1 ");

  const counts: Counts = {};
  const bump = (name: HostFn) => {
    counts[name] = (counts[name] ?? 0) + 1;
  };

  let nextId = 1;
  const names: string[] = [];
  const stub: Record<HostFn, (...args: never[]) => unknown> = {
    __vn_intern: ((n: string) => {
      bump("__vn_intern");
      let i = names.indexOf(n);
      if (i < 0) i = names.push(n) - 1;
      return i;
    }) as never,
    __vn_create_element: (() => (bump("__vn_create_element"), nextId++)) as never,
    __vn_create_text: (() => (bump("__vn_create_text"), nextId++)) as never,
    __vn_create_anchor: (() => (bump("__vn_create_anchor"), nextId++)) as never,
    __vn_create_root: (() => (bump("__vn_create_root"), nextId++)) as never,
    __vn_set_text: (() => bump("__vn_set_text")) as never,
    __vn_set_element_text: (() => bump("__vn_set_element_text")) as never,
    __vn_insert: (() => bump("__vn_insert")) as never,
    __vn_remove: (() => bump("__vn_remove")) as never,
    __vn_set_style: (() => bump("__vn_set_style")) as never,
    __vn_set_class: (() => bump("__vn_set_class")) as never,
    __vn_set_prop: (() => bump("__vn_set_prop")) as never,
    __vn_add_scope: (() => bump("__vn_add_scope")) as never,
    __vn_set_stylesheet: (() => bump("__vn_set_stylesheet")) as never,
    __vn_set_listener: (() => bump("__vn_set_listener")) as never,
    __vn_set_dispatcher: (() => bump("__vn_set_dispatcher")) as never,
    // The read-back calls answer plausibly: a `""` from `__vn_bounds` would short-circuit `rectOf` and under-count.
    __vn_query: (() => (bump("__vn_query"), "")) as never,
    __vn_bounds: (() => (bump("__vn_bounds"), "0,0,100,20")) as never,
    __vn_computed_style: (() => (bump("__vn_computed_style"), "display=block\ntransition-duration=0s")) as never,
    __vn_media_state: (() => (bump("__vn_media_state"), "")) as never,
    __vn_media_call: (() => (bump("__vn_media_call"), true)) as never,
    __vn_media_can_play: (() => (bump("__vn_media_can_play"), "")) as never,
    __vn_log: (() => bump("__vn_log")) as never,
  };

  const fakeGlobal: Record<string, unknown> = {
    document: { body: {}, documentElement: {}, head: {} },
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 0),
  };

  const params = [
    "createRenderer",
    "createVNode",
    "defineComponent",
    "Fragment",
    "getCurrentInstance",
    "getTransitionRawChildren",
    "h",
    "resolveTransitionHooks",
    "setTransitionHooks",
    "toRaw",
    "useTransitionState",
    "BaseTransition",
    "BaseTransitionPropsValidators",
    "version",
    "globalThis",
    "console",
    ...HOST_FNS,
  ];
  const args: unknown[] = [
    createRenderer,
    createVNode,
    defineComponent,
    Fragment,
    getCurrentInstance,
    getTransitionRawChildren,
    h,
    resolveTransitionHooks,
    setTransitionHooks,
    toRaw,
    useTransitionState,
    BaseTransition,
    BaseTransitionPropsValidators,
    version,
    fakeGlobal,
    { log() {}, warn() {}, error() {} },
    ...HOST_FNS.map((n) => stub[n]),
  ];

  const exported = new Function(...params, `${body}\nreturn { createApp };`)(...args) as {
    createApp: World["createApp"];
  };

  return {
    createApp: exported.createApp,
    counts,
    take() {
      const out = { ...counts };
      for (const k of Object.keys(counts) as HostFn[]) delete counts[k];
      return out;
    },
    reset() {
      for (const k of Object.keys(counts) as HostFn[]) delete counts[k];
    },
    global: fakeGlobal,
  };
}

const total = (c: Counts) => Object.values(c).reduce((a, b) => a + b, 0);

const ROWS = 100;

function row(state: { items: { id: number; label: string }[]; hot: number }, i: number) {
  const item = state.items[i]!;
  return h(
    "div",
    {
      key: item.id,
      class: i === state.hot ? "row row--hot" : "row",
      style: { paddingLeft: `${8 + (i % 3) * 4}px`, opacity: "1" },
      "data-index": String(i),
      "aria-selected": i === state.hot ? "true" : "false",
      onClick: () => {},
    },
    [h("span", { class: "label" }, item.label)],
  );
}

async function main() {
  const results: { name: string; note: string; counts: Counts }[] = [];
  const record = (name: string, note: string, counts: Counts) => results.push({ name, note, counts });

  const world = makeWorld();
  const state = reactive({
    items: Array.from({ length: ROWS }, (_, i) => ({ id: i, label: `item ${i}` })),
    hot: -1,
    theme: "light",
  });
  const bump = ref(0);

  const App = defineComponent({
    setup() {
      return () =>
        h("div", { class: `app app--${state.theme}` }, [
          h("h1", { class: "title" }, `count ${bump.value}`),
          h(
            "div",
            { class: "list" },
            state.items.map((_, i) => row(state, i)),
          ),
        ]);
    },
  });

  const app = world.createApp(App);
  world.reset();
  app.mount();
  record("mount", `${ROWS} rows, each: class + 2 attrs + 2 style decls + 1 listener`, world.take());

  bump.value++;
  await nextTick();
  record("text-only update", "one text node changes; nothing else in the tree does", world.take());

  state.hot = 5;
  await nextTick();
  record("one row's class + one attr", "the narrowest real interaction: a hover/selection", world.take());

  state.theme = "dark";
  await nextTick();
  record("root class swap", "a theme toggle — one class write, no child touched", world.take());

  state.items = state.items.slice().reverse();
  await nextTick();
  record("reverse the keyed list", `${ROWS} keyed nodes MOVE; no node is created`, world.take());

  state.items = state.items.slice(0, 50);
  await nextTick();
  record("drop half the rows", "50 subtrees removed", world.take());

  state.items = Array.from({ length: ROWS }, (_, i) => ({ id: 1000 + i, label: `fresh ${i}` }));
  await nextTick();
  record("replace every row", `${ROWS} new keys: full teardown + rebuild`, world.take());

  app.unmount();
  record("unmount", "every node freed", world.take());

  print(results);
}

function print(results: { name: string; note: string; counts: Counts }[]) {
  if (process.argv.includes("--json")) {
    console.log(JSON.stringify(results, null, 2));
    return;
  }
  const used = HOST_FNS.filter((fn) => results.some((r) => r.counts[fn]));
  const short = (fn: string) => fn.replace("__vn_", "");
  const nameW = Math.max(...results.map((r) => r.name.length));
  const head = ["scenario".padEnd(nameW), "total".padStart(6), ...used.map((f) => short(f).padStart(14))];
  console.log(head.join(" "));
  console.log("-".repeat(head.join(" ").length));
  for (const r of results) {
    const cells = used.map((f) => String(r.counts[f] ?? "").padStart(14));
    console.log([r.name.padEnd(nameW), String(total(r.counts)).padStart(6), ...cells].join(" "));
  }
  console.log();
  for (const r of results) console.log(`  ${r.name}: ${r.note}`);
}

await main();
