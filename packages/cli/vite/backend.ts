import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, posix, relative, resolve } from "node:path";
import { normalizePath, type BuildOptions, type Plugin, type UserConfig } from "vite";
import type { AotInputs } from "../aot-inputs";
import { FRAMEWORK_FILE } from "../check/frameworks";
import type { BuildStyles, ModuleGraph } from "../check/graph";
import { collectAssets } from "../cli/assets";
import { nativeSvelteRenderer } from "../cli/config";
import { pruneVendorPrefixes } from "../css-prefixes";
import { declaresDependency, packageDir, RUNTIME_SRC_DIR } from "../runtime-path";
import {
  declaresUtilities,
  findConfig,
  generate as generateTailwind,
  needsTailwindPass,
  tailwindProblem,
} from "../tailwind";
import { htmlPreludeModule, liftIndexHtml, readPackageJson, type CssPiece } from "../vitecompat";
import { nativeVitePlugins, sourceFile } from "./plugin";

export interface NativeViteBuildOptions {
  appRoot: string;
  entry: string;
  outFile: string;
  configFile?: string | undefined;
  skConfig: Record<string, unknown>;
  aliases: Record<string, string>;
  defines: Record<string, string>;
  windowJson: string;
  minify: boolean;
  graphOut?: string | undefined;
  aotInputs?: AotInputs | undefined;
  finishScript: (code: string) => string;
  finishAot?: (code: string, defines: Record<string, string>) => void;
}

const NATIVE_ENTRY = "virtual:skathi/application";
const PRELUDE = "virtual:skathi/document";
const STYLE = /\.(?:css|scss|sass|less|styl|stylus|pcss|postcss)$/i;
const COMPONENT = /\.(?:vue|svelte)$/i;

function atomicWrite(file: string, data: string | Uint8Array): void {
  mkdirSync(dirname(file), { recursive: true });
  const temp = `${file}.${process.pid}.tmp`;
  writeFileSync(temp, data);
  renameSync(temp, file);
}

function nativeBuildOptions(
  user: BuildOptions = {},
  entry: string,
  outDir: string,
  options: NativeViteBuildOptions,
): BuildOptions {
  const bundler = user.rolldownOptions ?? user.rollupOptions ?? {};
  const outputs = Array.isArray(bundler.output) ? bundler.output : [bundler.output];
  const formats = user.lib && user.lib.formats;
  if (formats && formats.length && !formats.includes("es") && !bundler.output) {
    throw new Error('Skathi supports only ES modules (ESM). Include "es" in build.lib.formats.');
  }
  const output = outputs.find((item) => !item?.format || ["es", "esm", "module"].includes(item.format));
  if (!output && outputs.some((item) => item?.format)) {
    throw new Error(
      `Skathi supports only ES modules (ESM). Set build.rolldownOptions.output.format to "es" (received: ${outputs.map((item) => item?.format).join(", ")}).`,
    );
  }
  return {
    ...user,
    outDir,
    write: false,
    copyPublicDir: false,
    emptyOutDir: false,
    watch: null,
    lib: false,
    ssr: false,
    manifest: false,
    ssrManifest: false,
    modulePreload: false,
    target: user.target ?? "esnext",
    cssCodeSplit: false,
    cssMinify: false,
    minify: options.minify ? (user.minify ?? true) : false,
    sourcemap: options.graphOut ? "hidden" : false,
    rolldownOptions: {
      ...bundler,
      platform: "browser",
      input: entry,
      external: [],
      output: {
        ...(output?.plugins && { plugins: output.plugins }),
        ...(output?.assetFileNames && { assetFileNames: output.assetFileNames }),
        ...(output?.banner && { banner: output.banner }),
        ...(output?.footer && { footer: output.footer }),
        ...(output?.postBanner && { postBanner: output.postBanner }),
        ...(output?.postFooter && { postFooter: output.postFooter }),
        ...(output?.intro && { intro: output.intro }),
        ...(output?.outro && { outro: output.outro }),
        ...(output?.charset && { charset: output.charset }),
        ...(output?.hashCharacters && { hashCharacters: output.hashCharacters }),
        ...(output?.comments !== undefined && { comments: output.comments }),
        format: "es",
        entryFileNames: "app.js",
        codeSplitting: false,
      },
    },
  };
}

