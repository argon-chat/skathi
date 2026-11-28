import type { Platform, PlatformSince, Release, Status, ValueSupport } from "../css/entries";
import type { Detail } from "../details";
import { bindings } from "../../../scripts/bindings-reference";

export type Kind = "class" | "namespace" | "object" | "function" | "type";

export type Category = `skathi:${string}`;
export const categories = [...bindings().std.keys()].sort() as Category[];

export interface StdEntry extends Detail {
  slug: string;
  name: string;
  kind: Kind;
  category: Category;
  summary: string;
  /** Use the public declaration(s) for the syntax block. True resolves the entry's own name. */
  declaration?: true | string[];
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  platformSince?: PlatformSince;
  skathiOnly?: boolean;
  values?: ValueSupport[];
  notes?: string[];
  limitations?: string[];
  workaround?: string;
  related?: string[];
  aliases?: string[];
  /** The entry's page lives in another reference; the row links there and gets no page here. */
  link?: { url: string; title: string };
}

export const OK: Status = "supported";
export const PART: Status = "partial";
export const PLAN: Status = "planned";
export const NO: Status = "not-planned";

const FIRST_RELEASE: Release = "0.7.0";

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

/** `platforms` left out means every platform. */
export type Draft = Omit<StdEntry, "category" | "status" | "skathiOnly" | "platforms"> & {
  platforms?: Record<Platform, Status>;
};

export function section(category: Category, drafts: readonly Draft[]): StdEntry[] {
  return drafts.map((d) => {
    const platforms = d.platforms ?? all(OK);
    const status = overall(platforms);
    const e: StdEntry = { ...d, platforms, category, status, skathiOnly: true };
    if (status === OK || status === PART) e.since = d.since ?? FIRST_RELEASE;
    return e;
  });
}

export const importLine = (module: Category, ...names: string[]): string =>
  `import { ${names.join(", ")} } from "${module}";`;
