import { realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, sep } from "node:path";

/** The `@skathi-native/runtime` package the builders bundle into every app. */
export const RUNTIME_DIR = realpathSync(
  dirname(createRequire(import.meta.path).resolve("@skathi-native/runtime/package.json")),
);
export const RUNTIME_SRC_DIR = join(RUNTIME_DIR, "src");
export const RUNTIME_ENTRY = join(RUNTIME_SRC_DIR, "runtime.ts");
export const LOADER_ENTRY = join(RUNTIME_SRC_DIR, "loader.ts");
/** Resolves from the runtime package: its own vue-router and pinia, for an app that declares neither. */
export const runtimeRequire = createRequire(join(RUNTIME_DIR, "package.json"));

/** The node_modules the runtime's dependencies are installed in (hoisted to the workspace root in a checkout). */
export const RUNTIME_MODULES_DIR = (() => {
  const dep = realpathSync(runtimeRequire.resolve("@vue/runtime-core"));
  const at = dep.lastIndexOf(`${sep}node_modules${sep}`);
  return at < 0 ? join(RUNTIME_DIR, "node_modules") : dep.slice(0, at + `${sep}node_modules`.length);
})();

/** Bare specifiers the builders serve from the runtime; an app cannot alias them. */
export const PRESEEDED: readonly string[] = ["vue", "@skathi-native/runtime", "vue-router", "pinia"];
export const isPreseeded = (spec: string) => PRESEEDED.includes(spec);
