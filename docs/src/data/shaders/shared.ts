import type { Platform, Release, Status } from "../css/entries";
import type { Detail } from "../details";
import type { Declaration } from "./declarations";

type Kind = Declaration["kind"] | "effect";
export type Category = "sk" | `sk.${string}` | "Built-in effects";

export interface ShaderEntry extends Detail {
  slug: string;
  name: string;
  kind: Kind;
  category: Category;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  skathiOnly?: boolean;
  notes?: string[];
  related?: string[];
  aliases?: string[];
  source?: string;
  syntaxLanguage?: "css";
}

/** `since` left out means the first release with effects. */
export type Guide = Omit<ShaderEntry, "summary" | "syntax" | "kind"> & { summary?: string };
export type Draft = Omit<Guide, "category" | "status" | "skathiOnly" | "platforms" | "since"> & {
  since?: Release;
  usage?: string;
};

const FIRST_RELEASE: Release = "0.2.0";
export const supported = {
  status: "supported",
  platforms: { windows: "supported", macos: "supported", linux: "supported" },
  skathiOnly: true,
} as const;

export function section(category: Category, drafts: readonly Draft[]): Guide[] {
  return drafts.map(({ usage, ...d }) => ({
    ...d,
    category,
    ...supported,
    since: d.since ?? FIRST_RELEASE,
    ...(usage ? { examples: [{ title: "Usage", lang: "slang" as const, code: usage }, ...(d.examples ?? [])] } : {}),
  }));
}
