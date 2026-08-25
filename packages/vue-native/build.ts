// vue-native dev builder, v2: two-tier output for REAL Vue HMR.
//
// Usage: bun build.ts [app-entry.js] [dist-dir] [--watch]
//
//   dist/vendor.js            IIFE — @vue/runtime-core + src/runtime.js +
//                             src/loader.js. Installs globalThis.__VN__.
//                             Rebuilt only when packages/vue-native/src/* change.
//   dist/modules/<enc>.js     one CLASSIC SCRIPT per app source file:
//                               __VN__.setCss("<id>", "...")      (if it has <style>)
//                               __VN__.setDeps("<id>", ["<id>", ...])
//                               __VN__.define("<id>", function (require, module, exports) { ... },
//                                             "<bodyHash>")
//                             <enc> = id with "/" -> "__".
//   dist/manifest.json        { rev, vendorRev, entry, modules: ["<id>", ...],
//                               deps: { "<id>": ["<imported id>", ...] } }
//   dist/changes.json         { rev, vendorRev, changed: [{ id, kind }] }
//
// Module ids are workspace-relative POSIX paths ("apps/hello/src/App.vue").
//
// The module GRAPH is what makes a shared plain .js file hot-reloadable: the
// loader walks importers up to the nearest .vue components and reloads those
// through Vue's HMR record instead of restarting the app. It rides along in the
// per-module script (`setDeps`) because the loader never reads manifest.json —
// only the Rust host does. `manifest.deps` mirrors it for tooling (prod builds,
// debugging) and is additively ignored by the host's reader.
//
// `bodyHash` (3rd arg of define) hashes the FACTORY BODY only — not the CSS —
// so a <style>-only edit redefines the module without invalidating its require
// cache entry, leaving the live exports (and the component def Vue's HMR record
// already owns) in place.
//
// TAILWIND rides the same transport as a SYNTHETIC CSS-ONLY MODULE: when the
// app has a tailwind.config.* (see ./tailwind.ts), the generated utilities
// sheet is written as dist/modules/.vn__tailwind.css.js — a real module script
// the host evals like any other, that calls setCss/setDeps/define and whose
// factory is empty. Nothing new in the protocol; the id (".vn/tailwind.css")
// sorts first, so utilities land ahead of every component <style> in the
// aggregate. A template edit that introduces a new class re-runs Tailwind and
// emits a "css-only" change entry — the running app restyles with NO component
// churn (no rerender, no reload, no state loss).
//
// `kind` is derived by diffing each .vue file's previous descriptor blocks:
//   script/scriptSetup changed, or a <style> block's scoped/module/lang shape
//                                changed          -> "reload"
//   template changed                              -> "rerender"   (state survives)
//   only <style> text changed                     -> "css-only"
//   non-.vue source changed                       -> "reload"
//
// Non-negotiables (research report F7 / e.3):
// - hoistStatic is FORCED OFF — our host has no insertStaticContent.
// - inlineTemplate is FORCED OFF — with an inline render the template lives
//   inside setup(), so __VUE_HMR_RUNTIME__.rerender cannot swap it without
//   re-running setup (which is exactly the state loss we are removing).
//   A separate `render` on _sfc_main is what makes rerender state-preserving.
// - the scope id hashes the module id ONLY, never the source, so [data-v-x]
//   stays stable across edits.

import { parse, compileScript, compileTemplate, compileStyle } from "@vue/compiler-sfc";
import type { SFCDescriptor } from "@vue/compiler-sfc";
import { resolve, dirname, join, relative, sep } from "path";
import { mkdirSync, readdirSync, renameSync, statSync, writeFileSync, rmSync } from "fs";

import { TW_ID, findConfig, generate as generateTailwind } from "./tailwind";

// ---- paths ----------------------------------------------------------------

const argv = process.argv.slice(2);
const positional = argv.filter((a) => !a.startsWith("--"));
const WATCH = argv.includes("--watch");

