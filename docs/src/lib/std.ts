import { categories, categoryNotes, entries } from "../data/std/entries";
import type { Reference } from "./reference";

export const stdReference: Reference = {
  id: "std",
  title: "Skathi modules",
  noun: "classes, namespaces, functions",
  categories,
  categoryNotes,
  entries,
  source: "docs/src/data/std/entries.ts",
};
