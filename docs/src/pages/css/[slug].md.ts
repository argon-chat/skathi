import type { APIRoute, GetStaticPaths } from "astro";
import { cssReference } from "../../lib/css";
import { entryMarkdown, ordered, type RefEntry } from "../../lib/reference";

export const getStaticPaths = (() =>
  ordered(cssReference).map((entry) => ({ params: { slug: entry.slug }, props: { entry } }))) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props, site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate Markdown URLs.");
  return new Response(entryMarkdown(cssReference, props["entry"] as RefEntry, site), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
};
