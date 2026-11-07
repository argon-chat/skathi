/** Generates an experimental native-AOT table matching the pinned Bun frontend.
 * No native host implementations are invented. Output is a trusted build input.
 * bun scripts/aot/generate-table.ts --bun-source <clean pinned checkout> --out <directory>
 */
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve, basename } from "node:path";
const FRONTEND = "7ca0042d3ba23ed60fd9cd8af1a1ce6ec3f8d074";
const WEBKIT = "4d45755fed4aeef19e95beb90d3d0c884a2812bd";
function argument(name: string): string {
  const index = process.argv.indexOf(name);
  const value = process.argv[index + 1];
  if (index < 0 || !value || value.startsWith("--")) throw new Error(`${name} needs a path`);
  return resolve(value);
}
const root = argument("--bun-source"), out = argument("--out");
const sdk = resolve(import.meta.dir, "../..");
const revision = Bun.spawnSync(["git", "-C", root, "rev-parse", "HEAD"]);
if (revision.exitCode || revision.stdout.toString().trim() !== FRONTEND) {
  throw new Error(`Bun checkout must be exactly ${FRONTEND}`);
}
const clean = Bun.spawnSync(["git", "-C", root, "diff", "--quiet", "HEAD", "--"]);
if (clean.exitCode !== 0) throw new Error("Bun tracked source checkout is modified; use a clean pinned checkout");
const engine = resolve(sdk, "vendor/redjs/prebuilt/bun-webkit/Source/JavaScriptCore/aot");
const generated = resolve(sdk, "vendor/redjs/prebuilt/bun-webkit/include/JavaScriptCore/aot");
const pins = readFileSync(resolve(sdk, "vendor/redjs/pins.txt"), "utf8");
if (!pins.includes(`webkit.version = ${WEBKIT}`)) throw new Error("Review the frontend/engine pin together before regenerating");
// The upstream parsers require exact LF data, even on Windows. Do not silently
// rewrite a user's source checkout or accept a table generated from CRLF text.
for (const name of ["module-exports.txt", "own-properties.txt"]) {
  const path = resolve(root, "src/codegen/builtins/data", name);
  if (readFileSync(path, "utf8").includes("\r")) throw new Error(`Checkout ${path} with core.autocrlf=false (LF required)`);
}
process.env.AOT_ENGINE_DIRECTORY = engine;
const { generate } = await import(resolve(root, "src/codegen/builtins/index.ts"));
const files: string[] = [...new Bun.Glob("src/**/*.bind.ts").scanSync({ cwd: root, absolute: true })];
const sources = files.map(file => {
  for (let directory = dirname(file); directory !== dirname(directory); directory = dirname(directory)) {
    const manifest = resolve(directory, "Cargo.toml");
    if (!existsSync(manifest)) continue;
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(readFileSync(manifest, "utf8"));
    if (name) return `${name[1]}=${file}`;
  }
  throw new Error(`No Cargo package for ${file}`);
});
const result = await generate({
  bunRoot: root, javaScriptCore: null, prebuiltGenerated: generated,
  target: "windows", bindgenSources: sources,
  typesOnly: files.filter(file => file.endsWith("_test.bind.ts")).map(file => basename(file)),
});
mkdirSync(out, { recursive: true });
for (const [name, code] of result.codegen) {
  writeFileSync(resolve(out, name), code.replace('#include "root.h"', '#include "config.h"'));
}
writeFileSync(resolve(out, "aot_builtin_table.json"), JSON.stringify(result.dump, null, 2));
writeFileSync(resolve(out, ".redjs-aot-table"), `redjs.aot-builtin-table.v1\nfrontend=${FRONTEND}\nwebkit=${WEBKIT}\n`);
console.log(`Generated ${result.codegen.size} matching native-AOT inputs in ${out}`);