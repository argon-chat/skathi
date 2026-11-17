// Publishes Skathi's own crates to crates.io in dependency order.
//
//   bun scripts/publish-crates.ts --tag v0.4.0 [--check]
//
// skathi-taffy (vendor/taffy), skathi-gpui (vendor/gpui) and redjs (vendor/redjs) are released from
// their own repositories, on version lines of their own (their READMEs say how), and Skathi's crates
// require versions of them that crates.io has. So the script first checks every one of those crates
// Skathi builds with: crates.io must have the version its vendor checkout is at, and the checkout must
// have no changes to it since that version was published. --check stops after that.
//
// Then Skathi's crates are packaged and verified (scripts/package-crates.ts), so a broken package
// stops the release before anything is uploaded. A version crates.io already has is skipped, so a
// re-run carries on where the last one stopped. crates.io answers a burst with 429 and the time to
// retry after (updates: 30, then one a minute; new crates: 5, then one per 10 minutes); the script
// waits that long.
//
// Release order:
//   1. `bun scripts/publish-crates.ts --tag v<next> --check` names each vendor repository that has
//      changes crates.io does not.
//   2. For each, in this order: argon-chat/taffy (`cargo publish` on branch vue-native; a new version
//      also needs gpui's `taffy` requirement raised, so a gpui release), argon-chat/skathi-gpui,
//      argon-chat/redjs. Bump its version (semver of its own API), commit, push the tag
//      v<its version>; its release workflow publishes it.
//   3. Here: point the submodules at those commits, raise the `version` of every skathi-gpui* and
//      redjs* dependency to the released one, and run step 1 again until it passes.
//      `bun scripts/package-crates.ts --vendor` tries Skathi's packages against vendor/ before then.
//   4. Bump Skathi's version (the workspace, its crates' requirements on each other, and the npm
//      packages), commit, push the tag v<version>. .github/workflows/release.yml checks the vendor
//      crates again before it builds anything, and runs this script last.

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { parseArgs } from "node:util";
import { type Package, publishable, ROOT, VENDOR } from "./package-crates";

const { values } = parseArgs({ options: { tag: { type: "string" }, check: { type: "boolean" } } });
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

const USER_AGENT = "skathi-release (https://github.com/argon-chat/skathi)";

async function published(name: string, version: string): Promise<boolean> {
  const res = await fetch(`https://crates.io/api/v1/crates/${name}/${version}`, {
    headers: { "User-Agent": USER_AGENT },
  });
  if (res.status === 404) return false;
  if (!res.ok) throw new Error(`crates.io answered ${res.status} for ${name}@${version}`);
  return !((await res.json()) as { version: { yanked: boolean } }).version.yanked;
}

// The commit and directory `cargo publish` recorded in the package (.cargo_vcs_info.json).
async function publishedFrom(name: string, version: string): Promise<{ sha1: string; path: string } | null> {
  const res = await fetch(`https://static.crates.io/crates/${name}/${name}-${version}.crate`, {
    headers: { "User-Agent": USER_AGENT },
  });
  if (!res.ok) throw new Error(`crates.io answered ${res.status} for the ${name}@${version} package`);
  const tar = Bun.gunzipSync(new Uint8Array(await res.arrayBuffer()));
  const field = (at: number, len: number) => new TextDecoder().decode(tar.subarray(at, at + len)).replace(/\0.*$/s, "");
  for (let at = 0; at + 512 <= tar.length;) {
    const path = field(at, 100);
    if (!path) break;
    const size = parseInt(field(at + 124, 12).trim() || "0", 8);
    if (path.endsWith("/.cargo_vcs_info.json")) {
      const info = JSON.parse(field(at + 512, size)) as { git?: { sha1: string }; path_in_vcs?: string };
      return info.git ? { sha1: info.git.sha1, path: info.path_in_vcs ?? "" } : null;
    }
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return null;
}

function git(cwd: string, args: string[]): { status: number | null; out: string } {
  const r = spawnSync("git", args, { cwd, encoding: "utf8" });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim() };
}

// Whether the checkout (working tree included) differs from `sha1` under the crate's directory or
// in its workspace manifest. A shallow submodule fetches the commit first.
function changedSince(repo: string, sha1: string, paths: string[]): boolean {
  if (git(repo, ["cat-file", "-e", `${sha1}^{commit}`]).status !== 0) {
    const fetched = git(repo, ["fetch", "--quiet", "--depth", "1", "origin", sha1]);
    if (fetched.status !== 0) throw new Error(`cannot fetch ${sha1} in ${repo}: ${fetched.out}`);
  }
  const diff = git(repo, ["diff", "--quiet", sha1, "--", ...paths]);
  if (diff.status === 0) return false;
  if (diff.status === 1) return true;
  throw new Error(`git diff ${sha1} failed in ${repo}: ${diff.out}`);
}

async function checkVendor(): Promise<string[]> {
  const lock = Bun.TOML.parse(readFileSync(join(ROOT, "Cargo.lock"), "utf8")) as {
    package: { name: string; source?: string }[];
  };
  const local = new Set(lock.package.filter((p) => !p.source).map((p) => p.name));
  const problems: string[] = [];
  for (const step of VENDOR) {
    const dir = join(ROOT, step.dir);
    const repo = git(dir, ["rev-parse", "--show-toplevel"]).out;
    const manifest = relative(repo, join(dir, "Cargo.toml")).replaceAll("\\", "/");
    const sub = relative(ROOT, repo).replaceAll("\\", "/");
    const used = publishable(dir, step.only).filter((c) => local.has(c.name));
    const missing: string[] = [];
    const changed: string[] = [];
    for (const c of used) {
      if (!(await published(c.name, c.version))) {
        missing.push(`${c.name} ${c.version}`);
        continue;
      }
      const from = await publishedFrom(c.name, c.version);
      if (!from) continue;
      const crateDir = relative(repo, dirname(c.manifest_path)).replaceAll("\\", "/") || ".";
      if (changedSince(repo, from.sha1, [crateDir, manifest])) {
        changed.push(`${c.name} (${crateDir}) since ${c.version} was published from ${from.sha1.slice(0, 10)}`);
      }
    }
    if (!missing.length && !changed.length) {
      console.log(`${sub}: the ${used.length} crates Skathi uses are on crates.io`);
      continue;
    }
    const where = used[0]?.repository ?? step.dir;
    const lines = [`${sub} needs a release of ${where} first:`];
    if (missing.length) lines.push(`  not on crates.io: ${missing.join(", ")}`);
    if (changed.length) lines.push(`  changed: ${changed.join("; ")}`);
    lines.push(
      `  Publish it from its repository, at a new version if crates.io has this one,`,
      `  point ${sub} at that commit, and require that version here.`,
    );
    problems.push(lines.join("\n"));
  }
  return problems;
}

const problems = await checkVendor();
if (problems.length) {
  console.error(`\n${problems.join("\n\n")}`);
  process.exit(1);
}
if (values.check) process.exit(0);

const plan = ordered(publishable(ROOT, undefined));
const off = plan.filter((c) => c.version !== version);
if (off.length) throw new Error(`not at ${version}: ${off.map((c) => `${c.name}@${c.version}`).join(", ")}`);

const pending: Package[] = [];
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
    console.log(`\n== publishing ${c.name}@${c.version}`);
    // Verified above, against the crates before it. --allow-dirty: sk-app carries the devtools bundle
    // package-crates.ts just generated, which git ignores.
    const r = spawnSync("cargo", ["publish", "--no-verify", "--allow-dirty", "-p", c.name], {
      cwd: ROOT,
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
