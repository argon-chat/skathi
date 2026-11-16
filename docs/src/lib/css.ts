import { categories, categoryNotes, entries } from "../data/css/entries";
import { mergeDetails, type Details } from "../data/details";
import type { Reference } from "./reference";

export const cssReference: Reference = {
  id: "css",
  title: "CSS coverage",
  noun: "properties, selectors, at-rules",
  categories,
  categoryNotes,
  entries: mergeDetails(entries, import.meta.glob<{ details: Details }>("../data/css/details/*.ts", { eager: true })),
  source: "docs/src/data/css/entries.ts",
};