const PKG_DIR = import.meta.dir; // .../packages/vue-native
const WS_ROOT = resolve(PKG_DIR, "../.."); // repo root — module ids are relative to this
const RUNTIME_SRC_DIR = join(PKG_DIR, "src");
const LOADER_ENTRY = join(RUNTIME_SRC_DIR, "loader.js");

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.js"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const DIST = resolve(positional[1] ?? join(PKG_DIR, "dist"));
const MODULES_DIR = join(DIST, "modules");

const SOURCE_EXT = /\.(vue|js|ts|mjs)$/;

const toId = (abs: string) => relative(WS_ROOT, abs).split(sep).join("/");
const encId = (id: string) => id.split("/").join("__") + ".js";
const J = (v: unknown) => JSON.stringify(v);

// Vue's esm-bundler build reads these as bare identifiers.
const DEFINES: Record<string, string> = {
  __DEV__: "true",
  "process.env.NODE_ENV": '"development"',
  __FEATURE_SUSPENSE__: "true",
  __COMPAT__: "false",
  __BROWSER__: "false",
  __ESM_BUNDLER__: "true",
  __GLOBAL__: "false",
  __NODE_JS__: "false",
  __SSR__: "false",
  __TEST__: "false",
  __VERSION__: '"3.5"',
};

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

function ensureDirs() {
  mkdirSync(MODULES_DIR, { recursive: true });
}

// ---- tier 1: vendor.js ----------------------------------------------------

