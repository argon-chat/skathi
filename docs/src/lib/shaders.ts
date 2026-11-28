import { builtinEffects } from "../data/shaders/builtins";
import { categoryNotes, guides } from "../data/shaders/entries";
import { libraryEntries } from "../data/shaders/library";
import type { Reference } from "./reference";

const files = import.meta.glob<string>(
  ["../../../packages/cli/effects/sk.slang", "../../../packages/cli/effects/sk/*.slang"],
  { eager: true, query: "?raw", import: "default" },
);
const entries = [...libraryEntries(files, guides), ...builtinEffects];

export const shadersReference: Reference = {
  id: "shaders",
  title: "Shader library",
  noun: "attributes, types, functions and effects",
  categories: [...new Set([...guides.map((guide) => guide.category), ...entries.map((entry) => entry.category)])],
  categoryNotes,
  entries,
  source: "packages/cli/effects/sk.slang",
  syntax: "slang",
};
