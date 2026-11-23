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
import { join } from "node:path";
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

function size(path: string): number {
  let total = 0;
  for (const e of readdirSync(path, { withFileTypes: true, recursive: true })) {
    if (e.isFile()) total += statSync(join(e.parentPath, e.name)).size;
  }
  return total;
}

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

/** `<target>/<profile>` and `<target>/<triple>/<profile>`: the directories that hold `deps`. */
function profiles(): string[] {
  const out: string[] = [];
  for (const a of dirs(target)) {
    if (existsSync(join(a, "deps"))) out.push(a);
    else for (const b of dirs(a)) if (existsSync(join(b, "deps"))) out.push(b);
  }
  return out;
}

for (const profile of profiles()) {
  const incremental = join(profile, "incremental");
  if (existsSync(incremental)) {
    for (const dir of dirs(incremental)) if (now - touched(dir) > cacheAge) drop(dir, size(dir));
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
  for (const name of [".fingerprint", "build"]) {
    const dir = join(profile, name);
    if (!existsSync(dir)) continue;
    for (const unit of dirs(dir)) if (now - touched(unit) > artifactAge) drop(unit, size(unit));
  }
}

const gb = (freed / 1024 ** 3).toFixed(1);
console.log(`${values["dry-run"] ? "would remove" : "removed"} ${removed} entries, ${gb} GB, from ${target}`);
