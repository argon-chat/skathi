import { appModuleError } from "../esm";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type { Api as VuePluginApi } from "@vitejs/plugin-vue";
import {
  build as viteBuild,
  normalizePath,
  type Alias,
  type BuildOptions,
  type ConfigEnv,
  type Plugin,
  type PluginOption,
  type UserConfig,
} from "vite";
import type { AssetURLOptions, AssetURLTagConfig } from "vue/compiler-sfc";
import { protectHapticPatterns } from "../haptic-patterns";
import {
  appVue,
  DOM_ENTRY,
  NATIVE_MODULES_ENTRY,
  RUNTIME_ENTRY,
  RUNTIME_SRC_DIR,
  sfcCompiler,
  STD_MODULES,
  stdModuleFile,
} from "../runtime-path";
import {
  rewriteWorkletUrls,
  stubWorkerUrls,
  TEMPLATE_ASSET_URLS,
  templateJsonUrls,
  tomlError,
  tomlModule,
  wasmStubModule,
  workerStubModule,
  workletAsset,
} from "../vitecompat";

import type { NativeVitePluginOptions } from "./public";
export type { NativeVitePluginOptions } from "./public";

const NATIVE = "\0skathi:native:";
const WORKLET = "\0skathi:worklet:";
const SVELTE_CSS = "?skathi-svelte.css";
const DEFAULT_VUE = Symbol.for("@skathi-native/cli/default-vue-plugin");
const OWN_PLUGINS = new Set(["skathi:modules", "skathi:svelte", "skathi:template-assets"]);

export function sourceFile(id: string): string | null {
  const file = id.split("?", 1)[0]!;
  return isAbsolute(file) && existsSync(file) ? resolve(file) : null;
}

async function defaultVuePlugin(): Promise<Plugin> {
  const shipped = new URL("../dist/vue-adapter.mjs", import.meta.url);
  const specifier = existsSync(shipped) ? shipped.href : "@vitejs/plugin-vue";
  const { default: vue } = (await import(specifier)) as typeof import("@vitejs/plugin-vue");
  return vue();
}

async function collectPlugins(options: PluginOption[], out: Plugin[]): Promise<void> {
  for (const option of options) {
    const plugin = await option;
    if (Array.isArray(plugin)) await collectPlugins(plugin, out);
    else if (plugin && !OWN_PLUGINS.has(plugin.name) && !(DEFAULT_VUE in plugin)) out.push(plugin);
  }
}

type CompilerFilter = (config: UserConfig, env: ConfigEnv) => boolean;

function compilerFilter(plugin: Plugin): CompilerFilter {
  const apply = plugin.apply;
  if (typeof apply !== "function") return (_config, env) => !apply || apply === env.command;
  // The SDK and Vite must use the same decision for the same config evaluation.
  const results = new WeakMap<ConfigEnv, boolean>();
  plugin.apply = (config, env) => {
    let result = results.get(env);
    if (result === undefined) {
      result = apply(config, env);
      results.set(env, result);
    }
    return result;
  };
  return plugin.apply;
}

function templateAssets(options: boolean | AssetURLTagConfig | AssetURLOptions | undefined) {
  if (options === false) return false;
  if (!options || options === true) return { ...TEMPLATE_ASSET_URLS, includeAbsolute: false };
  const isTagMap = Object.values(options).some(Array.isArray);
  const settings = isTagMap ? {} : (options as AssetURLOptions);
  const tags = isTagMap ? (options as AssetURLTagConfig) : settings.tags;
  return { includeAbsolute: false, ...settings, tags: { ...TEMPLATE_ASSET_URLS.tags, ...tags } };
}

function configureVue(plugin: Plugin, appRoot: string): void {
  const api = plugin.api as VuePluginApi | undefined;
  if (!api?.options)
    throw new Error("The configured Vue plugin must expose compiler options. Install @vitejs/plugin-vue 6.");
  const options = api.options;
  const compiler = options.compiler || sfcCompiler(appRoot);
  const original = options.template?.compilerOptions;
  api.options = {
    ...options,
    compiler: {
      ...compiler,
      async compileStyleAsync(input) {
        const protectedStyle = protectHapticPatterns(input.source);
        const result = await compiler.compileStyleAsync({ ...input, source: protectedStyle.css });
        return { ...result, code: protectedStyle.restore(result.code) };
      },
    },
    template: {
      ...options.template,
      transformAssetUrls: templateAssets(options.template?.transformAssetUrls),
      compilerOptions: {
        // A kept comment makes a comment-first template a fragment root, which loses the parent's scope id.
        comments: false,
        ...original,
        hoistStatic: false,
        isCustomElement: (tag) => tag.startsWith("sk-") || tag === "lottie" || !!original?.isCustomElement?.(tag),
      },
    },
  };
}

