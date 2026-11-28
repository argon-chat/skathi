import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { browserCompatData, isRecord, type BcdData, type CompatInfo } from "../src/data/browser-compat";

/** With an optional BCD data.json or package folder, audit a prospective version before changing the pin. */
export function checkCompat<T extends { slug: string; skathiOnly?: boolean }>(
  area: "css" | "web",
  entries: readonly T[],
  overrides: Readonly<Record<string, string | null>>,
  lookup: (entry: T, data: BcdData) => CompatInfo | undefined,
  allowMissing: (entry: T) => boolean = () => false,
) {
  const args = process.argv.slice(2);
  const source = args.find((arg) => !arg.startsWith("--"));
  let data = browserCompatData;
  if (source) {
    const file = statSync(source).isDirectory() ? join(source, "data.json") : source;
    const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
    if (!isRecord(parsed)) throw new Error(`${file} is not BCD data`);
    data = parsed;
  }
  const errors: string[] = [];
  const found: Array<[string, CompatInfo]> = [];
  const missing: string[] = [];
  for (const entry of entries) {
    const { slug } = entry;
    const hit = lookup(entry, data);
    if (hit) found.push([slug, hit]);
    else {
      if (!entry.skathiOnly) missing.push(slug);
      if (typeof overrides[slug] === "string") errors.push(`${slug}: override ${overrides[slug]} is missing from BCD`);
      else if (!entry.skathiOnly && !(slug in overrides) && !allowMissing(entry))
        errors.push(`${slug}: add a BCD key or null to src/data/${area}/bcd.ts`);
    }
    if (source && args.includes("--check") && JSON.stringify(hit) !== JSON.stringify(lookup(entry, browserCompatData)))
      errors.push(`${slug}: metadata differs from the pinned BCD package`);
  }
  const slugs = new Set(entries.map((entry) => entry.slug));
  for (const slug of Object.keys(overrides)) if (!slugs.has(slug)) errors.push(`${slug}: override has no entry`);
  const meta = data["__meta"];
  console.log(
    `${area}: BCD ${isRecord(meta) ? meta["version"] : "unknown"}, ${found.length} of ${entries.length} entries matched`,
  );
  console.log(`not in BCD: ${missing.join(", ")}`);
  for (const flag of ["experimental", "deprecated"] as const)
    console.log(
      `${flag}: ${found
        .filter(([, info]) => info[flag])
        .map(([slug]) => slug)
        .join(", ")}`,
    );
  if (errors.length) throw new Error(errors.join("\n"));
}
