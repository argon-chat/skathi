// Fills tools/sk-aot/.cache for `cargo build`: Bun's sources at the frontend pin of
// packages/cli/aot/toolchain.lock.json, the C dependencies at Bun's pins, the builtin table, and
// Bun's codegen with the AOT schema and builtin sources. Needs bun >= 1.4, git, curl and tar.
//   bun tools/sk-aot/setup.ts
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, copyFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { loadPin, verifyEngine, verifyTable, webkitDirectory } from "../../scripts/aot/toolchain";

const here = import.meta.dir;
const sdk = resolve(here, "../..");
const cache = join(here, ".cache");
const bun = join(cache, "bun");
const table = join(cache, "table");
const engine = join(webkitDirectory(sdk), "Source/JavaScriptCore/aot");
const pin = loadPin();
const revision = pin.frontend.revision;
const SPARSE = [
  "src",
  "scripts",
  "packages",
  "docs/compiler",
  "docs/lint/rules",
  "completions",
  "patches",
  "test/aot/native",
];

function run(command: string[], options: { cwd?: string; env?: Record<string, string>; allowFailure?: boolean } = {}) {
  console.log(`$ ${command.join(" ")}`);
  const result = Bun.spawnSync(command, {
    cwd: options.cwd,
    env: { ...process.env, ...options.env },
    stdout: "inherit",
    stderr: "inherit",
  });
  if (!result.success && !options.allowFailure) throw new Error(`${command[0]} exited with ${result.exitCode}`);
  return result.success;
}

function read(command: string[], cwd?: string): string {
  const result = Bun.spawnSync(command, { cwd, stdout: "pipe", stderr: "pipe" });
  return result.success ? result.stdout.toString().trim() : "";
}

if (Bun.semver.order(Bun.version.replace(/-.*/, ""), "1.4.0") < 0)
  throw new Error(`Bun's codegen needs bun >= 1.4, this is ${Bun.version}`);

function checkout() {
  if (existsSync(join(bun, ".git")) && read(["git", "rev-parse", "HEAD"], bun) === revision) return;
  rmSync(bun, { recursive: true, force: true });
  mkdirSync(bun, { recursive: true });
  const git = (...args: string[]) => run(["git", ...args], { cwd: bun });
  git("init", "-q");
  git("config", "core.autocrlf", "false");
  git("config", "core.longpaths", "true");
  git("sparse-checkout", "init", "--cone");
  git("sparse-checkout", "set", ...SPARSE);
  git("remote", "add", "origin", "https://github.com/oven-sh/bun.git");
  git("fetch", "-q", "--filter=blob:none", "--depth=1", "--no-tags", "origin", revision);
  git("checkout", "-q", "--detach", revision);
}

function dependencyCommit(name: string, constant: string): string {
  const text = readFileSync(join(bun, `scripts/build/deps/${name}.ts`), "utf8");
  const found = text.match(new RegExp(`${constant}\\s*=\\s*"([0-9a-f]{40})"`));
  if (!found) throw new Error(`scripts/build/deps/${name}.ts has no ${constant}`);
  return found[1]!;
}

function vendor(name: string, repo: string, commit: string, patches: string[] = []) {
  const folder = join(bun, "vendor", name);
  const stamp = join(folder, ".sk-aot-commit");
  if (existsSync(stamp) && readFileSync(stamp, "utf8") === commit) return;
  rmSync(folder, { recursive: true, force: true });
  mkdirSync(folder, { recursive: true });
  const archive = `${name}.tgz`;
  const vendored = join(bun, "vendor");
  run(["curl", "-sSfL", "-o", archive, `https://github.com/${repo}/archive/${commit}.tar.gz`], { cwd: vendored });
  // Relative paths: GNU tar takes `G:` for a remote host. zstd's tests hold links that it cannot make on Windows.
  run(["tar", "-xzf", archive, "-C", name, "--strip-components=1", "--exclude=tests"], { cwd: vendored });
  rmSync(join(vendored, archive));
  for (const patch of patches) run(["git", "apply", "--whitespace=nowarn", join(bun, patch)], { cwd: folder });
  writeFileSync(stamp, commit);
}

function libuvPatches(): string[] {
  const text = readFileSync(join(bun, "scripts/build/deps/libuv.ts"), "utf8");
  return [...text.matchAll(/"(patches\/libuv\/[^"]+\.patch)"/g)].map((found) => found[1]!);
}

