import { ALL_CODES, RANGES, RETIRED, type CodeInfo } from "../../../packages/cli/check/codes";

export type { CodeInfo };

export const checkPage = { id: "sk-check", title: "sk check", source: "packages/cli/check/codes.ts" } as const;

export interface CheckGroup {
  id: string;
  /** `SK1xxx`. */
  label: string;
  title: string;
  what: string;
  /** Where the docs explain the whole range. */
  more?: { label: string; href: string }[];
  codes: CodeInfo[];
}

const about: Record<number, Pick<CheckGroup, "title" | "more">> = {
  0: { title: "The check itself" },
  1000: {
    title: "Breaks under AOT",
    more: [{ label: "Built-ins are read-only", href: "/guides/aot/#built-ins-are-read-only" }],
  },
  2000: {
    title: "Slower under AOT",
    more: [
      { label: "Code made at run time", href: "/guides/aot/#code-made-at-run-time" },
      { label: "Tight loops", href: "/guides/aot/#tight-loops" },
    ],
  },
  3000: { title: "Typing", more: [{ label: "The AOT guide", href: "/guides/aot/" }] },
  4000: { title: "CSS", more: [{ label: "The CSS reference", href: "/css/" }] },
  5000: {
    title: "Vue and Svelte",
    more: [
      { label: "Vue support", href: "/guides/vue-support/" },
      { label: "Svelte support", href: "/guides/svelte-support/" },
    ],
  },
  6000: {
    title: "Permissions",
    more: [
      { label: "dlopen()", href: "/std/dlopen/" },
      { label: '"permissions" in sk.config.json', href: "/sk-config/#permissions" },
    ],
  },
};

const number = (code: string): number => Number(code.slice(2));

export const checkGroups: CheckGroup[] = RANGES.map((range) => {
  const label = `SK${range.from / 1000}xxx`;
  return {
    id: label.toLowerCase(),
    label,
    title: about[range.from]?.title ?? range.what,
    what: range.what,
    more: about[range.from]?.more ?? [],
    codes: ALL_CODES.filter((c) => number(c.code) >= range.from && number(c.code) <= range.to),
  };
}).filter((group) => group.codes.length > 0);

export const checkCodes: readonly CodeInfo[] = checkGroups.flatMap((group) => group.codes);

export const retiredCodes: { code: string; why: string }[] = [...RETIRED].map(([code, why]) => ({ code, why }));

export function groupOf(info: CodeInfo): CheckGroup {
  const group = checkGroups.find((g) => g.codes.includes(info));
  if (!group) throw new Error(`${info.code} is in no range`);
  return group;
}

export function codeUrl(code: string): string {
  return `/${checkPage.id}/${code.toLowerCase()}/`;
}

export function codeMarkdownUrl(code: string): string {
  return `/${checkPage.id}/${code.toLowerCase()}.md`;
}

/** Whether `check.allow` can hide it, and why not. */
export function allowance(info: CodeInfo): { allowed: boolean; note: string } {
  if (info.fatal) return { allowed: false, note: "It stops the check, with exit code 2, and cannot be allowed." };
  if (info.firm) return { allowed: false, note: "It is an error in the app's permissions and cannot be allowed." };
  return { allowed: true, note: "" };
}

const PROSE = new Set(["SK0001", "SK0006", "SK3001", "SK5013", "SK5016", "SK6005", "SK4005"]);

/** The language of a code's example; `text` for one that describes a situation. */
export function exampleLang(info: CodeInfo): string {
  if (PROSE.has(info.code) && !/^import /.test(info.example)) return "text";
  if (info.code === "SK0003") return "json";
  if (info.code === "SK0005") return "hlsl";
  if (info.code === "SK5015") return "svelte";
  if (info.example.startsWith("<")) return "vue";
  if (number(info.code) >= 4000 && number(info.code) < 5000 && !/^import /.test(info.example)) return "css";
  return "ts";
}

export function codeMarkdown(info: CodeInfo, site: URL): string {
  const group = groupOf(info);
  const allow = allowance(info);
  const lang = exampleLang(info);
  return [
    `# ${info.code}: ${info.title}`,
    "",
    `Source: ${new URL(codeUrl(info.code), site).href}`,
    "",
    `- Severity: ${info.severity}`,
    `- Range: ${group.label}, ${group.what}`,
    `- check.allow: ${allow.allowed ? "can hide it for a package or a glob" : allow.note}`,
    "",
    info.explanation,
    "",
    "## Example",
    "",
    `\`\`\`${lang === "text" ? "" : lang}`,
    info.example,
    "```",
    "",
    "## Fix",
    "",
    info.fix,
    "",
  ].join("\n");
}

export function checkMarkdown(site: URL): string {
  const lines: string[] = [];
  for (const group of checkGroups) {
    lines.push(`## ${group.label}: ${group.title}`, "", `${group.what}.`, "");
    for (const info of group.codes) {
      lines.push(
        `- [${info.code}](${new URL(codeMarkdownUrl(info.code), site).href}) (${info.severity}): ${info.title}`,
      );
    }
    lines.push("");
  }
  lines.push("## Retired codes", "");
  for (const { code, why } of retiredCodes) lines.push(`- ${code}: ${why}`);
  return lines.join("\n");
}
