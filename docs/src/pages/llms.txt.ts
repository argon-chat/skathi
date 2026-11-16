import type { APIRoute } from "astro";
import { getDocs, markdownUrl, sections } from "../lib/docs";

export const GET: APIRoute = async ({ site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate documentation URLs.");
  const pages = await getDocs();
  const groups = sections.map(
    (section) =>
      `## ${section}\n\n${pages
        .filter((page) => page.data.section === section)
        .map((page) => `- [${page.data.title}](${new URL(markdownUrl(page.id), site)}): ${page.data.description}`)
        .join("\n")}`,
  );
  return new Response(
    [
      "# Skathi",
      "> Build native desktop applications with Vue, TypeScript, a GPU renderer, and JavaScriptCore.",
      "Start an app with `bun create skathi my-app`. The packages are @skathi-native/runtime and @skathi-native/cli (the sk command).",
      `Full documentation: ${new URL("/llms-full.txt", site)}`,
      ...groups,
    ].join("\n\n"),
    { headers: { "Content-Type": "text/plain; charset=utf-8" } },
  );
};
