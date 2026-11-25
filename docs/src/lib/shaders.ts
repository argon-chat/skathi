import { categories, categoryNotes, entries } from "../data/shaders/entries";
import type { Reference } from "./reference";

export const shadersReference: Reference = {
  id: "shaders",
  title: "Shader library",
  noun: "attributes, types, functions",
  categories,
  categoryNotes,
  entries,
  source: "docs/src/data/shaders/entries.ts",
  syntax: "slang",
};
