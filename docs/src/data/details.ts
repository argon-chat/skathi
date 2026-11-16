export interface Example {
  title: string;
  lang: "vue" | "css" | "ts" | "html";
  code: string;
  note?: string;
}

export interface Detail {
  examples?: Example[];
  edgeCases?: string[];
}

export type Details = Readonly<Record<string, Detail>>;

export function mergeDetails<T extends { slug: string }>(
  entries: readonly T[],
  modules: Record<string, { details: Details }>,
): (T & Detail)[] {
  const all = new Map<string, Detail>();
  for (const [file, module] of Object.entries(modules))
    for (const [slug, detail] of Object.entries(module.details)) {
      if (all.has(slug)) throw new Error(`${file}: ${slug} has details in two files.`);
      all.set(slug, detail);
    }
  const slugs = new Set(entries.map((entry) => entry.slug));
  for (const slug of all.keys()) if (!slugs.has(slug)) throw new Error(`Details for unknown entry ${slug}.`);
  return entries.map((entry) => ({ ...entry, ...all.get(entry.slug) }));
}
