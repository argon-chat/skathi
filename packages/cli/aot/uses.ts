// What sk-aot's engine run found an image to use (`<ir>.uses.json`, tools/sk-aot/src/uses.rs), held
// against the engine features a workload leaves out.
import { existsSync, readFileSync } from "node:fs";

export interface FeatureUse {
  feature: string;
  id: number;
  /** Named, Unnamed, NamedAtRunTime, or FoundByName: a property read whose name matches the built-in's. */
  kind: string;
  builtin: string | null;
  module: number;
  position: number;
}

export interface Uses {
  entryModules: number[];
  features: FeatureUse[];
  unseen: { cause: string; builtin: string | null; module: number; position: number }[];
  statistics: Record<string, number>;
}

/** The host features (`--sk-host-info`) that leave an engine feature out of the workload. */
export const REMOVED_BY_HOST_FEATURE: Readonly<Record<string, string>> = { "no-wasm": "WebAssembly" };

export const removedFeatures = (hostFeatures: readonly string[]): string[] =>
  hostFeatures.flatMap((f) => (REMOVED_BY_HOST_FEATURE[f] ? [REMOVED_BY_HOST_FEATURE[f]] : []));

export function readUses(path: string): Uses | null {
  if (!existsSync(path)) return null;
  const uses = JSON.parse(readFileSync(path, "utf8")) as Uses & { schema?: string; version?: number };
  if (uses.schema !== "skathi.aot-uses" || uses.version !== 1) throw Error(`${path} is not a skathi.aot-uses v1 file`);
  return uses;
}

/**
 * The uses of `removed` features. Unnamed records (any built-in a dynamic call such as `o[k]()` might
 * reach) do not count, and FoundByName ones count only for the feature's own namespace (a read of
 * `globalThis.WebAssembly`, which the opaque host turns into a read by name): a member's name alone
 * (`exports`, `Module`) matches any object's.
 */
export function usesOf(uses: Uses, removed: readonly string[]): FeatureUse[] {
  return uses.features.filter(
    (use) =>
      removed.includes(use.feature) &&
      (use.kind === "Named" ||
        use.kind === "NamedAtRunTime" ||
        (use.kind === "FoundByName" && use.builtin === use.feature)),
  );
}

/** The module sk-aot's input is in; the others are the engine's own built-ins. */
export const INPUT_MODULE = 0;

/** `line:column` and the code around `position` in `source`. */
export function excerpt(source: string, position: number): string {
  const before = source.slice(0, position);
  const line = before.split("\n").length;
  const column = position - before.lastIndexOf("\n");
  const text = source
    .slice(Math.max(0, position - 40), position + 40)
    .replace(/\s+/g, " ")
    .trim();
  return `${line}:${column}: …${text}…`;
}
