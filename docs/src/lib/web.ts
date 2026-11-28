import { categories, categoryNotes, entries } from "../data/web/entries";
import { mergeDetails, type Details } from "../data/details";
import type { Reference } from "./reference";
import { bindings } from "../../scripts/bindings-reference";
import { rustBinding } from "../../scripts/rust-bindings";
import { apiEntries } from "./api-entries";

const api = bindings();

export const webReference: Reference = {
  id: "web-platform",
  title: "Web platform coverage",
  noun: "APIs, interfaces, events",
  categories: [...categories, "Native interfaces"],
  categoryNotes,
  entries: apiEntries(
    "web",
    api.web,
    mergeDetails(entries, import.meta.glob<{ details: Details }>("../data/web/details/*.ts", { eager: true })).map(
      (entry) => {
        if (entry.status !== "supported" && entry.status !== "partial") return entry;
        const syntax = api.syntax(api.web, entry.name);
        const root = entry.name.replace(/\(\)$/, "").split(".")[0]!;
        const symbol = api.web.get(root);
        const spec = rustBinding("web", root);
        const nativeBinding =
          spec && root === entry.name
            ? { ...spec, members: spec.members.filter((m) => !api.syntax(api.web, `${root}.${m.name}`)) }
            : undefined;
        return {
          ...entry,
          ...(syntax && symbol && { syntax, source: api.describe(symbol).source }),
          ...(nativeBinding && { nativeBinding }),
        };
      },
    ),
  ),
  source: "docs/src/data/web/entries.ts",
};
