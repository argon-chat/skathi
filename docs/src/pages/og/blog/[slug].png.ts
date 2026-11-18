import type { APIRoute, GetStaticPaths } from "astro";
import { formatDate, getPosts, type Post } from "../../../lib/blog";
import { renderCard } from "../../../lib/og";

export const getStaticPaths = (async () =>
  (await getPosts()).map((post) => ({ params: { slug: post.id }, props: { post } }))) satisfies GetStaticPaths;

export const GET: APIRoute = async ({ props }) => {
  const { title, metaTitle, description, date, tags, release } = (props["post"] as Post).data;
  const png = await renderCard({
    kicker: "Skathi blog",
    title: metaTitle ?? title,
    description,
    label: release ? `Release ${release.version}` : (tags[0] ?? "Blog"),
    meta: `${formatDate(date)} · skathi.land`,
  });
  return new Response(new Blob([new Uint8Array(png)], { type: "image/png" }));
};
