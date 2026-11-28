import { declarations, signatureOf, type Declaration } from "./declarations";
import { supported, type Category, type Guide, type ShaderEntry } from "./shared";

// Exported for the generated entry point, outside the effect author's contract.
const wrapperSymbols = new Set([
  "SkEffectQuad",
  "sk_begin",
  "sk_finish",
  "sk_texture",
  "sk_content_mask_coverage",
  "sk_clamp_premultiplied",
]);

export function libraryEntries(files: Record<string, string>, guides: readonly Guide[]): ShaderEntry[] {
  const entries: ShaderEntry[] = [];
  for (const [file, source] of Object.entries(files)) {
    const modulePath = file.split("packages/cli/effects/").at(-1)!;
    const category = modulePath.replace(/\.slang$/, "").replaceAll("/", ".") as Category;
    const sourcePath = `packages/cli/effects/${modulePath}`;
    const symbols = new Map<string, Declaration[]>();
    const publicNames = new Set<string>();
    function collect(nodes: Declaration[], prefix = "", standalone = true) {
      for (const node of nodes) {
        if (category === "sk" && !prefix && (wrapperSymbols.has(node.name) || node.name.startsWith("SK_FLAG_")))
          continue;
        const name = prefix + node.name;
        const overloads = symbols.get(name) ?? [];
        overloads.push(node);
        symbols.set(name, overloads);
        if (standalone && node.kind !== "namespace") publicNames.add(name);
        collect(node.members, name + ".", node.kind === "namespace");
      }
    }
    collect(declarations(source));

    const moduleGuides = guides.filter((guide) => guide.category === category);
    const documented = new Set(
      moduleGuides
        .flatMap((guide) => [guide.name, ...(guide.aliases ?? [])])
        .flatMap((name) => symbols.get(name) ?? []),
    );
    function syntax(names: string[]) {
      const blocks: string[] = [];
      let previousScope = "";
      for (const name of names) {
        const scope = name.slice(0, Math.max(0, name.lastIndexOf(".")));
        if (scope && names.includes(scope)) continue;
        if (scope && scope !== previousScope) blocks.push(`// ${scope}`);
        previousScope = scope;
        const matches = symbols.get(name)!;
        for (const decl of matches) {
          const showDoc = (node: Declaration) =>
            node === decl ? names.length > 1 || matches.length > 1 : node.kind === "field" || !documented.has(node);
          blocks.push(signatureOf(decl, showDoc));
        }
      }
      return `import ${category};\n\n${blocks.join("\n")}`;
    }
    const claimed = new Set<string>();
    for (const guide of moduleGuides) {
      const primary = symbols.get(guide.name)?.[0];
      if (!primary) throw new Error(`${sourcePath}: missing declaration for /shaders/${guide.slug}/ (${guide.name})`);
      // Search aliases may also be CSS spellings.
      const names = [guide.name, ...(guide.aliases ?? [])].filter((name) => symbols.has(name));
      for (const name of names) claimed.add(name);
      entries.push({
        ...guide,
        kind: primary.kind,
        summary: guide.summary ?? primary.summary,
        syntax: syntax(names),
        source: sourcePath,
      });
    }
    for (const name of publicNames) {
      if (claimed.has(name)) continue;
      const overloads = symbols.get(name)!;
      const first = overloads[0]!;
      entries.push({
        ...supported,
        slug: `${category}.${name}`
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-")
          .replace(/-$/, ""),
        name,
        kind: first.kind,
        category,
        source: sourcePath,
        summary: first.summary || `Exported ${first.kind} in \`${category}\`.`,
        syntax: syntax([name]),
      });
    }
  }
  for (const guide of guides) {
    if (!entries.some((entry) => entry.slug === guide.slug))
      throw new Error(`Missing shader module for ${guide.category}.${guide.name}`);
  }
  if (new Set(entries.map((entry) => entry.slug)).size !== entries.length)
    throw new Error("Duplicate shader reference URL");
  return entries;
}
