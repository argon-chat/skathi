import { dedent, parseLines, splitRegions, type Range } from "./code";

const files = import.meta.glob<string>("/samples/**/*.{vue,svelte,ts,json,css}", {
  query: "?raw",
  import: "default",
  eager: true,
});

export const frameworks = { vue: "Vue", svelte: "Svelte" } as const;
export type Framework = keyof typeof frameworks;

export interface SampleProps {
  /** Path under docs/samples, without the extension to show every framework variant beside it. */
  src: string;
  /** A `#region name` of the file to show instead of the whole file. */
  region?: string;
  /** Lines or region names to focus; the rest blur until the pointer is over the block. */
  focus?: string;
  /** Lines or region names to highlight. */
  highlight?: string;
  title?: string;
}

export interface SampleVariant {
  lang: string;
  framework: Framework | undefined;
  title: string;
  code: string;
  focus: Set<number>;
  highlight: Set<number>;
}

const frameworkOf = (path: string): Framework | undefined => {
  const ext = /\.(\w+)(?:\.ts)?$/.exec(path)?.[1];
  return ext !== undefined && ext in frameworks ? (ext as Framework) : undefined;
};

export function sampleVariants({ src, region, focus, highlight, title }: SampleProps): SampleVariant[] {
  const candidates = /\.\w+$/.test(src)
    ? [src]
    : Object.keys(frameworks).flatMap((ext) => [`${src}.${ext}`, `${src}.${ext}.ts`]);
  const variants = candidates
    .filter((path) => files[`/samples/${path}`] !== undefined)
    .map((path) => {
      const lang = path.slice(path.lastIndexOf(".") + 1);
      const { lines, regions } = splitRegions(files[`/samples/${path}`] ?? "");
      let shown = lines;
      let local: Map<string, Range> = regions;
      if (region !== undefined) {
        const range = regions.get(region);
        if (!range) throw new Error(`docs/samples/${path} has no region "${region}".`);
        shown = dedent(lines.slice(range.start, range.end));
        local = new Map(
          [...regions]
            .map(([name, r]): [string, Range] => [
              name,
              { start: Math.max(r.start, range.start) - range.start, end: Math.min(r.end, range.end) - range.start },
            ])
            .filter(([, r]) => r.end > r.start),
        );
      }
      return {
        lang,
        framework: frameworkOf(path),
        title: title ?? path.slice(path.lastIndexOf("/") + 1).replace(/\.vue\.ts$/, ".ts"),
        code: shown.join("\n"),
        focus: parseLines(focus, local),
        highlight: parseLines(highlight, local),
      };
    });
  if (!variants.length) throw new Error(`No sample at docs/samples/${src}.`);
  return variants;
}

/** Replaces each `<Sample … />` in an MDX body with its code as fenced blocks, for the Markdown copies of a page. */
export function expandSamples(body: string): string {
  return body
    .split(/(^```[\s\S]*?^```)/m)
    .map((part) => (part.startsWith("```") ? part : expandTags(part)))
    .join("");
}

function expandTags(text: string): string {
  return text.replace(/<Sample\s([^>]*?)\/>/gs, (_tag, attrs: string) => {
    const props: Record<string, string> = {};
    for (const [, key, value] of attrs.matchAll(/(\w+)="([^"]*)"/g)) if (key) props[key] = value ?? "";
    if (!props["src"]) return _tag;
    return sampleVariants(props as unknown as SampleProps)
      .map((v) => `\`\`\`${v.lang} title="${v.title}"\n${v.code}\n\`\`\``)
      .join("\n\n");
  });
}
