// Publishes the Rust crates to crates.io in dependency order, across the four workspaces: taffy,
// gpui, redjs and Skathi's own.
//
//   bun scripts/publish-crates.ts --tag v0.3.0     publish what crates.io does not have yet
//   bun scripts/publish-crates.ts --dry-run        package and verify everything, publish nothing
//
// A version crates.io already has is skipped, so a re-run carries on where the last one stopped.
// crates.io answers a burst of publishes with 429 and the time to retry after (new crates: 5, then
// one per 10 minutes); the script waits that long and goes on.

import { spawnSync } from "node:child_process";
import { dirname, join, relative, resolve } from "node:path";
import { parseArgs } from "node:util";

const ROOT = resolve(import.meta.dir, "..");
// `followsTag`: the crate versions must equal the release tag. The taffy fork keeps upstream's.
const WORKSPACES = [
  { dir: "vendor/taffy", followsTag: false },
  { dir: "vendor/gpui", followsTag: true },
  { dir: "vendor/redjs/redjs", followsTag: true },
  { dir: ".", followsTag: true },
] as const;

const { values } = parseArgs({
  options: { tag: { type: "string" }, "dry-run": { type: "boolean", default: false } },
});
const dryRun = values["dry-run"];
if (!dryRun && !values.tag) throw new Error("--tag v<version> or --dry-run");

type Dep = { name: string; req: string; kind: string | null; path?: string };
type Pkg = { name: string; version: string; manifest_path: string; publish: string[] | null; dependencies: Dep[] };
type Crate = Pkg & { workspace: string };

function cargo(args: string[], cwd = ROOT): { status: number; out: string } {
  const r = spawnSync("cargo", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 1 << 28 });
  return { status: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function packagesOf(dir: string): Crate[] {
  const r = cargo(["metadata", "--format-version", "1", "--no-deps", "--manifest-path", join(ROOT, dir, "Cargo.toml")]);
  if (r.status !== 0) throw new Error(`cargo metadata in ${dir}:\n${r.out}`);
  const json = JSON.parse(r.out.slice(r.out.indexOf("{")));
  return (json.packages as Pkg[])
    .filter((p) => p.publish === null || p.publish.includes("crates-io"))
    .map((p) => ({ ...p, workspace: dir }));
}

// Dependency order within a workspace. A dependency without a version requirement (path-only, as
// the dev-dependency cycles are) is dropped from the published manifest, so it does not order.
function ordered(crates: Crate[]): Crate[] {
  const byName = new Map(crates.map((c) => [c.name, c]));
  const done = new Set<string>();
  const out: Crate[] = [];
  const visit = (c: Crate, stack: string[]) => {
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

const plan = WORKSPACES.flatMap((w) => ordered(packagesOf(w.dir)).map((c) => ({ ...c, followsTag: w.followsTag })));

if (values.tag) {
  const version = values.tag.replace(/^v/, "");
  const off = plan.filter((c) => c.followsTag && c.version !== version);
  if (off.length) throw new Error(`not at ${version}: ${off.map((c) => `${c.name}@${c.version}`).join(", ")}`);
}

if (dryRun) {
  // Each workspace packages against the earlier ones' sources, as if they were on crates.io.
  const patches: string[] = [];
  for (const w of WORKSPACES) {
    const crates = plan.filter((c) => c.workspace === w.dir);
    const args = [
      "package",
      "--manifest-path",
      join(ROOT, w.dir, "Cargo.toml"),
      ...crates.flatMap((c) => ["-p", c.name]),
    ];
    console.log(`\n== cargo package (${w.dir}): ${crates.map((c) => c.name).join(", ")}`);
    const r = spawnSync("cargo", [...patches, ...args], { cwd: ROOT, stdio: "inherit" });
    if (r.status !== 0) process.exit(r.status ?? 1);
    for (const c of crates) {
      const dir = relative(ROOT, dirname(c.manifest_path)).replaceAll("\\", "/");
      patches.push("--config", `patch.crates-io.${c.name}.path='${dir}'`);
    }
  }
  process.exit(0);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
for (const c of plan) {
  if (await published(c.name, c.version)) {
    console.log(`${c.name}@${c.version} is already on crates.io`);
    continue;
  }
  for (;;) {
    console.log(`\n== publishing ${c.name}@${c.version} (${c.workspace})`);
    const r = cargo(["publish", "-p", c.name, "--manifest-path", join(ROOT, c.workspace, "Cargo.toml")]);
    process.stdout.write(r.out);
    if (r.status === 0) break;
    const retry = r.out.match(/429[\s\S]*?try again after ([^.\n]+(?:GMT|UTC|Z))/i);
    if (!retry) process.exit(r.status);
    const until = Date.parse(retry[1]!);
    const wait = Math.max(30_000, (Number.isNaN(until) ? Date.now() + 600_000 : until) - Date.now() + 15_000);
    console.log(`crates.io rate limit: waiting ${Math.round(wait / 60_000)} min`);
    await sleep(wait);
  }
}
console.log("\nevery crate is on crates.io");
