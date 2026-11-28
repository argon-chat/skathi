import { getCollection, type CollectionEntry } from "astro:content";
import type { MarkdownHeading } from "astro";
import { checkMarkdown, checkPage } from "./check";
import { configMarkdown, configPage } from "./config";
import { catalogMarkdown, categoryId } from "./reference";
import { moduleOverview } from "./std";
import { references } from "./references";
import { isNew, newLabel } from "./release";
import { expandSamples } from "./samples";

export { isNew, newLabel, version } from "./release";

export const sections = [
  "Getting started",
  "Guides",
  "Elements",
  "Embedding",
  "Devtools",
  "Reference",
  "Contributing",
] as const;
export const repository = "https://github.com/argon-chat/skathi";
export const docsHome = "/introduction/";

/** A heading in the page's table of contents; `fresh` when its section arrived in the current release. */
export interface TocHeading extends MarkdownHeading {
  fresh?: boolean;
}

export type PageStatus = NonNullable<CollectionEntry<"docs">["data"]["status"]>;

export const statusLabels: Record<PageStatus, string> = { experimental: "Experimental" };

export function pageUrl(id: string): string {
  return `/${id}/`;
}

export function markdownUrl(id: string): string {
  return `/${id}.md`;
}

export async function getDocs(): Promise<CollectionEntry<"docs">[]> {
  return (await getCollection("docs")).sort(
    (a, b) =>
      sections.indexOf(a.data.section) - sections.indexOf(b.data.section) ||
      a.data.order - b.data.order ||
      a.data.title.localeCompare(b.data.title),
  );
}

export function pageMarkdown(page: CollectionEntry<"docs">, site: URL): string {
  let body = expandSamples((page.body ?? "").replace(/^#\s+[^\n]*(?:\r?\n|$)/, "").trim());
  if (page.id === "std") {
    body = body.replace("<StdModules />", () =>
      [
        "| Module | What it gives |",
        "| --- | --- |",
        ...moduleOverview.map(
          ([name, summary]) => `| [\`${name}\`](#${categoryId(name)}) | ${summary.replaceAll("|", "\\|")} |`,
        ),
      ].join("\n"),
    );
  }
  const reference = references[page.id];
  const catalog = reference
    ? `\n\n${catalogMarkdown(reference, site)}`
    : page.id === configPage.id
      ? `\n\n${configMarkdown(site)}`
      : page.id === checkPage.id
        ? `\n\n${checkMarkdown(site)}`
        : "";
  const status = page.data.status ? `\n\nStatus: ${statusLabels[page.data.status]}` : "";
  const fresh = isNew(page.data.since) ? `\n\n${newLabel}` : "";
  return `# ${page.data.title}\n\n> ${page.data.description}\n\nSource: ${new URL(pageUrl(page.id), site).href}${status}${fresh}\n\n${body}${catalog}\n`;
}
