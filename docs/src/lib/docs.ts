import { getCollection, type CollectionEntry } from "astro:content";
import cli from "../../../packages/cli/package.json";
import { configMarkdown, configPage } from "./config";
import { catalogMarkdown } from "./reference";
import { references } from "./references";
import { expandSamples } from "./samples";

export const sections = ["Getting started", "Guides", "Reference", "Contributing"] as const;
export const repository = "https://github.com/argon-chat/skathi";
export const version = cli.version;
export const docsHome = "/introduction/";

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
  const body = expandSamples((page.body ?? "").replace(/^#\s+[^\n]*(?:\r?\n|$)/, "").trim());
  const reference = references[page.id];
  const catalog = reference
    ? `\n\n${catalogMarkdown(reference, site)}`
    : page.id === configPage.id
      ? `\n\n${configMarkdown(site)}`
      : "";
  return `# ${page.data.title}\n\n> ${page.data.description}\n\nSource: ${new URL(pageUrl(page.id), site).href}\n\n${body}${catalog}\n`;
}
