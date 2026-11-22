import type { APIRoute, GetStaticPaths } from "astro";
import { checkCodes, codeMarkdown, type CodeInfo } from "../../lib/check";

export const getStaticPaths = (() =>
  checkCodes.map((info) => ({
    params: { code: info.code.toLowerCase() },
    props: { info },
  }))) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props, site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate Markdown URLs.");
  return new Response(codeMarkdown(props["info"] as CodeInfo, site), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
};
