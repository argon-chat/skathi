import { defineCollection } from "astro:content";
import { z } from "astro/zod";
import { glob } from "astro/loaders";

const docs = defineCollection({
  loader: glob({
    base: ".",
    pattern: ["*.md", "!README.md", "pages/**/*.{md,mdx}"],
    generateId: ({ entry }) => entry.replace(/^pages\//, "").replace(/\.(md|mdx)$/, ""),
  }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    section: z.enum(["Getting started", "Guides", "Devtools", "Embedding", "Reference", "Contributing"]),
    order: z.number().default(0),
    label: z.string().optional(),
    status: z.enum(["experimental"]).optional(),
    /** The minor release the page arrived in, such as "0.4"; it is marked New while that release is current. */
    since: z
      .string()
      .regex(/^\d+\.\d+$/)
      .optional(),
  }),
});

const blog = defineCollection({
  loader: glob({ base: "./blog", pattern: "**/*.{md,mdx}" }),
  schema: ({ image }) =>
    z.object({
      title: z.string(),
      /** The summary on the index, in the feed and under the title. */
      description: z.string(),
      date: z.coerce.date(),
      author: z.string().default("The Skathi team"),
      tags: z.array(z.string()).default([]),
      /** Left out of production builds; see `showDrafts`. */
      draft: z.boolean().default(false),
      /** A release post: its version and the commands that install it, shown at the top. */
      release: z
        .object({
          version: z.string(),
          platforms: z.string().optional(),
          install: z.array(z.object({ label: z.string(), command: z.string() })).min(1),
        })
        .optional(),
      /** An image from src/assets, or a clip in public/media given as `<group>/<name>` like `Demo`. */
      hero: z
        .object({
          image: image().optional(),
          video: z.string().optional(),
          alt: z.string(),
          caption: z.string().optional(),
        })
        .refine((hero) => Boolean(hero.image) !== Boolean(hero.video), "A hero has an image or a video, not both.")
        .optional(),
    }),
});

export const collections = { docs, blog };