async function buildVendor() {
  const result = await Bun.build({
    entrypoints: [LOADER_ENTRY],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [
      {
        // vue-router (and anything else vendored later) imports from "vue";
        // in the vendor bundle that must be OUR runtime module. Exact-match
        // filter only — bun 1.4 crashes on catch-all onResolve filters.
        name: "vendor-vue-alias",
        setup(b: any) {
          b.onResolve({ filter: /^vue$/ }, () => ({ path: join(RUNTIME_SRC_DIR, "runtime.js") }));
        },
      },
    ],
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("vendor build failed");
  }
  const text = await result.outputs[0].text();
  // Sanity: the pre-seeded "vue" namespace must still carry the compiler
  // helpers that generated render functions call at runtime — if Bun ever
  // tree-shakes the `export *` re-export, templates die with "not a function".
  for (const sym of ["openBlock", "createElementBlock", "createElementVNode", "toDisplayString"]) {
    if (!text.includes(sym)) throw new Error(`vendor bundle is missing ${sym}`);
  }
  writeAtomic(join(DIST, "vendor.js"), text);
  // vendorRev is a CONTENT hash, not a counter: restarting the watcher must
  // not make a running host believe its already-evaluated vendor.js is current.
  vendorRev = Number(BigInt.asUintN(48, BigInt(Bun.hash(text) as any)));
  return text.length;
}

// ---- tier 2: one classic script per app module ----------------------------

/** Stable per-file scope id — must NOT depend on file contents (see header). */
const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/** Filled by the .vue onLoad hook; each Bun.build below handles exactly one file. */
let capturedCss: string | null = null;

function compileVue(absPath: string, source: string): string {
  const id = toId(absPath);
  const { descriptor, errors } = parse(source, { filename: absPath });
  if (errors.length) {
    throw new Error(`${id}: ${errors.map((e) => e.message).join("; ")}`);
  }
  const shortId = scopeIdOf(id);
  const scopeId = `data-v-${shortId}`;
  const hasScoped = descriptor.styles.some((s) => s.scoped);

  const chunks: string[] = [];
  let bindings: any;

  if (descriptor.scriptSetup || descriptor.script) {
    const compiled = compileScript(descriptor, {
      id: shortId,
      inlineTemplate: false, // REQUIRED for rerender-based HMR
      genDefaultAs: "_sfc_main",
    });
    bindings = compiled.bindings;
    chunks.push(compiled.content);
  } else {
    chunks.push("const _sfc_main = {};");
  }

  if (descriptor.template) {
    const t = compileTemplate({
      source: descriptor.template.content,
      filename: absPath,
      id: shortId,
      scoped: hasScoped,
      slotted: descriptor.slotted,
      compilerOptions: {
        hoistStatic: false, // compileTemplate force-sets true; our spread wins (F7)
        bindingMetadata: bindings,
      },
    });
    if (t.errors.length) {
      throw new Error(`${id} <template>: ${t.errors.map(String).join("; ")}`);
    }
    chunks.push(t.code);
    chunks.push("_sfc_main.render = render;");
  }

  if (hasScoped) chunks.push(`_sfc_main.__scopeId = ${J(scopeId)};`);
  chunks.push(`_sfc_main.__file = ${J(id)};`);

  // Our replacement for plugin-vue's `import.meta.hot` epilogue. createRecord
  // is idempotent (returns false when the record exists), so re-evaluating a
  // module never clobbers the record that holds the live instances.
  chunks.push(`_sfc_main.__hmrId = ${J(id)};`);
  chunks.push(
    `if (typeof __VUE_HMR_RUNTIME__ !== "undefined") __VUE_HMR_RUNTIME__.createRecord(${J(id)}, _sfc_main);`,
  );
  chunks.push("export default _sfc_main;");

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    const out = compileStyle({
      source: style.content,
      filename: absPath,
      id: scopeId,
      scoped: style.scoped,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(out.code);
  }
  capturedCss = cssChunks.length ? cssChunks.join("\n") : null;

  return chunks.join("\n");
}

const vuePlugin = {
  name: "vue-sfc",
  setup(build: any) {
    build.onLoad({ filter: /\.vue$/ }, async (args: any) => {
      const source = await Bun.file(args.path).text();
      return { contents: compileVue(args.path, source), loader: "js" };
    });
  },
};

// ---- module dependency graph ----------------------------------------------
// We build every module with external:"*", so each import survives the CJS
// transform as a literal `require("<spec>")` — the specifiers are recoverable
// straight from the emitted body, no second parse of the source needed.
//
// Bare specifiers ("vue", "vue-native", node_modules) are vendor: they live in
// vendor.js, are never hot-reloaded, and are deliberately kept OUT of the graph.

const REQUIRE_RE = /\brequire\(\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\)/g;

/** Same candidate list as loader.js `resolveId` — keep the two in lockstep. */
const CANDIDATE_EXTS = ["", ".js", ".ts", ".mjs", ".vue", "/index.js", "/index.ts", "/index.vue"];

function normalizeId(path: string): string {
  const out: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      out.pop();
      continue;
    }
    out.push(seg);
  }
  return out.join("/");
}

function unquote(raw: string): string | null {
  try {
    if (raw.charCodeAt(0) === 34 /* " */) return JSON.parse(raw);
    const inner = raw.slice(1, -1).replace(/\\'/g, "'").replace(/"/g, '\\"');
    return JSON.parse('"' + inner + '"');
  } catch {
    return null;
  }
}

/** Resolved, deduped, in-graph ids that `id`'s emitted body requires. */
function depsOf(id: string, body: string, ids: Set<string>): string[] {
  const slash = id.lastIndexOf("/");
  const dir = slash < 0 ? "" : id.slice(0, slash);
  const found: string[] = [];
  REQUIRE_RE.lastIndex = 0;
  for (let m = REQUIRE_RE.exec(body); m; m = REQUIRE_RE.exec(body)) {
    const spec = unquote(m[1]);
    if (spec === null || spec.charCodeAt(0) !== 46 /* . */) continue; // bare -> vendor
    const base = normalizeId((dir ? dir + "/" : "") + spec);
    const hit = CANDIDATE_EXTS.map((ext) => base + ext).find((cand) => ids.has(cand));
    if (!hit) {
      // Not a divergence we can paper over: the loader will throw the same way
      // a browser would 404 — say so now, while there is a file to point at.
      console.warn(
        `${stamp()} ${id}: import ${J(spec)} resolves to no built module — ` +
          `require() will throw at runtime`,
      );
      continue;
    }
    if (hit !== id && !found.includes(hit)) found.push(hit);
  }
  return found;
}

