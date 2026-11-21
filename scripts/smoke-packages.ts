// Creates apps from the packed npm packages, as a user gets them, and builds them.
//
//   bun scripts/smoke-packages.ts                    packs runtime and cli (with their prepack) first
//   bun scripts/smoke-packages.ts --tarballs <dir>   uses the tarballs release.yml is about to publish
//
// Each template is created with the packed CLI, installed from the tarballs and run through sk check
// and sk build --payload-only, which needs no host from a release.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const SDK = resolve(import.meta.dir, "..");
const { values } = parseArgs({ options: { tarballs: { type: "string" }, keep: { type: "boolean" } } });

const version = (JSON.parse(readFileSync(join(SDK, "packages/cli/package.json"), "utf8")) as { version: string })
  .version;
const work = mkdtempSync(join(tmpdir(), "sk-smoke-"));

function run(cmd: string[], cwd: string): void {
  console.log(`$ ${cmd.join(" ")}   (${cwd})`);
  const r = spawnSync(cmd[0]!, cmd.slice(1), { cwd, stdio: "inherit", shell: process.platform === "win32" });
  if (r.status !== 0) throw new Error(`${cmd.join(" ")} exited with ${r.status ?? r.signal}`);
}

const tarballs = values.tarballs ? resolve(values.tarballs) : join(work, "tarballs");
if (!values.tarballs) {
  mkdirSync(tarballs);
  for (const pkg of ["runtime", "cli"])
    run(["bun", "pm", "pack", "--destination", tarballs], join(SDK, "packages", pkg));
}
const tgz = (name: string) => `file:${join(tarballs, `${name}-${version}.tgz`).replaceAll("\\", "/")}`;
const runtime = tgz("skathi-native-runtime");
const cli = tgz("skathi-native-cli");

const write = (path: string, json: unknown) => writeFileSync(path, `${JSON.stringify(json, null, 2)}\n`);

const tool = join(work, "tool");
mkdirSync(tool);
write(join(tool, "package.json"), {
  private: true,
  dependencies: { "@skathi-native/cli": cli },
  overrides: { "@skathi-native/runtime": runtime },
});
run(["bun", "install"], tool);
const sk = join(tool, "node_modules/@skathi-native/cli/cli/sk.ts");

const templates: { name: string; args: string[] }[] = [
  { name: "showcase", args: ["--template", "showcase"] },
  { name: "minimal-vue", args: ["--template", "minimal", "--framework", "vue"] },
  { name: "minimal-svelte", args: ["--template", "minimal", "--framework", "svelte"] },
];

try {
  for (const t of templates) {
    run(["bun", sk, "create", t.name, ...t.args, "--pm", "bun", "-y"], work);
    const dir = join(work, t.name);
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as {
      dependencies?: Record<string, string>;
      devDependencies?: Record<string, string>;
      overrides?: Record<string, string>;
    };
    pkg.dependencies = { ...pkg.dependencies, "@skathi-native/runtime": runtime };
    pkg.devDependencies = { ...pkg.devDependencies, "@skathi-native/cli": cli };
    pkg.overrides = { "@skathi-native/runtime": runtime };
    write(join(dir, "package.json"), pkg);
    run(["bun", "install"], dir);
    run(["bunx", "sk", "check"], dir);
    run(["bunx", "sk", "build", "--payload-only"], dir);
  }
  console.log(`✓ ${templates.map((t) => t.name).join(", ")} build from the ${version} packages`);
} finally {
  if (!values.keep) rmSync(work, { recursive: true, force: true });
}