function nativeModules(options: NativeVitePluginOptions): Plugin {
  const installedVue = appVue(options.appRoot);
  const stubs = new Map<string, string>();
  const files = {
    "@skathi-native/runtime": installedVue ? RUNTIME_ENTRY : DOM_ENTRY,
    "@skathi-native/runtime/svelte": join(RUNTIME_SRC_DIR, "svelte.ts"),
    "@skathi-native/runtime/svelte-renderer": join(RUNTIME_SRC_DIR, "svelte-renderer.ts"),
    ...Object.fromEntries(STD_MODULES.map((name) => [`skathi:${name}`, stdModuleFile(`skathi:${name}`)])),
    ...(installedVue && { vue: RUNTIME_ENTRY, "@vue/runtime-core": installedVue.runtimeCore }),
    ...options.aliases,
  };
  const alias: Alias[] = Object.entries(files).map(([name, file]) => ({
    find: new RegExp("^" + name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "$"),
    replacement: normalizePath(file),
  }));
  alias.unshift({ find: /^native:(.+)$/, replacement: NATIVE + "$1.cjs" });
  const worklets = new Map<string, Promise<string>>();
  let workletBuild: Pick<UserConfig, "define" | "resolve"> & { minify: BuildOptions["minify"] } = { minify: true };
  // A worklet scope evaluates one classic script, so the module is built on its own and emitted as an asset.
  const bundleWorklet = async (file: string): Promise<{ key: string; code: string }> => {
    const name = normalizePath(relative(options.appRoot, file));
    const { minify, ...shared } = workletBuild;
    let chunks: string[];
    try {
      const result = await viteBuild({
        ...shared,
        configFile: false,
        root: options.appRoot,
        logLevel: "silent",
        build: {
          write: false,
          minify: minify ?? true,
          copyPublicDir: false,
          rolldownOptions: {
            input: file,
            output: { format: "iife", entryFileNames: "worklet.js", codeSplitting: false },
          },
        },
      });
      if (Array.isArray(result) || !("output" in result)) throw new Error("the build produced no script");
      if (result.output.some((out) => out.type !== "chunk"))
        throw new Error("it imports a file that is not script or JSON, and a worklet has nowhere to load one from");
      chunks = result.output.flatMap((out) => (out.type === "chunk" ? [out.code] : []));
      if (chunks.length !== 1) throw new Error("it did not build into one script");
    } catch (error) {
      throw new Error(
        `${name}: the audio worklet module did not bundle: ${error instanceof Error ? error.message : String(error)}\n` +
          `A worklet module is built into one self-contained script, so everything it imports must be inlinable.`,
      );
    }
    return { key: workletAsset(file, options.appRoot).key, code: chunks[0]! };
  };
  const workletUrl = (emit: (file: { type: "asset"; fileName: string; source: string }) => string, file: string) => {
    const path = resolve(file);
    let url = worklets.get(path);
    if (!url) {
      url = bundleWorklet(path).then(({ key, code }) => {
        emit({ type: "asset", fileName: key, source: code });
        return workletAsset(path, options.appRoot).url;
      });
      worklets.set(path, url);
    }
    return url;
  };
  return {
    name: "skathi:modules",
    enforce: "pre",
    config: {
      order: "post",
      handler() {
        return {
          resolve: { alias, dedupe: ["svelte", "vue-router", "pinia"] },
          ...(options.nativeSvelte && {
            environments: {
              client: { resolve: { conditions: ["svelte-custom-renderer", "browser", "module", "production"] } },
            },
          }),
        };
      },
    },
    configResolved: {
      order: "pre",
      handler(config) {
        workletBuild = {
          minify: config.build.minify,
          define: config.define ?? {},
          resolve: { alias: [...config.resolve.alias] },
        };
        if (installedVue) {
          for (const plugin of config.plugins) if (plugin.name === "vite:vue") configureVue(plugin, options.appRoot);
        }
      },
    },
    generateBundle(output) {
      if (output.format !== "es")
        this.error("Skathi supports only ES modules (ESM). Set the Vite output format to es.");
    },
    moduleParsed(info) {
      const error = appModuleError(info.id, info.inputFormat);
      if (error && sourceFile(info.id)) this.error({ id: info.id, message: error });
    },
    async resolveId(id, importer) {
      if (id.startsWith(NATIVE)) return id;
      if (!/[?&](?:sharedworker|worker|worklet)(?:[=&]|$)|\.wasm(?:\?|$)/i.test(id)) return null;
      const [path, query] = id.split("?", 2);
      if (!path) return null;
      const params = new Set(new URLSearchParams(query).keys());
      const worker = params.has("worker") || params.has("sharedworker");
      const worklet = !worker && params.has("worklet");
      if (!worker && !worklet && !path.toLowerCase().endsWith(".wasm")) return null;
      const file = await this.resolve(path, importer, { skipSelf: true });
      if (!file || file.external) return null;
      if (worklet) return WORKLET + encodeURIComponent(file.id) + ".js";
      const name = normalizePath(relative(options.appRoot, file.id));
      const virtual = "\0skathi:stub:" + encodeURIComponent(file.id + "?" + (query ?? "")) + ".js";
      stubs.set(virtual, worker ? workerStubModule(name, params.has("url")) : wasmStubModule(name, params));
      return virtual;
    },
    async load(id) {
      if (id.startsWith(WORKLET)) {
        const file = decodeURIComponent(id.slice(WORKLET.length, -".js".length));
        return `export default ${JSON.stringify(await workletUrl((asset) => this.emitFile(asset), file))};\n`;
      }
      if (/\.toml$/i.test(id) && !id.startsWith("\0")) {
        const text = await readFile(id, "utf8");
        const module = tomlModule(text);
        if (module.loader === "js") return module.contents;
        const at = await tomlError(id, text);
        this.error({
          message: `${normalizePath(relative(options.appRoot, id))}:${at?.line ?? 1}:${at?.column ?? 1}: ${at?.message ?? "invalid TOML"}`,
          id,
          ...(at && { loc: { file: id, line: at.line, column: at.column } }),
        });
      }
      if (!id.startsWith(NATIVE)) return stubs.get(id);
      const name = id.slice(NATIVE.length, -".cjs".length);
      return `module.exports = require(${JSON.stringify(normalizePath(NATIVE_MODULES_ENTRY))}).nativeModule(${JSON.stringify(name)});`;
    },
    async transform(code, id) {
      if (!/\.(?:[cm]?[jt]sx?|vue|svelte)(?:\?|$)/.test(id)) return null;
      let out = code;
      // A Vite asset URL would read import.meta.url before the constructor can report the unsupported worker.
      if (out.includes("Worker"))
        out = stubWorkerUrls(out, normalizePath(relative(options.appRoot, id)), false)?.code ?? out;
      // Vite's own asset URL would emit the module's source as written, not a script a worklet scope can run.
      const specs = new Set<string>();
      rewriteWorkletUrls(out, (spec) => (specs.add(spec), ""));
      if (specs.size) {
        const from = dirname(id.split("?", 1)[0]!);
        const urls = new Map<string, string>();
        for (const spec of specs)
          urls.set(spec, await workletUrl((asset) => this.emitFile(asset), resolve(from, spec)));
        out = rewriteWorkletUrls(out, (spec) => JSON.stringify(urls.get(spec)))!.code;
      }
      return out === code ? null : out;
    },
  };
}

