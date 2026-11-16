// Publishes the Rust crates to crates.io in dependency order: skathi-taffy, the skathi-gpui
// workspace, redjs and Skathi's own.
//
//   bun scripts/publish-crates.ts --tag v0.3.0
//
// Everything is packaged and verified first (scripts/package-crates.ts), so a broken package stops
// the release before anything is uploaded. A version crates.io already has is skipped, so a re-run
// carries on where the last one stopped. crates.io answers a burst of publishes with 429 and the
// time to retry after (new crates: 5, then one per 10 minutes); the script waits that long.

import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { type Package, publishable, ROOT, STEPS } from "./package-crates";

const { values } = parseArgs({ options: { tag: { type: "string" } } });
if (!values.tag) throw new Error("--tag v<version>");
const version = values.tag.replace(/^v/, "");

// Within a workspace. A dependency without a version requirement (path-only, as the dev-dependency
// cycles are) is dropped from the published manifest, so it does not order.
function ordered(crates: Package[]): Package[] {
  const byName = new Map(crates.map((c) => [c.name, c]));
  const done = new Set<string>();
  const out: Package[] = [];
  const visit = (c: Package, stack: string[]) => {
    if (done.has(c.name)) return;
    if (stack.includes(c.name)) throw new Error(`dependency cycle: ${[...stack, c.name].join(" -> ")}`);
    for (const d of c.dependencies) {
      const dep = byName.get(d.name);
      if (dep && d.req !== "*") visit(dep, [...stack, c.name]);
    }
    done.add(c.name);
    out.push(c);
  };
  for (const c of crates) visit(c, []);
  return out;
}

async function published(name: string, version: string): Promise<boolean> {
  const res = await fetch(`https://crates.io/api/v1/crates/${name}/${version}`, {
    headers: { "User-Agent": "skathi-release (https://github.com/argon-chat/skathi)" },
  });
  if (res.status === 404) return false;
  if (res.ok) return true;
  throw new Error(`crates.io answered ${res.status} for ${name}@${version}`);
}

// The taffy fork keeps upstream's version; everything else is released at the tag's.
const plan = STEPS.flatMap((s) =>
  ordered(publishable(join(ROOT, s.dir), s.only)).map((c) => ({ ...c, dir: s.dir, only: s.only })),
);
const off = plan.filter((c) => !c.only && c.version !== version);
if (off.length) throw new Error(`not at ${version}: ${off.map((c) => `${c.name}@${c.version}`).join(", ")}`);

const pending: typeof plan = [];
for (const c of plan) {
  if (await published(c.name, c.version)) console.log(`${c.name}@${c.version} is already on crates.io`);
  else pending.push(c);
}
if (!pending.length) {
  console.log("every crate is on crates.io");
  process.exit(0);
}

const pack = spawnSync("bun", [join(ROOT, "scripts/package-crates.ts")], { cwd: ROOT, stdio: "inherit" });
if (pack.status !== 0) process.exit(pack.status ?? 1);

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
for (const c of pending) {
  for (;;) {
    console.log(`\n== publishing ${c.name}@${c.version} (${c.dir})`);
    // Verified above, against the packages of the crates before it.
    const r = spawnSync("cargo", ["publish", "--no-verify", "-p", c.name], {
      cwd: join(ROOT, c.dir),
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
    process.stdout.write(out);
    if (r.status === 0) break;
    const retry = out.match(/429[\s\S]*?try again after ([^.\n]+(?:GMT|UTC|Z))/i);
    if (!retry) process.exit(r.status ?? 1);
    const until = Date.parse(retry[1]!);
    const wait = Math.max(30_000, (Number.isNaN(until) ? Date.now() + 600_000 : until) - Date.now() + 15_000);
    console.log(`crates.io rate limit: waiting ${Math.round(wait / 60_000)} min`);
    await sleep(wait);
  }
}
console.log("\nevery crate is on crates.io");
