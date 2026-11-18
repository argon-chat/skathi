import type { APIRoute } from "astro";
import { blogDescription, blogFeed, blogHome, blogTitle, getPosts, postUrl } from "../../lib/blog";

const escape = (text: string): string =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export const GET: APIRoute = async ({ site }) => {
  if (!site) throw new Error("Set site in the Astro configuration to generate the feed's URLs.");
  const posts = await getPosts();
  const items = posts.map((post) => {
    const url = new URL(postUrl(post.id), site).href;
    return [
      "<item>",
      `<title>${escape(post.data.title)}</title>`,
      `<link>${url}</link>`,
      `<guid isPermaLink="true">${url}</guid>`,
      `<pubDate>${post.data.date.toUTCString()}</pubDate>`,
      `<dc:creator>${escape(post.data.author)}</dc:creator>`,
      `<description>${escape(post.data.description)}</description>`,
      ...post.data.tags.map((tag) => `<category>${escape(tag)}</category>`),
      "</item>",
    ].join("");
  });
  const latest = posts[0]?.data.date;
  const xml = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/elements/1.1/">',
    "<channel>",
    `<title>${escape(blogTitle)}</title>`,
    `<link>${new URL(blogHome, site).href}</link>`,
    `<description>${escape(blogDescription)}</description>`,
    "<language>en</language>",
    `<atom:link href="${new URL(blogFeed, site).href}" rel="self" type="application/rss+xml"/>`,
    ...(latest ? [`<lastBuildDate>${latest.toUTCString()}</lastBuildDate>`] : []),
    ...items,
    "</channel>",
    "</rss>",
  ].join("\n");
  return new Response(xml, { headers: { "Content-Type": "application/rss+xml; charset=utf-8" } });
};
