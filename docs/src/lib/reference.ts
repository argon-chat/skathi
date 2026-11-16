import cli from "../../../packages/cli/package.json";
import type { Platform, Release, Status, ValueSupport } from "../data/css/entries";
import type { Detail, Example, Param } from "../data/details";

export type { Example, Param, Platform, Release, Status, ValueSupport };

export interface RefEntry extends Detail {
  slug: string;
  name: string;
  kind: string;
  category: string;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  experimental?: boolean;
  webDeprecated?: boolean;
  skathiOnly?: boolean;
  mdn?: string;
  values?: ValueSupport[];
  notes?: string[];
  limitations?: string[];
  workaround?: string;
  animates?: "interpolated" | "discrete";
  related?: string[];
  aliases?: string[];
}

export interface Reference {
  /** The collection page the entries belong under, also the URL segment. */
  id: string;
  title: string;
  noun: string;
  categories: readonly string[];
  categoryNotes: Readonly<Partial<Record<string, string>>>;
  entries: readonly RefEntry[];
  source: string;
}

/** The slim shape the catalog island receives. */
export type CatalogEntry = Pick<
  RefEntry,
  | "slug"
  | "name"
  | "kind"
  | "category"
  | "summary"
  | "status"
  | "platforms"
  | "since"
  | "experimental"
  | "skathiOnly"
  | "aliases"
> & { changed?: Release; split?: boolean };

export const currentVersion = cli.version;

export const platforms: readonly {
  id: Platform;
  label: string;
  short: string;
}[] = [
  { id: "windows", label: "Windows", short: "Win" },
  { id: "macos", label: "macOS", short: "mac" },
  { id: "linux", label: "Linux", short: "Linux" },
];

export const statuses: Record<Status, { label: string; tone: string; summary: string }> = {
  supported: {
    label: "Supported",
    tone: "ok",
    summary: "Works as in browsers",
  },
  partial: {
    label: "Partial",
    tone: "partial",
    summary: "Works with a limitation",
  },
  planned: {
    label: "Planned",
    tone: "planned",
    summary: "Not yet; on the roadmap",
  },
  "on-hold": {
    label: "On hold",
    tone: "hold",
    summary: "Waiting on something outside Skathi",
  },
  "not-planned": {
    label: "Not planned",
    tone: "no",
    summary: "Not supported, and not planned for now",
  },
  deprecated: {
    label: "Deprecated",
    tone: "gone",
    summary: "Will not be added, or will be removed",
  },
};

const kindLabels: Record<string, string> = {
  property: "Property",
  prop: "Component prop",
  option: "Config option",
  component: "Component",
  exposed: "Exposed member",
  shorthand: "Shorthand",
  selector: "Selector",
  "pseudo-class": "Pseudo-class",
  "pseudo-element": "Pseudo-element",
  "at-rule": "At-rule",
  function: "Function",
  "media-feature": "Media feature",
  unit: "Unit",
  feature: "Feature",
  interface: "Interface",
  method: "Method",
  event: "Event",
  element: "Element",
  api: "API",
};

export function kindLabel(kind: string): string {
  return kindLabels[kind] ?? kind.charAt(0).toUpperCase() + kind.slice(1);
}

export function categoryId(category: string): string {
  return category
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
}

export function entryUrl(ref: Reference, slug: string): string {
  return `/${ref.id}/${slug}/`;
}

export function entryMarkdownUrl(ref: Reference, slug: string): string {
  return `/${ref.id}/${slug}.md`;
}

export function ordered(ref: Reference): RefEntry[] {
  return ref.categories.flatMap((category) => ref.entries.filter((entry) => entry.category === category));
}

export function find(ref: Reference, slug: string): RefEntry | undefined {
  return ref.entries.find((entry) => entry.slug === slug);
}

export function platformsDiffer(entry: Pick<RefEntry, "platforms">): boolean {
  return new Set(Object.values(entry.platforms)).size > 1;
}

export function changedIn(entry: RefEntry): Release | undefined {
  const releases = [entry.since, ...(entry.values ?? []).map((value) => value.since)].filter(
    (release): release is Release => release !== undefined,
  );
  return releases.sort().at(-1);
}

export function catalogEntries(ref: Reference): CatalogEntry[] {
  return ordered(ref).map((entry) => {
    const slim: CatalogEntry = {
      slug: entry.slug,
      name: entry.name,
      kind: entry.kind,
      category: entry.category,
      summary: entry.summary,
      status: entry.status,
      platforms: entry.platforms,
    };
    if (entry.since) slim.since = entry.since;
    const changed = changedIn(entry);
    if (changed) slim.changed = changed;
    if (entry.experimental) slim.experimental = true;
    if (entry.skathiOnly) slim.skathiOnly = true;
    if (entry.aliases?.length) slim.aliases = entry.aliases;
    if (platformsDiffer(entry)) slim.split = true;
    return slim;
  });
}

