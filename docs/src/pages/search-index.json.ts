import type { APIRoute, MarkdownHeading } from "astro";
import { render } from "astro:content";
import { getPosts, postUrl } from "../lib/blog";
import { configPage, configSections, optionId, type ConfigOption } from "../lib/config";
import { getDocs, pageUrl, sections, statusLabels } from "../lib/docs";
import { entryUrl, kindLabel, ordered, plain, statuses } from "../lib/reference";
import { references } from "../lib/references";
import type { SearchData, SearchRecord } from "../lib/search";

const apiGroup = "API";
const groupOf: Readonly<Record<string, string>> = {
  "app-api": apiGroup,
  "window-api": apiGroup,
  "web-platform": "Web platform",
  css: "CSS",
};
const blogGroup = "Blog";
const groups = [...sections, apiGroup, "Web platform", "CSS", "sk.config.json", blogGroup];

/** Markdown prose as plain text: no code blocks, MDX imports or tags, link targets or emphasis marks. */
function plainText(markdown: string): string {
  return markdown
    .replace(/^\s*(?:import|export)\s[^\n]*$/gm, "")
    .replace(/^\s*\[[^\]]+\]:\s*\S+.*$/gm, "")
    .replace(/^\s{0,3}(?:>\s?)+/gm, "")
    .replace(/^\s*(?:[-*+]|\d+\.)\s+/gm, "")
    .replace(/^\s*\|?\s*:?-{3,}.*$/gm, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\[[^\]]*\]/g, "$1")
    .split("`")
    .map((part, index) =>
      index % 2
        ? part
        : part
            .replace(
              /<\/?(?:[A-Z][\w.]*|div|span|br|p|a|img|details|summary|kbd|sup|sub|em|strong|code|figure)\b[^>]*>/g,
              " ",
            )
            .replace(/\|/g, " ")
            .replace(/\*\*|__|\*|(?<![\p{L}\p{N}])_|_(?![\p{L}\p{N}])/gu, "")
            .replace(/&nbsp;/g, " ")
            .replace(/&lt;/g, "<")
            .replace(/&gt;/g, ">")
            .replace(/&amp;/g, "&"),
    )
    .join("")
    .replace(/\s+/g, " ")
    .trim();
}

interface Section {
  depth: number;
  heading: string;
  lines: string[];
}

function splitSections(markdown: string): Section[] {
  const out: Section[] = [{ depth: 1, heading: "", lines: [] }];
  let fence: string | undefined;
  for (const line of markdown.split(/\r?\n/)) {
    const marker = /^\s*(`{3,}|~{3,})/.exec(line)?.[1];
    if (marker && (!fence || marker.startsWith(fence))) {
      fence = fence ? undefined : marker;
      continue;
    }
    if (fence) continue;
    const heading = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (heading?.[1] && heading[1].length > 1)
      out.push({ depth: heading[1].length, heading: heading[2] ?? "", lines: [] });
    else if (!heading) out.at(-1)?.lines.push(line);
  }
  return out;
}

interface Page {
  group: number;
  title: string;
  crumb: string;
  url: string;
  description: string;
  status: string;
  body: string;
  headings: MarkdownHeading[];
}

/** A record for the page's lead and one per heading down to level four. */
function pageRecords(page: Page): SearchRecord[] {
  const records: SearchRecord[] = [];
  const rendered = page.headings.filter((heading) => heading.depth > 1);
  let next = 0;
  const parents: string[] = [];
  for (const section of splitSections(page.body)) {
    const text = plainText(section.lines.join("\n"));
    if (section.depth === 1) {
      records.push([page.group, page.title, page.crumb, page.url, page.description, text, "", "", page.status]);
      continue;
    }
    const index = rendered.findIndex((heading, i) => i >= next && heading.depth === section.depth);
    const match = rendered[index];
    if (!match) continue;
    next = index + 1;
    parents[section.depth] = match.text;
    parents.length = section.depth + 1;
    const trail = [page.title, ...parents.slice(2, section.depth)].filter(Boolean);
    if (section.depth > 4) continue;
    records.push([
      page.group,
      match.text,
      trail.join(" › "),
      `${page.url}#${match.slug}`,
      "",
      text,
      "",
      "",
      page.status,
    ]);
  }
  return records;
}

async function docRecords(): Promise<SearchRecord[]> {
  const records: SearchRecord[] = [];
  for (const page of await getDocs()) {
    records.push(
      ...pageRecords({
        group: sections.indexOf(page.data.section),
        title: page.data.title,
        crumb: page.data.section,
        url: pageUrl(page.id),
        description: page.data.description,
        status: page.data.status ? statusLabels[page.data.status] : "",
        body: page.body ?? "",
        headings: (await render(page)).headings,
      }),
    );
  }
  return records;
}

async function postRecords(): Promise<SearchRecord[]> {
  const records: SearchRecord[] = [];
  for (const post of await getPosts()) {
    records.push(
      ...pageRecords({
        group: groups.indexOf(blogGroup),
        title: post.data.title,
        crumb: blogGroup,
        url: postUrl(post.id),
        description: post.data.description,
        status: post.data.draft ? "Draft" : "",
        body: post.body ?? "",
        headings: (await render(post)).headings,
      }),
    );
  }
  return records;
}

function referenceRecords(): SearchRecord[] {
  return Object.values(references).flatMap((reference) => {
    const group = groups.indexOf(groupOf[reference.id] ?? apiGroup);
    return ordered(reference).map((entry): SearchRecord => {
      const body = [
        ...(entry.values ?? []).map((value) => value.value),
        ...(entry.params ?? []).map((param) => param.name),
        ...(entry.notes ?? []),
        ...(entry.limitations ?? []),
      ]
        .map(plainText)
        .join(" ");
      return [
        group,
        plain(entry.name),
        `${reference.title} › ${entry.category}`,
        entryUrl(reference, entry.slug),
        plainText(entry.summary),
        body,
        (entry.aliases ?? []).join("\n"),
        kindLabel(entry.kind),
        entry.status === "supported" ? "" : statuses[entry.status].label,
      ];
    });
  });
}

function configRecords(): SearchRecord[] {
  const group = groups.indexOf("sk.config.json");
  const walk = (option: ConfigOption, section: string): SearchRecord[] => [
    [
      group,
      option.path,
      `sk.config.json › ${section}`,
      `${pageUrl(configPage.id)}#${optionId(option.path)}`,
      plainText(option.description),
      [...option.values.map((value) => value.value), ...option.notes, ...option.limitations].map(plainText).join(" "),
      "",
      option.type || "Option",
      "",
    ],
    ...option.children.flatMap((child) => walk(child, section)),
  ];
  return configSections.flatMap((section) => section.options.flatMap((option) => walk(option, section.title)));
}

export const GET: APIRoute = async () => {
  const data: SearchData = {
    groups,
    records: [...(await docRecords()), ...referenceRecords(), ...configRecords(), ...(await postRecords())],
  };
  return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json; charset=utf-8" } });
};
