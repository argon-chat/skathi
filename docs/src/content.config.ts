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
    section: z.enum(["Getting started", "Guides", "Embedding", "Reference", "Contributing"]),
    order: z.number().default(0),
    label: z.string().optional(),
    status: z.enum(["experimental"]).optional(),
  }),
});

export const collections = { docs };
