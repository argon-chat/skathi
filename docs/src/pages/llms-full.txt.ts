import type { APIRoute } from "astro";
import { getDocs, pageMarkdown } from "../lib/docs";
import { entryMarkdown, pages } from "../lib/reference";
import { references } from "../lib/references";

export const GET: APIRoute = async ({ site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate documentation URLs.");
  const content = (await getDocs()).map((page) => pageMarkdown(page, site));
  for (const reference of Object.values(references))
    for (const entry of pages(reference)) content.push(entryMarkdown(reference, entry, site));
  return new Response(content.join("\n\n---\n\n"), {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  });
};
