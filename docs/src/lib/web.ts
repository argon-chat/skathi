import { categories, categoryNotes, entries } from "../data/web/entries";
import { mergeDetails, type Details } from "../data/details";
import type { Reference } from "./reference";

export const webReference: Reference = {
  id: "web-platform",
  title: "Web platform coverage",
  noun: "APIs, interfaces, events",
  categories,
  categoryNotes,
  entries: mergeDetails(entries, import.meta.glob<{ details: Details }>("../data/web/details/*.ts", { eager: true })),
  source: "docs/src/data/web/entries.ts",
};
