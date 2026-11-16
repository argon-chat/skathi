import type { APIRoute, GetStaticPaths } from "astro";
import type { CollectionEntry } from "astro:content";
import { getDocs, pageMarkdown } from "../lib/docs";

export const getStaticPaths = (async () =>
  (await getDocs()).map((page) => ({ params: { slug: page.id }, props: { page } }))) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props, site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate Markdown URLs.");
  const page = props["page"] as CollectionEntry<"docs">;
  return new Response(pageMarkdown(page, site), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
};