const moduleDeps = new Map<string, string[]>(); // id -> dep ids

/** ESM -> CJS for one file, everything else external, wrapped in __VN__.define. */
async function buildModule(absPath: string, ids: Set<string>) {
  const id = toId(absPath);
  capturedCss = null;
  const result = await Bun.build({
    entrypoints: [absPath],
    format: "cjs" as any,
    target: "node",
    external: ["*"], // transform, don't bundle: imports stay as require() calls
    plugins: [vuePlugin],
    define: DEFINES,
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error(`module build failed: ${id}`);
  }
  const body = await result.outputs[0].text();
  if (/\bcreateStaticVNode\b/.test(body)) {
    throw new Error(`${id}: emitted createStaticVNode — hoistStatic leaked back on (F7)`);
  }

  const deps = depsOf(id, body, ids);
  moduleDeps.set(id, deps);

  const out: string[] = [`// vue-native module: ${id}`];
  if (capturedCss !== null) out.push(`__VN__.setCss(${J(id)}, ${J(capturedCss)});`);
  // Guarded: a module script is also readable by an older vendor.js (a dist
  // that was only half-rebuilt) — a missing setDeps must degrade to "no graph",
  // not to a TypeError that kills the whole script.
  out.push(`__VN__.setDeps && __VN__.setDeps(${J(id)}, ${J(deps)});`);
  out.push(`__VN__.define(${J(id)}, function (require, module, exports) {`);
  out.push(body);
  out.push(`}, ${J(Bun.hash(body).toString(16))});`);
  writeAtomic(join(MODULES_DIR, encId(id)), out.join("\n"));
}

// ---- tailwind: the synthetic css-only module -------------------------------
// Absent a tailwind.config.*, EVERY line below is inert and the emitted
// manifest/module set is byte-identical to a pre-Tailwind build.

const TW_CONFIG = findConfig(APP_SRC_DIR, WS_ROOT);
/** Last generated utilities sheet — the change detector for watch mode. */
let twCss: string | null = null;
/** Constant: the factory body never changes, so define() never invalidates. */
const TW_HASH = Bun.hash("").toString(16);

/** Runs Tailwind and rewrites the module script. True when the CSS moved. */
async function buildTailwind(): Promise<boolean> {
  if (!TW_CONFIG) return false;
  const t0 = Date.now();
  const css = await generateTailwind(TW_CONFIG);
  moduleDeps.set(TW_ID, []);
  if (css === twCss) return false;
  twCss = css;
  writeAtomic(
    join(MODULES_DIR, encId(TW_ID)),
    [
      `// vue-native module: ${TW_ID} (synthetic — Tailwind utilities, generated)`,
      `// source: ${toId(TW_CONFIG)}; do not edit`,
      `__VN__.setCss(${J(TW_ID)}, ${J(css)});`,
      `__VN__.setDeps && __VN__.setDeps(${J(TW_ID)}, []);`,
      `__VN__.define(${J(TW_ID)}, function (require, module, exports) {`,
      `}, ${J(TW_HASH)});`,
    ].join("\n"),
  );
  console.log(
    `${stamp()} tailwind: ${(css.length / 1024).toFixed(1)} KB of utilities in ${Date.now() - t0} ms`,
  );
  return true;
}

// ---- change classification ------------------------------------------------

type Blocks = { src: string; script: string; template: string; styleShape: string; styleText: string };
type Kind = "reload" | "rerender" | "css-only";

const prevBlocks = new Map<string, Blocks>(); // abs path -> blocks

function blocksOf(absPath: string, src: string): Blocks {
  if (!absPath.endsWith(".vue")) {
    return { src, script: src, template: "", styleShape: "", styleText: "" };
  }
  let d: SFCDescriptor;
  try {
    d = parse(src, { filename: absPath }).descriptor;
  } catch {
    return { src, script: src, template: "", styleShape: "", styleText: "" };
  }
  const blockKey = (b: any) => (b ? JSON.stringify([b.content, b.lang ?? "", b.attrs ?? {}]) : "");
  return {
    src,
    script: blockKey(d.script) + " " + blockKey(d.scriptSetup),
    template: d.template ? d.template.content : "",
    // scoped/module/lang changes alter codegen or the scope id -> full reload
    styleShape: JSON.stringify(d.styles.map((s) => [!!s.scoped, !!s.module, s.lang ?? ""])),
    styleText: d.styles.map((s) => s.content).join(" "),
  };
}

function classify(prev: Blocks | undefined, next: Blocks): Kind | null {
  if (!prev) return "reload"; // new file
  if (prev.src === next.src) return null;
  if (prev.script !== next.script || prev.styleShape !== next.styleShape) return "reload";
  if (prev.template !== next.template) return "rerender";
  if (prev.styleText !== next.styleText) return "css-only";
  return null; // whitespace between blocks etc.
}

// ---- source discovery -----------------------------------------------------

function scanSources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === "dist" || name.startsWith(".")) continue;
    const abs = join(dir, name);
    const st = statSync(abs);
    if (st.isDirectory()) scanSources(abs, out);
    else if (SOURCE_EXT.test(name)) out.push(abs);
  }
  return out.sort();
}

