import { categories, categoryNotes, entries } from "../data/std/entries";
import type { Reference } from "./reference";
import { bindings } from "../../scripts/bindings-reference";
import { rustBinding } from "../../scripts/rust-bindings";
import { apiEntries } from "./api-entries";

const api = bindings();
export const moduleOverview = [...api.moduleSummaries];

export const stdReference: Reference = {
  id: "std",
  title: "Skathi modules",
  noun: "classes, namespaces, functions",
  categories,
  categoryNotes,
  entries: categories.flatMap((category) => {
    const scope = api.std.get(category)!;
    const guides = entries
      .filter((entry) => entry.category === category)
      .map((entry) => {
        const names = Array.isArray(entry.declaration) ? entry.declaration : [entry.name];
        const root = names[0]!.replace(/\(\)$/, "").split(".")[0]!;
        const symbol = scope.get(root);
        const nativeBinding =
          symbol && (rustBinding(entry.category, root) ?? rustBinding(entry.category, api.typeName(symbol) ?? root));
        const source = symbol && api.describe(symbol).source;
        const provenance = { ...(source && { source }), ...(nativeBinding && { nativeBinding }) };
        if (entry.link || (!entry.declaration && entry.syntax)) return { ...entry, ...provenance };
        const lines = names.map((name) => api.syntax(scope, name, true));
        if (lines.includes(undefined)) {
          if (entry.declaration)
            throw new Error(`Missing public declaration for ${entry.category} / ${names.join(", ")}`);
          return { ...entry, ...provenance };
        }
        const imported = [...new Set(names.map((name) => name.replace(/\(\)$/, "").split(".")[0]!))];
        const imports = imported.map((name) => (api.typeOnly(scope.get(name)!) ? `type ${name}` : name));
        const syntax = `import { ${imports.join(", ")} } from "${entry.category}";\n\n${lines.join("\n\n")}`;
        return { ...entry, ...provenance, syntax };
      });
    return apiEntries(category, scope, guides);
  }),
  source: "docs/src/data/std/entries.ts",
};
