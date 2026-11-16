import type { APIRoute, GetStaticPaths } from "astro";
import { entryMarkdown, ordered, type RefEntry } from "../../lib/reference";
import { webReference } from "../../lib/web";

export const getStaticPaths = (() =>
  ordered(webReference).map((entry) => ({ params: { slug: entry.slug }, props: { entry } }))) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props, site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate Markdown URLs.");
  return new Response(entryMarkdown(webReference, props["entry"] as RefEntry, site), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
};