// ---- manifest / changes ---------------------------------------------------

let rev = 0;
let vendorRev = 0; // content hash of vendor.js, set by buildVendor()

function writeManifest(files: string[]) {
  // The synthetic Tailwind module goes FIRST: the host evals `modules` in
  // order, the loader's cssOrder is first-setCss-wins, so this is what puts
  // the utilities layer ahead of every component <style>. (It also sorts
  // first — "." < any letter — which keeps prod.ts's sorted aggregation in
  // agreement without a second rule.)
  const ids = (TW_CONFIG ? [TW_ID] : []).concat(files.map(toId));
  const seen = new Set<string>();
  for (const id of ids) {
    const enc = encId(id);
    if (seen.has(enc)) throw new Error(`module filename collision for ${id} -> ${enc}`);
    seen.add(enc);
  }
  // `deps` is additive: the host's reader only looks at vendorRev/entry/modules.
  // The loader gets the same edges from each module script's setDeps() call —
  // this copy is for tooling (prod bundling, graph debugging).
  const deps: Record<string, string[]> = {};
  for (const id of ids) deps[id] = moduleDeps.get(id) ?? [];
  writeAtomic(
    join(DIST, "manifest.json"),
    JSON.stringify({ rev, vendorRev, entry: toId(ENTRY_ABS), modules: ids, deps }, null, 2) + "\n",
  );
}

function writeChanges(changed: { id: string; kind: Kind }[]) {
  // Written LAST, and it is the file the host polls: by the time its mtime
  // moves, every module script and the manifest are already on disk.
  writeAtomic(
    join(DIST, "changes.json"),
    JSON.stringify({ rev, vendorRev, changed }) + "\n",
  );
}

// ---- build passes ---------------------------------------------------------

const stamp = () => `[${new Date().toLocaleTimeString()}]`;

async function fullBuild(reason: string) {
  ensureDirs();
  const t0 = Date.now();
  const bytes = await buildVendor(); // also refreshes vendorRev
  const files = scanSources(APP_SRC_DIR);
  const ids = new Set(files.map(toId));
  prevBlocks.clear();
  moduleDeps.clear();
  twCss = null; // force a rewrite: the module file may not exist yet
  await buildTailwind();
  for (const abs of files) {
    const src = await Bun.file(abs).text();
    await buildModule(abs, ids);
    prevBlocks.set(abs, blocksOf(abs, src));
  }
  rev++;
  writeManifest(files);
  // Empty change list: if vendorRev actually moved the host cold-restarts,
  // otherwise this is a no-op ping (e.g. a runtime edit that did not alter the
  // emitted vendor bundle).
  writeChanges([]);
  console.log(
    `${stamp()} full build (${reason}): vendor ${(bytes / 1024) | 0} KB + ${files.length} modules` +
      (TW_CONFIG ? ` + tailwind (${toId(TW_CONFIG)})` : "") +
      ` in ${Date.now() - t0} ms`,
  );
}