// A free drive letter for the cache: Bun's codegen command lines are too long under the real path.
function substDrive(): string {
  for (const letter of "STUVWXYZ") {
    if (!existsSync(`${letter}:\\`) && run(["subst", `${letter}:`, cache], { allowFailure: true })) return `${letter}:`;
  }
  throw new Error("no free drive letter for subst");
}

function codegen() {
  const isWindows = process.platform === "win32";
  const drive = isWindows ? substDrive() : undefined;
  try {
    const root = drive ? `${drive}\\bun` : bun;
    const home = drive ? `${drive}\\home` : join(cache, "home");
    mkdirSync(join(cache, "home"), { recursive: true });
    const env: Record<string, string> = { USERPROFILE: home, HOME: home, AOT_ENGINE_DIRECTORY: engine };
    // Bun's codegen needs perl; Git for Windows carries one. Appended, so nothing else on PATH is shadowed.
    const git = Bun.which("git");
    if (!Bun.which("perl") && git) {
      const gitUsrBin = ["../..", "../../..", "../../../.."]
        .map((up) => resolve(git, up, "usr/bin"))
        .find((dir) => existsSync(join(dir, "perl.exe")));
      const key = Object.keys(process.env).find((k) => k.toUpperCase() === "PATH") ?? "PATH";
      if (gitUsrBin) env[key] = `${process.env[key] ?? ""};${gitUsrBin}`;
    }
    rmSync(join(bun, "build/release-codegen"), { recursive: true, force: true });
    // Bindgen fails here and nothing that sk-aot builds needs it: what is needed is checked below.
    run([process.execPath, "scripts/build.ts", "--mode=codegen", "--profile=release", "--target=codegen", "-k0"], {
      cwd: root,
      env,
      allowFailure: true,
    });
    const out = join(root, "build/release-codegen/codegen");
    run(
      [
        process.execPath,
        "run",
        join(engine, "ir/generator/index.ts"),
        "--schema",
        join(engine, "ir/AOTSchema.def"),
        "--rust-out",
        out,
      ],
      { cwd: root, env },
    );
    for (const name of readdirSync(table).filter((name) => /\.(rs|json)$/.test(name)))
      copyFileSync(join(table, name), join(out, name));
    run(
      [
        process.execPath,
        "run",
        "src/codegen/builtins-manifest.ts",
        "--debug=OFF",
        join(out, "builtins.manifest"),
        join(out, "builtin_sources.bin"),
        `--engine-functions=${join(out, "aot_engine_functions.json")}`,
        `--engine-directory=${engine}`,
        `--bundled-names-of-modules=${join(out, "bundled_names_of_modules.txt")}`,
        `--bundled-helpers=${join(out, "runtime.out.js")}`,
        `--evaluated-modules=${join(out, "InternalModuleRegistryConstants.bin")}`,
      ],
      { cwd: root, env },
    );
  } finally {
    if (drive) run(["subst", drive, "/D"], { allowFailure: true });
    // Codegen rewrites the header of a tracked file with its own path.
    run(["git", "checkout", "--", "src/js_parser/defines_table.generated.rs"], { cwd: bun, allowFailure: true });
  }
  for (const name of ["builtin_sources.bin", "aot_schema.rs", "aot_builtin_table.rs", "runtime.out.js"]) {
    if (!existsSync(join(bun, "build/release-codegen/codegen", name))) throw new Error(`codegen did not write ${name}`);
  }
}

verifyEngine(sdk, pin);
checkout();
vendor("lolhtml", "oven-sh/lol-html", dependencyCommit("lolhtml", "LOLHTML_COMMIT"));
vendor("zstd", "facebook/zstd", dependencyCommit("zstd", "ZSTD_COMMIT"));
if (process.platform === "win32")
  vendor("libuv", "oven-sh/libuv", dependencyCommit("libuv", "LIBUV_COMMIT"), libuvPatches());
run([process.execPath, join(sdk, "scripts/aot/generate-table.ts"), "--bun-source", bun, "--out", table], { cwd: sdk });
await verifyTable(table, sdk, pin);
codegen();
console.log(`tools/sk-aot/.cache is at frontend ${revision}; build with: cd tools/sk-aot && cargo build --release`);
