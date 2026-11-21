import { existsSync, readFileSync, realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve, sep } from "node:path";
import type * as CompilerSfc from "vue/compiler-sfc";

/** The `@skathi-native/runtime` package the builders bundle into every app. */
export const RUNTIME_DIR = realpathSync(
  dirname(createRequire(import.meta.url).resolve("@skathi-native/runtime/package.json")),
);
export const RUNTIME_SRC_DIR = join(RUNTIME_DIR, "src");
export const RUNTIME_ENTRY = join(RUNTIME_SRC_DIR, "runtime.ts");
/** The runtime without Vue's renderer: "@skathi-native/runtime" for an app that has no `vue` installed. */
export const DOM_ENTRY = join(RUNTIME_SRC_DIR, "dom.ts");
export const LOADER_ENTRY = join(RUNTIME_SRC_DIR, "loader.ts");
export const runtimeRequire = createRequire(join(RUNTIME_DIR, "package.json"));

interface PackageJson {
  version?: string;
  main?: string;
  module?: string;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
}

const readPackage = (path: string): PackageJson => JSON.parse(readFileSync(path, "utf8")) as PackageJson;

const RUNTIME_PKG = readPackage(join(RUNTIME_DIR, "package.json"));
export const RUNTIME_VERSION = RUNTIME_PKG.version ?? "0.0.0";
/** The `vue` versions the runtime accepts (its peer dependency). */
export const VUE_RANGE = RUNTIME_PKG.peerDependencies?.vue ?? "^3.5";
export const VUE_MISSING = `this app uses Vue but "vue" is not installed: bun add vue@${VUE_RANGE}`;

/** `native:<name>`: a module the embedding program provides at run time, never a file to bundle. */
export const isNativeModule = (spec: string) => spec.startsWith("native:");

/** `skathi:<name>`: a module of Skathi's own library, served from the runtime package. */
export const STD_MODULES: readonly string[] = [
  "app",
  "binary",
  "encoding",
  "network",
  "numeric",
  "os",
  "runtime",
  "toml",
];
export const isStdModule = (spec: string) => spec.startsWith("skathi:");

/** The runtime file behind `skathi:<name>`; throws for a name Skathi has no module for. */
export function stdModuleFile(spec: string): string {
  const name = spec.slice("skathi:".length);
  if (!STD_MODULES.includes(name)) {
    throw new Error(
      `"${spec}" is not a Skathi module; there are ${STD_MODULES.map((known) => `"skathi:${known}"`).join(", ")}`,
    );
  }
  return join(RUNTIME_SRC_DIR, "std", `${name}.ts`);
}

/** Bare specifiers the builders serve from the runtime (and every `native:` and `skathi:` module); an app cannot alias them. */
export const PRESEEDED: readonly string[] = ["vue", "@skathi-native/runtime", "vue-router", "pinia"];
export const isPreseeded = (spec: string) => PRESEEDED.includes(spec) || isNativeModule(spec) || isStdModule(spec);
/** Libraries the builders give one shared copy, the app's own install; never part of the runtime. */
export const FRAMEWORK_LIBS: readonly string[] = ["vue-router", "pinia"];
export const NATIVE_MODULES_ENTRY = join(RUNTIME_SRC_DIR, "native-modules.ts");

const requireFrom = (dir: string) => createRequire(join(dir, "package.json"));

/** Package `name`'s real directory as Node finds it from `dir`; unlike require.resolve under Bun, never auto-installed. */
export function packageDir(name: string, dir: string): string | null {
  for (let at = resolve(dir); ;) {
    const candidate = join(at, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) return realpathSync(candidate);
    const up = dirname(at);
    if (up === at) return null;
    at = up;
  }
}

export function declaresDependency(appRoot: string, name: string): boolean {
  try {
    const pkg = readPackage(join(appRoot, "package.json"));
    return [pkg.dependencies, pkg.devDependencies, pkg.peerDependencies].some((deps) => deps?.[name] !== undefined);
  } catch {
    return false;
  }
}

export interface AppVue {
  version: string;
  /** The @vue/runtime-core entry the runtime's renderer is built on: the app's own when it declares one, else `vue`'s. */
  runtimeCore: string;
  /** The node_modules that entry is installed in. */
  modulesDir: string;
}

/** The Vue the app at `appRoot` has installed, or null when `vue` does not resolve from it. */
export function appVue(appRoot: string): AppVue | null {
  const vue = packageDir("vue", appRoot);
  if (!vue) return null;
  const coreDir =
    (declaresDependency(appRoot, "@vue/runtime-core") ? packageDir("@vue/runtime-core", appRoot) : null) ??
    packageDir("@vue/runtime-core", vue);
  if (!coreDir) return null;
  const core = readPackage(join(coreDir, "package.json"));
  const runtimeCore = join(coreDir, core.module ?? core.main ?? "index.js");
  const at = coreDir.lastIndexOf(`${sep}node_modules${sep}`);
  return {
    version: readPackage(join(vue, "package.json")).version ?? "unknown",
    runtimeCore,
    modulesDir: at < 0 ? dirname(coreDir) : coreDir.slice(0, at + `${sep}node_modules`.length),
  };
}

export type SfcCompiler = typeof CompilerSfc;

const compilers = new Map<string, SfcCompiler>();

/** `vue/compiler-sfc` as resolved from `dir`: the compiler of the app's own Vue, loaded on first use. */
export function sfcCompiler(dir: string): SfcCompiler {
  const vue = packageDir("vue", dir);
  if (!vue) throw new Error(VUE_MISSING);
  let compiler = compilers.get(vue);
  if (!compiler) {
    compiler = requireFrom(vue)("./compiler-sfc") as SfcCompiler;
    compilers.set(vue, compiler);
  }
  return compiler;
}
