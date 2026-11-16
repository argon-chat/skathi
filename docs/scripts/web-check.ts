// Consistency check for src/data/web/entries.ts. Usage: bun scripts/web-check.ts
import type { Status } from "../src/data/css/entries";
import { categories, entries, type WebEntry } from "../src/data/web/entries";

const errors: string[] = [];
const warnings: string[] = [];
const fail = (e: WebEntry, msg: string) => errors.push(`${e.slug}: ${msg}`);

const SLUG = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const RELEASED: readonly Status[] = ["supported", "partial"];
const slugs = new Set<string>();

for (const e of entries) {
  if (!SLUG.test(e.slug)) fail(e, "slug is not lowercase and url-safe");
  if (slugs.has(e.slug)) fail(e, "duplicate slug");
  slugs.add(e.slug);
}

for (const e of entries) {
  if (!(categories as readonly string[]).includes(e.category)) fail(e, `unknown category "${e.category}"`);
  if (e.summary.trim() === "" || !/[.!?]$/.test(e.summary)) fail(e, "summary must be one sentence");
  for (const r of e.related ?? []) {
    if (!slugs.has(r)) fail(e, `related slug "${r}" does not exist`);
    if (r === e.slug) fail(e, "relates to itself");
  }
  if (RELEASED.includes(e.status) !== (e.since !== undefined))
    fail(e, `since must be set exactly when status is supported or partial`);
  if (e.mdn !== undefined && !e.mdn.startsWith("https://developer.mozilla.org/")) fail(e, "mdn is not an MDN URL");
  if (e.skathiOnly && e.mdn !== undefined) fail(e, "a Skathi-only entry has an MDN link");

  const explained = (e.limitations?.length ?? 0) > 0 || (e.notes?.length ?? 0) > 0 || e.workaround !== undefined;
  const valueNotes = (e.values ?? []).some((v) => v.status !== "supported" && v.note !== undefined);
  if (e.status === "partial" && !explained && !valueNotes) fail(e, "partial without a note, limitation or value note");
  if (e.status === "on-hold" && !explained) fail(e, "on hold without saying what it waits for");
  if (e.status === "planned" && !explained)
    warnings.push(`${e.slug}: planned without a note (the coverage page has none)`);

  for (const v of e.values ?? []) {
    if (v.status === "partial" && v.note === undefined) fail(e, `value "${v.value}" is partial without a note`);
    if (v.since !== undefined && (!RELEASED.includes(v.status) || v.since === e.since))
      fail(e, `value "${v.value}" has a redundant or invalid since`);
  }
  const perPlatform = Object.values(e.platforms);
  for (const p of perPlatform) {
    if (RELEASED.includes(p) && !RELEASED.includes(e.status)) fail(e, "supported on a platform but not overall");
  }
  if (RELEASED.includes(e.status) && !perPlatform.some((p) => RELEASED.includes(p)))
    fail(e, "supported overall but on no platform");
  if (e.status === "partial" && !perPlatform.includes("partial") && new Set(perPlatform).size > 1)
    fail(e, "partial overall but on no platform");
  for (const s of [
    ...(e.notes ?? []),
    ...(e.limitations ?? []),
    ...(e.workaround === undefined ? [] : [e.workaround]),
  ]) {
    if (!/^[A-Z0-9`:@<"-]/.test(s) || !/[.)]$/.test(s)) fail(e, `not a full sentence: "${s}"`);
  }
}

const by = <K extends keyof WebEntry>(k: K) =>
  [...Map.groupBy(entries, (e) => String(e[k]))].map(([key, list]) => `${key} ${list.length}`).join(", ");
console.log(`${entries.length} entries`);
console.log(`by kind: ${by("kind")}`);
console.log(`by status: ${by("status")}`);
for (const w of warnings) console.warn(`warning: ${w}`);
if (errors.length > 0) {
  for (const e of errors) console.error(`error: ${e}`);
  process.exit(1);
}
