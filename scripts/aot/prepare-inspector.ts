/** Build a fresh native inspector from one production bundle, for embedding in
 * the ordinary host executable. Heavy subprocesses run only from this CLI.
 * bun scripts/aot/prepare-inspector.ts --out DIR --cache DIR
 * Optional verified local inputs: --compiler <sk-aot.exe> (default: tools/sk-aot's release build)
 * --bun-source DIR --table DIR.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { compilerInfo, repoSkAot } from "../../packages/cli/aot/native";
import {
  fileDigest,
  loadPin,
  needsTable,
  verifyCompilerInfo,
  verifyEngine,
  verifyFile,
  verifyTable,
} from "./toolchain";
const sdk = resolve(import.meta.dir, "../..");
function argument(name: string, optional = false): string | undefined {
  const index = process.argv.indexOf(name),
    value = process.argv[index + 1];
  if (index < 0 && optional) return undefined;
  if (index < 0 || !value || value.startsWith("--")) throw Error(`${name} needs a path`);
  return resolve(value);
}
function run(command: string[], cwd = sdk, extra: Record<string, string> = {}): string {
  const child = Bun.spawnSync(command, {
    cwd,
    env: { ...process.env, GIT_TERMINAL_PROMPT: "0", ...extra },
    stdout: "pipe",
    stderr: "pipe",
  });
  if (child.exitCode !== 0)
    throw Error(`${command[0]} failed (${child.exitCode})\n${child.stdout.toString()}\n${child.stderr.toString()}`);
  if (child.stderr.length) console.error(child.stderr.toString().trim());
  return child.stdout.toString();
}
export async function prepare(): Promise<void> {
  if (process.platform !== "win32" || process.arch !== "x64")
    throw Error("Native inspector packaging currently requires Windows x64");
  const pin = loadPin();
  verifyEngine(sdk, pin);
  const out = argument("--out")!,
    cache = argument("--cache")!;
  mkdirSync(out, { recursive: true });
  mkdirSync(cache, { recursive: true });
  const compiler = argument("--compiler", true) ?? repoSkAot(sdk);
  if (!existsSync(compiler))
    throw Error(
      `No sk-aot at ${compiler}; build it (bun tools/sk-aot/setup.ts, then cargo build --release in tools/sk-aot) or pass --compiler`,
    );
  verifyCompilerInfo({ ...(await compilerInfo(compiler)) }, pin);
  const table = argument("--table", true) ?? join(cache, `table-${pin.table.id}`);
  if (needsTable(table)) {
    const source = argument("--bun-source", true) ?? join(cache, `source-${pin.frontend.revision}`);
    if (!existsSync(join(source, ".git"))) {
      if (existsSync(source))
        throw Error("AOT source cache exists without Git provenance; choose another --cache or --bun-source");
      // Fetch only the reviewed commit: a full moving HEAD is not an input to the table.
      run(["git", "init", source]);
      run(["git", "-C", source, "config", "core.autocrlf", "false"]);
      run(["git", "-C", source, "config", "core.longpaths", "true"]);
      run(["git", "-C", source, "sparse-checkout", "init", "--cone"]);
      run(["git", "-C", source, "remote", "add", "origin", pin.frontend.sourceRepository]);
      run([
        "git",
        "-C",
        source,
        "fetch",
        "--filter=blob:none",
        "--depth=1",
        "--no-tags",
        "origin",
        pin.frontend.revision,
      ]);
      run(["git", "-C", source, "checkout", "--detach", pin.frontend.revision]);
    }
    // Bun's build globs these source lists; a cached checkout gets new paths too.
    run([
      "git",
      "-C",
      source,
      "sparse-checkout",
      "set",
      "src",
      "scripts",
      "packages",
      "docs/lint/rules",
      "test/aot/native",
    ]);
    if (run(["git", "-C", source, "rev-parse", "HEAD"]).trim() !== pin.frontend.revision)
      throw Error("AOT source checkout revision mismatch");
    if (run(["git", "-C", source, "status", "--porcelain", "--untracked-files=normal"]).trim())
      throw Error("AOT source checkout is not clean");
    run([process.execPath, join(sdk, "scripts/aot/generate-table.ts"), "--bun-source", source, "--out", table]);
  }
  await verifyTable(table, sdk, pin);
  const source = join(out, "source");
  mkdirSync(source, { recursive: true });
  const bundle = join(source, "app.prod.js");
  // This is the only SFC/TS production build. The exact resulting JS feeds
  // both the native compiler and the host's SK_DEVTOOLS_BUNDLE override.
  run(
    [
      process.execPath,
      join(sdk, "packages/cli/prod.ts"),
      join(sdk, "apps/devtools/src/main.ts"),
      bundle,
      "--aot-inputs",
      join(source, "typed-inputs"),
    ],
    join(sdk, "packages/cli"),
  );
  console.log(
    run([
      process.execPath,
      join(sdk, "scripts/aot/build-devtools.ts"),
      "--compiler",
      compiler,
      "--table",
      table,
      "--bundle",
      bundle,
      "--out",
      out,
    ]).trim(),
  );
  const manifest = join(out, "inspector.native.json");
  const document = JSON.parse(readFileSync(manifest, "utf8"));
  verifyCompilerInfo(document, pin);
  await verifyFile(join(out, document.image), document.imageSha256);
  await verifyFile(join(out, "app.prod.js"), document.sourceBundleSha256);
  const receipt = {
    schema: "skathi.embedded-native-inspector",
    version: 1,
    stage: "native-image",
    manifest,
    bundle: join(out, "app.prod.js"),
    table,
    compiler,
    imageSha256: document.imageSha256,
    sourceBundleSha256: document.sourceBundleSha256,
    compilerSha256: await fileDigest(compiler),
  };
  writeFileSync(join(out, "build.json"), JSON.stringify(receipt, null, 2) + "\n");
  console.log(`Embedded inspector build inputs: ${join(out, "build.json")}`);
}
if (import.meta.main) await prepare();
