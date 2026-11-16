import type { Platform, Release, Status, ValueSupport } from "../css/entries";
import type { Detail } from "../details";

export type Kind = "property" | "method" | "event" | "interface";

export const categories = [
  "Tray",
  "Global hotkeys",
  "Storage",
  "Shell",
  "Permissions",
  "Screen capture",
  "Locale",
  "Launch",
  "Embedding",
  "Devtools",
] as const;

export type Category = (typeof categories)[number];

export interface AppEntry extends Detail {
  slug: string;
  name: string;
  kind: Kind;
  category: Category;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  skathiOnly?: boolean;
  values?: ValueSupport[];
  notes?: string[];
  limitations?: string[];
  workaround?: string;
  related?: string[];
  aliases?: string[];
}

export const OK: Status = "supported";
export const PART: Status = "partial";
export const PLAN: Status = "planned";
export const NO: Status = "not-planned";

const FIRST_RELEASE: Release = "0.2.0";

export const all = (status: Status): Record<Platform, Status> => ({ windows: status, macos: status, linux: status });
export const os = (windows: Status, macos: Status, linux: Status): Record<Platform, Status> => ({
  windows,
  macos,
  linux,
});

function overall(p: Record<Platform, Status>): Status {
  const s = [p.windows, p.macos, p.linux];
  if (s.every((x) => x === p.windows)) return p.windows;
  if (s.some((x) => x === OK || x === PART)) return PART;
  return s.includes(PLAN) ? PLAN : NO;
}

export type Draft = Omit<AppEntry, "category" | "status" | "since" | "skathiOnly">;

export function section(category: Category, drafts: readonly Draft[]): AppEntry[] {
  return drafts.map((d) => {
    const status = overall(d.platforms);
    const e: AppEntry = { ...d, category, status, skathiOnly: true };
    if (status === OK || status === PART) e.since = FIRST_RELEASE;
    return e;
  });
}

export const RUNTIME = "@skathi-native/runtime";
export const importLine = (...names: string[]): string => `import { ${names.join(", ")} } from "${RUNTIME}";`;
