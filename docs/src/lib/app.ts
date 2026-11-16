import { categories, categoryNotes, entries } from "../data/app/entries";
import type { Reference } from "./reference";

export const appReference: Reference = {
  id: "app-api",
  title: "App APIs",
  noun: "methods, properties, events, types",
  categories,
  categoryNotes,
  entries,
  source: "docs/src/data/app/entries.ts",
};
