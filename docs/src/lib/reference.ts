import type { Platform, PlatformSince, Release, Status, ValueSupport } from "../data/css/entries";
import type { Detail, Example, Param } from "../data/details";
import { isNew, newLabel } from "./release";

export type { Example, Param, Platform, PlatformSince, Release, Status, ValueSupport };
export { isNew, newLabel };

export interface RefEntry extends Detail {
  slug: string;
  name: string;
  kind: string;
  category: string;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  platformSince?: PlatformSince;
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

/** What a browser does with an entry only Skathi has: CSS is skipped there, a script fails. */
export function skathiOnlyNote(ref: Reference): string {
  if (ref.id === "css") return "Browsers ignore it; Skathi reads it.";
  if (ref.id === "std") return "A browser has no such module, and an import of it fails there.";
  return "A browser does not have it, and code that uses it fails there.";
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
  | "platformSince"
> & { changed?: Release; split?: boolean; fresh?: string };

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

export function changedIn(entry: Pick<RefEntry, "since" | "platformSince" | "values">): Release | undefined {
  const releases = [
    entry.since,
    ...Object.values(entry.platformSince ?? {}),
    ...(entry.values ?? []).map((value) => value.since),
  ].filter((release): release is Release => release !== undefined);
  return releases.sort().at(-1);
}

/** The release `platform` got the entry in; none where it is not supported. */
export function sinceOn(
  entry: Pick<RefEntry, "since" | "platformSince" | "platforms">,
  platform: Platform,
): Release | undefined {
  const status = entry.platforms[platform];
  if (status !== "supported" && status !== "partial") return undefined;
  return entry.platformSince?.[platform] ?? entry.since;
}

/** "0.2.0; macOS, Linux 0.3.0": the entry's release and the platforms that got it later. */
export function sinceText(entry: Pick<RefEntry, "since" | "platformSince" | "platforms">): string {
  if (!entry.since) return "";
  const later = new Map<Release, string[]>();
  for (const p of platforms) {
    const since = sinceOn(entry, p.id);
    if (since !== undefined && since !== entry.since) later.set(since, [...(later.get(since) ?? []), p.label]);
  }
  return [entry.since, ...[...later].map(([since, labels]) => `${labels.join(", ")} ${since}`)].join("; ");
}

/** What arrived in the current release, for the marker's tooltip; undefined when nothing did. */
export function freshNote(
  entry: Pick<RefEntry, "since" | "platformSince" | "platforms" | "values">,
): string | undefined {
  const changed = changedIn(entry);
  if (!isNew(changed)) return undefined;
  if (isNew(entry.since)) return `Arrived in ${entry.since}`;
  const parts: string[] = [];
  const os = platforms.filter((p) => isNew(sinceOn(entry, p.id))).map((p) => p.label);
  if (os.length) parts.push(`${os.join(" and ")} support`);
  const values = (entry.values ?? []).filter((v) => isNew(v.since)).map((v) => plain(v.value));
  if (values.length) parts.push(values.join(", "));
  return `${parts.join("; ")} arrived in ${changed}`;
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
    if (entry.platformSince) slim.platformSince = entry.platformSince;
    const changed = changedIn(entry);
    if (changed) slim.changed = changed;
    const fresh = freshNote(entry);
    if (fresh) slim.fresh = fresh;
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
  return platforms
    .map((p) => {
      const since = entry.platformSince ? sinceOn(entry, p.id) : undefined;
      return `${p.label}: ${statuses[entry.platforms[p.id]].label.toLowerCase()}${releaseQualifier(since)}`;
    })
    .join(", ");
}

export function releaseQualifier(since?: Release): string {
  if (since === "Unreleased") return " in development builds";
  return since ? ` since ${since}` : "";
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
    `- Status in Skathi: ${statuses[entry.status].label}${releaseQualifier(entry.since)}`,
    `- Platforms: ${platformLine(entry)}`,
  ];
  const fresh = freshNote(entry);
  if (fresh) lines.push(`- ${newLabel}: ${fresh}.`);
  if (entry.skathiOnly) lines.push(`- Skathi only: not part of the web platform. ${skathiOnlyNote(ref)}`);
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
          return `| [\`${entry.name}\`](${new URL(entryMarkdownUrl(ref, entry.slug), site).href}) | ${statuses[entry.status].label} | ${where} | ${sinceText(entry)} | ${notes} |`;
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
