import type ts from "typescript";
import { bindings } from "../../scripts/bindings-reference";
import { rustBinding } from "../../scripts/rust-bindings";
import type { RefEntry } from "./reference";

type Entry = RefEntry & { declaration?: true | string[] };

function slugPart(name: string): string {
  return (
    name
      .replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
      .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "export"
  );
}

/** Attach directly used public types to their API. Shared types can be read at each call site. */
export function apiEntries(scope: string, symbols: ReadonlyMap<string, ts.Symbol>, entries: Entry[]): RefEntry[] {
  const api = bindings();
  const claimed = new Set<string>();
  const namesFor = (entry: Entry): string[] => {
    const names = [...entry.name.split(/,\s*/), ...(entry.aliases ?? [])];
    if (Array.isArray(entry.declaration)) names.push(...entry.declaration);
    return [...new Set(names.map((name) => name.replace(/\(\)$/, "")).filter((name) => symbols.has(name)))];
  };
  const names = entries.map(namesFor);
  for (const keys of names) for (const key of keys) claimed.add(key);

  const covered = new Set(claimed);
  const withTypes = (entry: RefEntry, keys: readonly string[]): RefEntry => {
    if (entry.link || !["supported", "partial", "declared"].includes(entry.status)) return entry;
    const types = new Set<string>();
    const aliases = new Set(entry.aliases);
    for (const key of keys) {
      const symbol = symbols.get(key)!;
      const shape = api.typeName(symbol);
      if (shape && symbols.has(shape) && shape !== key && !claimed.has(shape) && entry.syntax) {
        aliases.add(shape);
        covered.add(shape);
      }
      if (key !== entry.name) aliases.add(key);
      for (const type of api.referencedTypes(symbols, symbol)) {
        if (type !== shape && !claimed.has(type)) types.add(type);
      }
    }
    for (const name of types) {
      covered.add(name);
      aliases.add(name);
    }
    return {
      ...entry,
      ...(aliases.size > 0 && { aliases: [...aliases] }),
      ...(types.size > 0 && { types: [...types].sort().map((name) => api.describe(symbols.get(name)!, true)) }),
    };
  };
  const result = entries.map((entry, i) => withTypes(entry, names[i]!));
  const standardLibrary = scope !== "web";
  const slugs = new Set(entries.map((entry) => entry.slug.toLowerCase()));
  const prefix = standardLibrary ? slugPart(scope.slice("skathi:".length)) : "interface";
  // SDK ambient declarations also contain helpers and optional globals. Only the registered
  // web interfaces have enough provenance for a new coverage entry without an authored guide.
  const pending = [...symbols].flatMap(([name, symbol]) => {
    if (covered.has(name)) return [];
    const nativeBinding = rustBinding(scope, name);
    return standardLibrary || nativeBinding ? [{ name, symbol, nativeBinding, typeOnly: api.typeOnly(symbol) }] : [];
  });
  // A public value has its own entry even when its instance type appears in another value's signature.
  for (const item of pending) if (!item.typeOnly) claimed.add(item.name);
  pending.sort((a, b) => Number(a.typeOnly) - Number(b.typeOnly));
  for (const { name, symbol, typeOnly, nativeBinding: spec } of pending) {
    if (covered.has(name)) continue;
    const { source, summary, syntax } = api.describe(symbol, true);
    const nativeBinding =
      !standardLibrary && spec
        ? { ...spec, members: spec.members.filter((member) => !api.syntax(symbols, `${name}.${member.name}`)) }
        : spec;
    const base = `${prefix}-${slugPart(name)}`;
    let slug = base;
    if (slugs.has(slug)) {
      const qualified = `${base}-${typeOnly ? "type" : "value"}`;
      slug = qualified;
      for (let suffix = 2; slugs.has(slug); suffix++) slug = `${qualified}-${suffix}`;
    }
    slugs.add(slug);
    const entry: RefEntry = {
      name,
      slug,
      category: standardLibrary ? scope : "Native interfaces",
      kind: typeOnly ? "type" : nativeBinding ? "interface" : "export",
      status: "declared",
      platforms: { windows: "declared", linux: "declared", macos: "declared" },
      summary,
      source,
      syntax: standardLibrary ? `import ${typeOnly ? "type " : ""}{ ${name} } from "${scope}";\n\n${syntax}` : syntax,
      ...(standardLibrary && { skathiOnly: true }),
      ...(nativeBinding && { nativeBinding }),
    };
    result.push(typeOnly ? entry : withTypes(entry, [name]));
  }
  return result;
}
