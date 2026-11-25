import type { APIRoute, GetStaticPaths } from "astro";
import { shadersReference } from "../../lib/shaders";
import { entryMarkdown, pages, type RefEntry } from "../../lib/reference";

export const getStaticPaths = (() =>
  pages(shadersReference).map((entry) => ({
    params: { slug: entry.slug },
    props: { entry },
  }))) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props, site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate Markdown URLs.");
  return new Response(entryMarkdown(shadersReference, props["entry"] as RefEntry, site), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
};