function nativeSveltePlugin(options: NativeVitePluginOptions, configured?: Plugin): Plugin {
  if (configured) {
    const restoreStyles = new Map<string, (css: string) => string>();
    return {
      name: "skathi:svelte",
      enforce: "pre",
      configResolved: {
        order: "post",
        handler() {
          const api = configured.api as
            { options?: { emitCss?: boolean; compilerOptions?: Record<string, unknown> } } | undefined;
          if (!api?.options)
            throw new Error(
              "Use @sveltejs/vite-plugin-svelte 7 for native builds, or omit it to use Skathi’s Svelte compiler.",
            );
          const compiler = (api.options.compilerOptions ??= {});
          api.options.emitCss = true;
          compiler.fragments = "tree";
          compiler.css = "external";
          if (options.nativeSvelte) {
            compiler.experimental = {
              ...(typeof compiler.experimental === "object" && compiler.experimental),
              customRenderer: "@skathi-native/runtime/svelte-renderer",
            };
          }
        },
      },
      async transform(source, id) {
        if (!/@haptic-pattern\b/i.test(source)) return null;
        const [filename, query] = id.split("?", 2);
        if (!filename?.endsWith(".svelte")) return null;
        if (query) {
          return new URLSearchParams(query).get("type") === "style" ? restoreStyles.get(filename)?.(source) : null;
        }
        const { svelteCompiler } = await import("../svelte");
        const compiler = await svelteCompiler(options.appRoot);
        const transformed = await compiler.preprocess(
          source,
          {
            style({ content }) {
              const patterns = protectHapticPatterns(content);
              restoreStyles.set(filename, patterns.restore);
              return { code: patterns.css };
            },
          },
          { filename },
        );
        const map = transformed.map;
        return { code: transformed.code, map: map && typeof map === "object" ? JSON.stringify(map) : (map ?? null) };
      },
    };
  }
  const sheets = new Map<string, string>();
  return {
    name: "skathi:svelte",
    enforce: "pre",
    resolveId(id) {
      return id.endsWith(SVELTE_CSS) ? id : null;
    },
    load(id) {
      return id.endsWith(SVELTE_CSS) ? sheets.get(id) : null;
    },
    async transform(code, id) {
      if (!/\.svelte(?:\.[jt]s)?$/.test(id)) return;
      const { compileSvelte } = await import("../svelte");
      const output = await compileSvelte(options.appRoot, id, code, {
        dev: false,
        native: options.nativeSvelte ?? false,
        filename: relative(options.appRoot, id).split(sep).join("/"),
      });
      for (const message of output.warnings) this.warn(message);
      if (!output.css) return { code: output.js, moduleType: "js" };
      const styleId = id + SVELTE_CSS;
      sheets.set(styleId, output.css);
      return { code: `import ${JSON.stringify(styleId)};\n${output.js}`, moduleType: "js" };
    },
  };
}