async function viteForApp(appRoot: string): Promise<typeof import("vite")> {
  if (!declaresDependency(appRoot, "vite-plus")) return import("vite");
  const installed = packageDir("vite-plus", appRoot);
  if (!installed)
    throw new Error(
      "This app declares vite-plus but it is not installed. Install the app's dependencies and run sk again.",
    );
  // Bun expects a filesystem path for the parent; a file: URL can resolve into its global install cache.
  return import(import.meta.resolve("vite-plus", join(installed, "package.json")));
}

/** Compiles a project's Vite module graph into the script and sidecars consumed by the Skathi packager. */
export async function buildWithVite(options: NativeViteBuildOptions): Promise<void> {
  const { appRoot, entry, outFile } = options;
  const outDir = dirname(outFile);
  const { build, loadConfigFromFile } = await viteForApp(appRoot);
  const user = options.configFile
    ? (await loadConfigFromFile({ command: "build", mode: "production" }, options.configFile, appRoot))?.config
    : {};
  if (!user) throw new Error(`Could not load ${options.configFile}`);
  const plugins = await nativeVitePlugins(
    { appRoot, aliases: options.aliases, nativeSvelte: nativeSvelteRenderer(options.skConfig.svelte) },
    user.plugins,
  );
  const htmlPath = join(appRoot, "index.html");
  const page = existsSync(htmlPath) ? liftIndexHtml(htmlPath, appRoot) : undefined;
  for (const warning of page?.advisories ?? []) console.log(`[prod] advisory: ${warning}`);
  const styles = new Set<string>();
  const htmlSheets = new Map<string, CssPiece>();
  // A CSS file ID beside the original sheet lets Vite resolve its imports and url() against the right directory.
  const sheetImports = (pieces: CssPiece[]) =>
    pieces.map((piece) => {
      const file = piece.from.startsWith(htmlPath + "?") ? htmlPath : piece.from;
      styles.add(file);
      const id = normalizePath(join(dirname(file), `.skathi-html-${htmlSheets.size}.css`)) + "?skathi-document";
      htmlSheets.set(id, piece);
      return `import ${JSON.stringify(id)};`;
    });
  const imports = [
    ...sheetImports(page?.headCss ?? []),
    ...(page ? [`import ${JSON.stringify(PRELUDE)};`] : []),
    `import ${JSON.stringify(normalizePath(entry))};`,
    ...sheetImports(page?.bodyCss ?? []),
  ].join("\n");
  const tailwindConfig = findConfig(dirname(entry), appRoot);
  const hasCssConfig =
    user.css?.postcss !== undefined ||
    plugins.some((plugin) => plugin.name.includes("tailwindcss")) ||
    readPackageJson(appRoot)?.postcss !== undefined ||
    new Bun.Glob("{postcss.config.*,.postcssrc,.postcssrc.*}").scanSync({ cwd: appRoot, dot: true }).next().done ===
      false;
  const tailwind = tailwindConfig && !hasCssConfig && !tailwindProblem(tailwindConfig) ? tailwindConfig : null;
  let appOwnsUtilities = false;
  let publicDir = "";
  let finalDefines: Record<string, string> = {};
  const aliases = { ...options.aliases };
  const settings = nativeBuildOptions(user.build, NATIVE_ENTRY, outDir, options);
  const defines = { ...options.defines };
  // Vite expands globs, asset URLs, and the env object itself. Bun needs extra replacements for those.
  for (const key of ["import.meta", "import.meta.glob", "import.meta.env", "import.meta.hot"]) delete defines[key];
  // Vite keeps import.meta.url as the base of asset URLs. The bundled module uses Skathi's page origin.
  defines["import.meta.url"] = JSON.stringify("app://redjs/app.prod.js");
  const target: Plugin = {
    name: "skathi:application",
    enforce: "post",
    config: {
      order: "post",
      handler(config) {
        config.root = appRoot;
        config.base = "/";
        config.build = nativeBuildOptions(config.build, NATIVE_ENTRY, outDir, options);
        config.environments = { client: { ...config.environments?.client, consumer: "client", build: config.build } };
        config.define = { ...config.define, ...defines };
      },
    },
    configResolved(config) {
      publicDir = config.publicDir;
      for (const alias of config.resolve.alias) {
        if (typeof alias.find === "string" && isAbsolute(alias.replacement) && !alias.customResolver) {
          aliases[alias.find] ??= resolve(alias.replacement);
        }
      }
      finalDefines = Object.fromEntries(
        Object.entries(config.define ?? {}).map(([name, value]) => [
          name,
          typeof value === "string" ? value : JSON.stringify(value),
        ]),
      );
    },
    configEnvironment: {
      order: "post",
      handler(name, config) {
        if (name === "client") config.build = nativeBuildOptions(config.build, NATIVE_ENTRY, outDir, options);
      },
    },
    buildStart() {
      if (this.environment.config.build.write)
        throw new Error("A Vite plugin enabled writing the web build during Skathi packaging.");
    },
    resolveId(id) {
      if (id === NATIVE_ENTRY || id === PRELUDE) return "\0" + id;
      if (htmlSheets.has(id)) return id;
      return null;
    },
    load(id) {
      if (id === "\0" + NATIVE_ENTRY) return imports;
      if (id === "\0" + PRELUDE && page) return htmlPreludeModule(page, appRoot);
      return htmlSheets.get(id)?.text ?? null;
    },
    generateBundle() {
      for (const id of this.getModuleIds()) {
        const file = sourceFile(id);
        if (file && (STYLE.test(file) || COMPONENT.test(file))) styles.add(file);
      }
    },
  };
  const scanCss: Plugin = {
    name: "skathi:tailwind",
    enforce: "pre",
    transform(code, id) {
      const file = id.split("?", 1)[0]!;
      if ((STYLE.test(file) || /[?&]type=style(?:&|$)/.test(id)) && declaresUtilities(code)) appOwnsUtilities = true;
      return null;
    },
  };
  const config: UserConfig = {
    ...user,
    root: appRoot,
    base: "/",
    publicDir:
      typeof options.skConfig.publicDir === "string" || options.skConfig.publicDir === false
        ? options.skConfig.publicDir
        : (user.publicDir ?? "public"),
    plugins: [
      ...plugins,
      ...(tailwind ? [scanCss] : []),
      ...(options.aotInputs ? [(await import("./inventory")).viteInventory(options.aotInputs)] : []),
      target,
    ],
    build: settings,
  };
  const result = await build({ ...config, configFile: false });
  if (Array.isArray(result) || !("output" in result)) throw new Error("Skathi requires one non-watching Vite build");
  const outputs = result.output;
  const chunks = outputs.filter((output) => output.type === "chunk");
  const chunk = chunks[0];
  // Rolldown can list inlined dynamic imports of the entry itself in its chunk metadata.
  const external = chunk && [...chunk.imports, ...chunk.dynamicImports].some((name) => name !== chunk.fileName);
  if (chunks.length !== 1 || !chunk?.isEntry || external) {
    const details = chunks
      .map((output) => {
        const imports = [...output.imports, ...output.dynamicImports];
        return imports.length ? `${output.fileName} imports ${imports.join(", ")}` : output.fileName;
      })
      .join("; ");
    throw new Error(
      `Vite must emit one self-contained entry script for Skathi (emitted: ${details}). Check the app's output plugins and external imports.`,
    );
  }
  const assets = outputs.filter((output) => output.type === "asset");
  // Vite emits its aggregate stylesheet with a synthetic original filename. A .css?url
  // import instead has an actual source path and must remain accessible as a payload asset.
  const sheet = assets.find(
    (asset) => asset.names.includes("style.css") && asset.originalFileNames.includes("style.css"),
  );
  let css = sheet ? (typeof sheet.source === "string" ? sheet.source : new TextDecoder().decode(sheet.source)) : "";
  if (tailwind) {
    if (needsTailwindPass(css)) css = await generateTailwind(tailwind, { text: css, from: join(appRoot, "index.css") });
    if (!appOwnsUtilities) css = (await generateTailwind(tailwind)) + "\n" + css;
  }
  css = pruneVendorPrefixes(css);
  const modules = new Set<string>();
  for (const [id, info] of Object.entries(chunk.modules)) {
    const file = sourceFile(id);
    if (!file) continue;
    if (STYLE.test(file) || COMPONENT.test(file)) styles.add(file);
    if (info.renderedLength && FRAMEWORK_FILE.test(file) && !/[?&](?:raw|url)(?:&|$)/.test(id)) modules.add(file);
  }
  if (options.graphOut) {
    const map = chunk.map;
    const graph: ModuleGraph = {
      schema: "skathi.module-graph",
      version: 1,
      entry,
      appRoot,
      aliases,
      declarations: [...new Bun.Glob("*.d.ts").scanSync({ cwd: RUNTIME_SRC_DIR, absolute: true })],
      modules: [...modules].sort(),
      styles: [...styles].sort(),
      tailwind: !!tailwind,
      compiledCss: css,
      errors: [],
      ...(map
        ? {
            bundle: {
              code: chunk.code,
              mappings: map.mappings,
              sources: map.sources.map((id, i) => {
                const file = sourceFile(resolve(outDir, dirname(chunk.fileName), id));
                return file && readFileSync(file, "utf8").replace(/^\uFEFF/, "") === map.sourcesContent?.[i]
                  ? file
                  : null;
              }),
            },
          }
        : {}),
    };
    atomicWrite(options.graphOut, JSON.stringify(graph) + "\n");
    return;
  }
  const code = options.finishScript(
    (css ? `(globalThis.__SK_CSS__ ??= []).push(${JSON.stringify(css)});\n` : "") + chunk.code,
  );
  const packed = new Map<string, string>();
  if (publicDir) {
    const root = dirname(publicDir);
    for (const file of collectAssets(root, [], true, basename(publicDir)).files) packed.set(file.key, file.path);
  }
  for (const asset of assets) {
    if (asset === sheet || asset.fileName === chunk.sourcemapFileName) continue;
    const key = asset.fileName;
    if (
      !key ||
      isAbsolute(key) ||
      /[:\\\0]/.test(key) ||
      posix.normalize(key) !== key ||
      key.startsWith("../") ||
      key === ".." ||
      key === "."
    ) {
      throw new Error(`A Vite plugin emitted an invalid asset filename: ${JSON.stringify(key)}`);
    }
    // Payload keys are case-sensitive, including on file systems where file names are not.
    const file = join(outDir, "vite-assets", createHash("sha256").update(asset.source).digest("hex"));
    if (!existsSync(file)) atomicWrite(file, asset.source);
    packed.set(key, file);
  }
  const styleInfo: BuildStyles = { files: [...styles].sort(), tailwind: !!tailwind, compiledCss: css };
  atomicWrite(join(outDir, "styles.json"), JSON.stringify(styleInfo) + "\n");
  atomicWrite(
    join(outDir, "assets.json"),
    JSON.stringify([...packed].sort(([a], [b]) => a.localeCompare(b)).map(([key, path]) => ({ key, path }))) + "\n",
  );
  atomicWrite(join(outDir, "window.json"), options.windowJson + "\n");
  atomicWrite(outFile, code);
  options.finishAot?.(code, finalDefines);
  console.log(`prod build: ${Math.ceil(Buffer.byteLength(code) / 1024)} KB with Vite -> ${relative(appRoot, outFile)}`);
}
