// vue-native dev builder, v2: two-tier output for REAL Vue HMR.
//
// Usage: bun build.ts [app-entry.js] [dist-dir] [--watch]
//
//   dist/vendor.js            IIFE — @vue/runtime-core + src/runtime.js +
//                             src/loader.js. Installs globalThis.__VN__.
//                             Rebuilt only when packages/vue-native/src/* change.
//   dist/modules/<enc>.js     one CLASSIC SCRIPT per app source file:
//                               __VN__.setCss("<id>", "...")      (if it has <style>)
//                               __VN__.define("<id>", function (require, module, exports) { ... })
//                             <enc> = id with "/" -> "__".
//   dist/manifest.json        { rev, vendorRev, entry, modules: ["<id>", ...] }
//   dist/changes.json         { rev, vendorRev, changed: [{ id, kind }] }
//
// Module ids are workspace-relative POSIX paths ("apps/hello/src/App.vue").
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

/** ESM -> CJS for one file, everything else external, wrapped in __VN__.define. */
async function buildModule(absPath: string) {
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

  const out: string[] = [`// vue-native module: ${id}`];
  if (capturedCss !== null) out.push(`__VN__.setCss(${J(id)}, ${J(capturedCss)});`);
  out.push(`__VN__.define(${J(id)}, function (require, module, exports) {`);
  out.push(body);
  out.push("});");
  writeAtomic(join(MODULES_DIR, encId(id)), out.join("\n"));
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
  const ids = files.map(toId);
  const seen = new Set<string>();
  for (const id of ids) {
    const enc = encId(id);
    if (seen.has(enc)) throw new Error(`module filename collision for ${id} -> ${enc}`);
    seen.add(enc);
  }
  writeAtomic(
    join(DIST, "manifest.json"),
    JSON.stringify({ rev, vendorRev, entry: toId(ENTRY_ABS), modules: ids }, null, 2) + "\n",
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
  prevBlocks.clear();
  for (const abs of files) {
    const src = await Bun.file(abs).text();
    await buildModule(abs);
    prevBlocks.set(abs, blocksOf(abs, src));
  }
  rev++;
  writeManifest(files);
  // Empty change list: if vendorRev actually moved the host cold-restarts,
  // otherwise this is a no-op ping (e.g. a runtime edit that did not alter the
  // emitted vendor bundle).
  writeChanges([]);
  console.log(
    `${stamp()} full build (${reason}): vendor ${(bytes / 1024) | 0} KB + ${files.length} modules in ${Date.now() - t0} ms`,
  );
}

async function incrementalBuild() {
  ensureDirs();
  const files = scanSources(APP_SRC_DIR);
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
      await buildModule(abs);
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
    try {
      rmSync(join(MODULES_DIR, encId(toId(abs))));
    } catch {}
    structural = true;
  }

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
  console.log(`watching ${APP_SRC_DIR} and ${RUNTIME_SRC_DIR} ...`);
}