/** Assemble a native Vite plugin list around the plugins in the application's Vite config. */
export async function nativeVitePlugins(
  options: NativeVitePluginOptions,
  userPlugins: PluginOption[] = [],
): Promise<Plugin[]> {
  const plugins: Plugin[] = [];
  await collectPlugins(userPlugins, plugins);
  const vuePlugins = plugins.filter((plugin) => plugin.name === "vite:vue");
  if (appVue(options.appRoot) && !vuePlugins.some((plugin) => !plugin.apply)) {
    const applies = vuePlugins.map(compilerFilter);
    const fallback = Object.assign(await defaultVuePlugin(), { [DEFAULT_VUE]: true });
    fallback.apply = (config, env) => !applies.some((matches) => matches(config, env));
    plugins.push(fallback);
  }
  const sveltePlugins = ["vite-plugin-svelte:config", "vite-plugin-svelte"].flatMap((name) =>
    plugins.filter((p) => p.name === name),
  );
  const choices = sveltePlugins.map((plugin) => ({ plugin, applies: compilerFilter(plugin) }));
  const svelte = [...sveltePlugins, undefined].map((plugin) => {
    const adapter = nativeSveltePlugin(options, plugin);
    if (choices.length) adapter.apply = (config, env) => choices.find((c) => c.applies(config, env))?.plugin === plugin;
    return adapter;
  });
  return [
    nativeModules(options),
    ...plugins,
    ...svelte,
    {
      name: "skathi:template-assets",
      enforce: "post",
      transform(code, id) {
        if (/\.vue(?:$|\?)/.test(id) && code.includes(".json")) {
          const result = templateJsonUrls(code);
          if (code !== result) return result;
        }
        return null;
      },
    },
  ];
}
