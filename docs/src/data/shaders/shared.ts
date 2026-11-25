import type { Platform, Release, Status } from "../css/entries";
import type { Detail } from "../details";

export type Kind = "attribute" | "struct" | "interface" | "method" | "function" | "field" | "enum" | "type";

export const categories = ["sk", "sk.color"] as const;

export type Category = (typeof categories)[number];

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
}

/** `since` left out means the first release with effects. */
export type Draft = Omit<ShaderEntry, "category" | "status" | "skathiOnly" | "platforms" | "since"> & {
  since?: Release;
};

const FIRST_RELEASE: Release = "0.2.0";

export function section(category: Category, drafts: readonly Draft[]): ShaderEntry[] {
  return drafts.map((d) => ({
    ...d,
    category,
    status: "supported",
    platforms: { windows: "supported", macos: "supported", linux: "supported" },
    since: d.since ?? FIRST_RELEASE,
    skathiOnly: true,
  }));
}
