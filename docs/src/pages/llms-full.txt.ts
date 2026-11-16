import type { APIRoute } from "astro";
import { getDocs, pageMarkdown } from "../lib/docs";

export const GET: APIRoute = async ({ site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate documentation URLs.");
  return new Response((await getDocs()).map((page) => pageMarkdown(page, site)).join("\n\n---\n\n"), {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  });
};
