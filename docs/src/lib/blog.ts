import { getCollection, type CollectionEntry } from "astro:content";

export type Post = CollectionEntry<"blog">;

export const blogHome = "/blog/";
export const blogFeed = "/blog/rss.xml";
export const blogTitle = "Skathi blog";
export const blogDescription =
  "What changed in each Skathi release, what it measured, and what to do when you upgrade.";

/** Drafts are built by `astro dev` and by a build with SKATHI_DRAFTS=1, never by a plain production build. */
export const showDrafts = import.meta.env.DEV || process.env["SKATHI_DRAFTS"] === "1";

export async function getPosts(): Promise<Post[]> {
  return (await getCollection("blog", (post) => showDrafts || !post.data.draft)).sort(
    (a, b) => b.data.date.getTime() - a.data.date.getTime() || a.id.localeCompare(b.id),
  );
}

export function postUrl(id: string): string {
  return `${blogHome}${id}/`;
}

const dateFormat = new Intl.DateTimeFormat("en", { dateStyle: "long", timeZone: "UTC" });

export function formatDate(date: Date): string {
  return dateFormat.format(date);
}

export function isoDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}

/** Minutes at 230 words a minute, counting prose and code, not MDX imports or tags. */
export function readingMinutes(body: string): number {
  const words = body
    .replace(/^\s*(?:import|export)\s[^\n]*$/gm, "")
    .replace(/<\/?[A-Za-z][^>]*>/g, " ")
    .split(/\s+/)
    .filter((word) => /[\p{L}\p{N}]/u.test(word)).length;
  return Math.max(1, Math.round(words / 230));
}
