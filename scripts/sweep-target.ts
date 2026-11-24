// Removes what cargo leaves behind in target/ and never deletes itself.
//
//   bun scripts/sweep-target.ts [--days 3] [--deps-days 14] [--target <dir>] [--dry-run]
//
// Every feature set, profile, test target and crate version gets an incremental cache and a set
// of artifacts of its own, and the ones no build uses any more stay for good. An incremental
// cache is swept when no build has written to it for --days; an artifact (deps, examples, build
// script output, fingerprints) when it is older than --deps-days. Cargo rebuilds whatever a later
// build still needs.

import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import { parseArgs } from "node:util";
import { ROOT } from "./package-crates";

const { values } = parseArgs({
  options: {
    days: { type: "string", default: "3" },
    "deps-days": { type: "string", default: "14" },
    target: { type: "string", default: join(ROOT, "target") },
    "dry-run": { type: "boolean", default: false },
  },
});
const DAY = 86_400_000;
const now = Date.now();
const cacheAge = Number(values.days) * DAY;
const artifactAge = Number(values["deps-days"]) * DAY;
if (!(cacheAge >= 0) || !(artifactAge >= 0)) throw new Error("--days and --deps-days take a number of days");
const target = values.target;
if (!existsSync(join(target, "CACHEDIR.TAG"))) throw new Error(`${target} is not a cargo target directory`);

const dirs = (path: string) =>
  readdirSync(path, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => join(path, e.name));

/** The last time a build wrote into `dir`: cargo replaces a session directory on every compile. */
function touched(dir: string): number {
  let latest = statSync(dir).mtimeMs;
  for (const e of readdirSync(dir)) latest = Math.max(latest, statSync(join(dir, e)).mtimeMs);
  return latest;
}

let freed = 0;
let removed = 0;
function drop(path: string, bytes: number) {
  freed += bytes;
  removed++;
  if (!values["dry-run"]) rmSync(path, { recursive: true, force: true });
}

/** Cargo puts host and cross-target profiles one or two levels below the cache root. */
function profiles(): string[] {
  const isProfile = (p: string) => existsSync(join(p, "deps")) || existsSync(join(p, ".cargo-build-lock"));
  const out: string[] = [];
  for (const a of dirs(target)) {
    if (isProfile(a)) out.push(a);
    else for (const b of dirs(a)) if (isProfile(b)) out.push(b);
  }
  return out;
}

function sweepUnit(unit: string, maxAge: number) {
  if (now - touched(unit) <= maxAge) return;
  let bytes = 0;
  for (const entry of readdirSync(unit, { withFileTypes: true, recursive: true })) {
    if (!entry.isFile() && !entry.isDirectory()) continue;
    const stat = statSync(join(entry.parentPath, entry.name));
    if (now - stat.mtimeMs <= maxAge) return;
    if (entry.isFile()) bytes += stat.size;
  }
  drop(unit, bytes);
}

for (const profile of profiles()) {
  const incremental = join(profile, "incremental");
  if (existsSync(incremental)) {
    for (const dir of dirs(incremental)) sweepUnit(dir, cacheAge);
  }
  for (const name of ["deps", "examples"]) {
    const dir = join(profile, name);
    if (!existsSync(dir)) continue;
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (!e.isFile()) continue;
      const path = join(dir, e.name);
      const stat = statSync(path);
      if (now - stat.mtimeMs > artifactAge) drop(path, stat.size);
    }
  }
  const fingerprints = join(profile, ".fingerprint");
  if (existsSync(fingerprints)) {
    for (const unit of dirs(fingerprints)) sweepUnit(unit, artifactAge);
  }
  const build = join(profile, "build");
  if (!existsSync(build)) continue;
  for (const pkg of dirs(build)) {
    // Cargo 1.100 groups units by package. A sibling with another hash can still be in use.
    const units = dirs(pkg).filter((path) => /^[0-9a-f]{16}$/.test(basename(path)));
    if (units.length) {
      for (const unit of units) sweepUnit(unit, artifactAge);
    } else if (/-[0-9a-f]{16}$/.test(basename(pkg))) {
      sweepUnit(pkg, artifactAge);
    }
  }
}

const gb = (freed / 1024 ** 3).toFixed(1);
console.log(`${values["dry-run"] ? "would remove" : "removed"} ${removed} entries, ${gb} GB, from ${target}`);
