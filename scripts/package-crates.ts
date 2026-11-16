// Packages every crate Skathi publishes, in publish order, without publishing anything.
//
//   bun scripts/package-crates.ts [--out <dir>] [--no-verify]
//
// Writes the devtools inspector bundle the published sk-app carries (crates/sk-app/devtools/),
// then runs `cargo package` for skathi-taffy, the skathi-gpui workspace, redjs and Skathi's own
// crates. Each step verifies against the unpacked packages of the steps before it (a
// `[patch.crates-io]` in <out>/patch.toml), so only what would be uploaded is compiled. <out>
// (default target/crates) is also the target directory: <out>/package holds the .crate files and
// their unpacked contents, and the final patch.toml points another project at all of them.

import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

export const ROOT = resolve(import.meta.dir, "..");

// `only`: the one crate of a workspace that is published; otherwise every member not `publish = false`.
export const STEPS: { dir: string; only?: string }[] = [
  { dir: "vendor/taffy", only: "skathi-taffy" },
  { dir: "vendor/gpui" },
  { dir: "vendor/redjs/redjs" },
  { dir: "." },
];

function run(cmd: string[], cwd: string): void {
  console.error(`> ${cmd.join(" ")}  (${cwd})`);
  const proc = Bun.spawnSync(cmd, { cwd, stdio: ["inherit", "inherit", "inherit"] });
  if (proc.exitCode !== 0) throw new Error(`${cmd[0]} exited with ${proc.exitCode}`);
}

export interface Package {
  name: string;
  version: string;
  publish: string[] | null;
  dependencies: { name: string; req: string }[];
}

export function publishable(cwd: string, only: string | undefined): Package[] {
  const proc = Bun.spawnSync(["cargo", "metadata", "--no-deps", "--format-version", "1"], { cwd });
  if (proc.exitCode !== 0) throw new Error(`cargo metadata failed in ${cwd}`);
  const all = (JSON.parse(proc.stdout.toString()) as { packages: Package[] }).packages;
  return all.filter((p) => (only ? p.name === only : p.publish === null || p.publish.length > 0));
}

if (import.meta.main) packageAll();

function packageAll(): void {
  const { values } = parseArgs({
    options: { out: { type: "string" }, "no-verify": { type: "boolean" } },
  });
  const out = resolve(values.out ?? join(ROOT, "target", "crates"));
  const packages = join(out, "package");
  const patch = join(out, "patch.toml");

  function writePatch(done: Package[]): void {
    const lines = ["[patch.crates-io]"];
    for (const p of done) {
      const dir = join(packages, `${p.name}-${p.version}`).replaceAll("\\", "/");
      lines.push(`${p.name} = { path = ${JSON.stringify(dir)} }`);
    }
    writeFileSync(patch, lines.join("\n") + "\n");
  }

  run(
    ["bun", "prod.ts", join(ROOT, "apps/devtools/src/main.ts"), join(ROOT, "crates/sk-app/devtools/app.prod.js")],
    join(ROOT, "packages/cli"),
  );

  mkdirSync(packages, { recursive: true });
  const done: Package[] = [];
  for (const step of STEPS) {
    const cwd = join(ROOT, step.dir);
    const these = publishable(cwd, step.only);
    writePatch(done);
    const cmd = ["cargo", "package", ...these.flatMap((p) => ["-p", p.name])];
    cmd.push("--target-dir", out, "--config", patch, "--allow-dirty");
    if (values["no-verify"]) cmd.push("--no-verify");
    // The patch would otherwise land in the workspace's lockfile as `[[patch.unused]]`.
    const lock = join(cwd, "Cargo.lock");
    const locked = existsSync(lock) ? readFileSync(lock) : null;
    try {
      run(cmd, cwd);
    } finally {
      if (locked) writeFileSync(lock, locked);
    }
    done.push(...these);
  }
  writePatch(done);

  for (const p of done) {
    const file = `${p.name}-${p.version}.crate`;
    const kib = statSync(join(packages, file)).size / 1024;
    console.log(`${file.padEnd(44)} ${kib.toFixed(0).padStart(8)} KiB`);
  }
  console.log(`\n${patch}`);
}