async function incrementalBuild() {
  ensureDirs();
  const files = scanSources(APP_SRC_DIR);
  const ids = new Set(files.map(toId));
  const changed: { id: string; kind: Kind }[] = [];
  let structural = false;

  for (const abs of files) {
    const src = await Bun.file(abs).text();
    const next = blocksOf(abs, src);
    const isNew = !prevBlocks.has(abs);
    const kind = classify(prevBlocks.get(abs), next);
    prevBlocks.set(abs, next);
    if (!kind) continue;
    if (isNew) structural = true;
    try {
      await buildModule(abs, ids);
    } catch (e) {
      console.error(`${stamp()} ${String(e)}`);
      return; // leave dist untouched; the app keeps running the last good build
    }
    changed.push({ id: toId(abs), kind });
  }

  const alive = new Set(files);
  for (const abs of Array.from(prevBlocks.keys())) {
    if (alive.has(abs)) continue;
    prevBlocks.delete(abs);
    moduleDeps.delete(toId(abs));
    try {
      rmSync(join(MODULES_DIR, encId(toId(abs))));
    } catch {}
    structural = true;
  }

  // ALWAYS re-run Tailwind: a class list changes with the template (a
  // "rerender" edit), with a `:class` binding (a "reload" edit), and even with
  // a plain .js file that builds class strings — there is no cheaper signal
  // than "some content file moved", which is exactly what got us here.
  // A new utility therefore arrives as its OWN css-only entry, next to (not
  // instead of) whatever the edited component needs.
  if (await buildTailwind()) changed.push({ id: TW_ID, kind: "css-only" });

  if (!changed.length && !structural) return;
  rev++;
  writeManifest(files);
  writeChanges(changed);
  console.log(
    `${stamp()} rev ${rev}: ` +
      (changed.map((c) => `${c.kind} ${c.id}`).join(", ") || "module set changed"),
  );
}

// ---- entry ----------------------------------------------------------------

await fullBuild("startup").catch((e) => {
  console.error(e);
  if (!WATCH) process.exit(1);
});

if (WATCH) {
  const { watch } = await import("fs");
  let timer: any = null;
  let vendorTouched = false;
  const schedule = (isRuntime: boolean) => (_evt: any, file: any) => {
    const name = String(file ?? "");
    if (name.endsWith(".tmp")) return;
    if (isRuntime) vendorTouched = true;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const full = vendorTouched;
      vendorTouched = false;
      try {
        if (full) await fullBuild("runtime source changed");
        else await incrementalBuild();
      } catch (e) {
        console.error(`${stamp()} ${String(e)}`);
      }
    }, 60);
  };
  watch(APP_SRC_DIR, { recursive: true }, schedule(false));
  watch(RUNTIME_SRC_DIR, { recursive: true }, schedule(true));
  const watched = [APP_SRC_DIR, RUNTIME_SRC_DIR];
  if (TW_CONFIG) {
    // The config lives OUTSIDE the source tree (apps/hello/, not apps/hello/src/),
    // so editing the theme would otherwise never reach the running app. One
    // non-recursive watch on its directory, filtered to the config file itself.
    const configDir = dirname(TW_CONFIG);
    const configName = relative(configDir, TW_CONFIG);
    watch(configDir, { recursive: false }, (evt, file) => {
      if (String(file ?? "") !== configName) return;
      schedule(false)(evt, file);
    });
    watched.push(TW_CONFIG);
  }
  console.log(`watching ${watched.join(", ")} ...`);
}
