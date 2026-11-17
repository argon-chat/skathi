import type { APIRoute } from "astro";
import { render } from "astro:content";
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
const groups = [...sections, apiGroup, "Web platform", "CSS", "sk.config.json"];

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

async function docRecords(): Promise<SearchRecord[]> {
  const records: SearchRecord[] = [];
  for (const page of await getDocs()) {
    const { headings } = await render(page);
    const url = pageUrl(page.id);
    const group = sections.indexOf(page.data.section);
    const status = page.data.status ? statusLabels[page.data.status] : "";
    const rendered = headings.filter((heading) => heading.depth > 1);
    let next = 0;
    const parents: string[] = [];
    for (const section of splitSections(page.body ?? "")) {
      const text = plainText(section.lines.join("\n"));
      if (section.depth === 1) {
        records.push([group, page.data.title, page.data.section, url, page.data.description, text, "", "", status]);
        continue;
      }
      const index = rendered.findIndex((heading, i) => i >= next && heading.depth === section.depth);
      const match = rendered[index];
      if (!match) continue;
      next = index + 1;
      parents[section.depth] = match.text;
      parents.length = section.depth + 1;
      const trail = [page.data.title, ...parents.slice(2, section.depth)].filter(Boolean);
      if (section.depth > 4) continue;
      records.push([group, match.text, trail.join(" › "), `${url}#${match.slug}`, "", text, "", "", status]);
    }
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
  const data: SearchData = { groups, records: [...(await docRecords()), ...referenceRecords(), ...configRecords()] };
  return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json; charset=utf-8" } });
};