/** A concept name ("Pointer Lock", "Class selector") rather than a token to set in mono. */
export function isProseName(name: string): boolean {
  return name.includes("`") || (/\s/.test(name) && !/^[<@:[]/.test(name));
}

export function plain(text: string): string {
  return text.replaceAll("`", "");
}

/** Inline Markdown code spans and links (site paths or https) to HTML, for the short prose fields. */
export function inline(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\[([^\]]+)\]\((\/[^)\s"]*|https:\/\/[^)\s"]+)\)/g, '<a href="$2">$1</a>');
}

function platformLine(entry: RefEntry): string {
  return platforms.map((p) => `${p.label}: ${statuses[entry.platforms[p.id]].label.toLowerCase()}`).join(", ");
}

export function entryMarkdown(ref: Reference, entry: RefEntry, site: URL): string {
  const lines = [
    `# ${entry.name}`,
    "",
    `> ${entry.summary}`,
    "",
    `Source: ${new URL(entryUrl(ref, entry.slug), site).href}`,
    "",
    `- Kind: ${kindLabel(entry.kind)} (${entry.category})`,
    `- Status in Skathi: ${statuses[entry.status].label}${entry.since ? ` since ${entry.since}` : ""}`,
    `- Platforms: ${platformLine(entry)}`,
  ];
  if (entry.skathiOnly) lines.push("- Skathi only: not part of the web platform; browsers ignore it.");
  if (entry.experimental) lines.push("- Experimental on the web.");
  if (entry.webDeprecated) lines.push("- Deprecated on the web.");
  if (entry.animates) lines.push(`- Animates: ${entry.animates}.`);
  if (entry.mdn) lines.push(`- MDN: ${entry.mdn}`);
  if (entry.syntax) {
    lines.push("", "## Syntax", "", "```ts", entry.syntax.trim(), "```");
    if (entry.params?.length)
      lines.push(
        "",
        "| Parameter | Type | Description |",
        "| --- | --- | --- |",
        ...entry.params.map(
          (p) =>
            `| \`${p.name}\`${p.optional ? " (optional)" : ""} | \`${p.type.replaceAll("|", "\\|")}\` | ${p.description}${p.default ? ` Default: \`${p.default}\`.` : ""} |`,
        ),
      );
    if (entry.returns) lines.push("", `Returns: ${entry.returns}`);
  }
  if (entry.notes?.length) lines.push("", "## Behavior", "", ...entry.notes.map((note) => `- ${note}`));
  if (entry.values?.length) {
    lines.push("", "## Values", "", "| Value | Status | Since | Notes |", "| --- | --- | --- | --- |");
    for (const value of entry.values)
      lines.push(
        `| \`${value.value}\` | ${statuses[value.status].label} | ${value.since ?? entry.since ?? ""} | ${value.note ?? ""} |`,
      );
  }
  if (entry.examples?.length) {
    lines.push("", "## Examples");
    for (const example of entry.examples) {
      lines.push("", `### ${example.title}`, "");
      if (example.note) lines.push(example.note, "");
      lines.push(`\`\`\`${example.lang}`, example.code.trim(), "```");
    }
  }
  if (entry.edgeCases?.length) lines.push("", "## Edge cases", "", ...entry.edgeCases.map((e) => `- ${e}`));
  if (entry.limitations?.length) lines.push("", "## Limitations", "", ...entry.limitations.map((l) => `- ${l}`));
  if (entry.workaround) lines.push("", "## Instead", "", entry.workaround);
  const related = (entry.related ?? []).map((slug) => find(ref, slug)).filter((r) => r !== undefined);
  if (related.length)
    lines.push(
      "",
      "## Related",
      "",
      ...related.map((r) => `- [${r.name}](${new URL(entryMarkdownUrl(ref, r.slug), site).href})`),
    );
  return `${lines.join("\n")}\n`;
}

export function catalogMarkdown(ref: Reference, site: URL): string {
  return ref.categories
    .map((category) => {
      const note = ref.categoryNotes[category];
      const rows = ref.entries
        .filter((entry) => entry.category === category)
        .map((entry) => {
          const where = platformsDiffer(entry)
            ? platforms
                .filter((p) => entry.platforms[p.id] === "supported" || entry.platforms[p.id] === "partial")
                .map((p) => p.label)
                .join(", ")
            : "all";
          const notes = [...(entry.limitations ?? []), ...(entry.workaround ? [entry.workaround] : [])].join(" ");
          return `| [\`${entry.name}\`](${new URL(entryMarkdownUrl(ref, entry.slug), site).href}) | ${statuses[entry.status].label} | ${where} | ${entry.since ?? ""} | ${notes} |`;
        });
      return [
        `## ${category}`,
        ...(note ? ["", note] : []),
        "",
        "| Name | Status | Platforms | Since | Notes |",
        "| --- | --- | --- | --- | --- |",
        ...rows,
      ].join("\n");
    })
    .join("\n\n");
}
