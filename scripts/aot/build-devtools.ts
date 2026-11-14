/** The devtools inspector -> JSC native image. This is not Bun standalone packaging.
 * bun scripts/aot/build-devtools.ts --compiler <sk-aot.exe> --table <generated matching table>
 *   --out <artifact directory> [--bundle <production JS>]
 */
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { compileNative, compilerInfo, nativeManifest } from "../../packages/cli/aot/native";
import { hostGlobalsFromTable, opaqueGlobals, TABLE_ID, type BuiltinEntry } from "../../packages/cli/aot/transform";
import { loadPin, verifyCompilerInfo, verifyEngine, verifyTable } from "./toolchain";
const PIN = loadPin();
const sdk = resolve(import.meta.dir, "../..");
function argument(name: string, optional = false): string | undefined {
  const index = process.argv.indexOf(name),
    value = process.argv[index + 1];
  if (index < 0 && optional) return undefined;
  if (index < 0 || !value || value.startsWith("--")) throw new Error(`${name} needs a path`);
  return resolve(value);
}
const compiler = argument("--compiler")!;
const table = argument("--table")!,
  out = argument("--out")!;
const digest = (bytes: string | Uint8Array): string => new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
function run(command: string[], cwd = sdk): string {
  const child = Bun.spawnSync(command, { cwd, stdout: "pipe", stderr: "pipe" });
  const stdout = child.stdout.toString(),
    stderr = child.stderr.toString();
  if (child.exitCode) throw new Error(`${command[0]} failed (${child.exitCode})\n${stdout}\n${stderr}`);
  if (stderr.trim()) console.error(stderr.trim());
  return stdout;
}
verifyEngine(sdk, PIN);
await verifyTable(table, sdk, PIN);
const info = await compilerInfo(compiler);
verifyCompilerInfo({ ...info }, PIN);
const dump = JSON.parse(readFileSync(join(table, "aot_builtin_table.json"), "utf8")) as { entries: BuiltinEntry[] };
const derived = opaqueGlobals(hostGlobalsFromTable(dump.entries));
if (TABLE_ID !== PIN.table.id || derived.join() !== opaqueGlobals().join())
  throw new Error(
    "packages/cli/aot/host-globals.json differs from the pinned table; run bun scripts/aot/host-globals.ts",
  );
mkdirSync(out, { recursive: true });
const stage = mkdtempSync(join(out, ".native-stage-"));
const bundle = argument("--bundle", true) ?? join(stage, "app.prod.js");
if (!process.argv.includes("--bundle")) {
  run(
    [
      process.execPath,
      join(sdk, "packages/cli/prod.ts"),
      join(sdk, "apps/devtools/src/main.ts"),
      bundle,
      "--aot-inputs",
      join(stage, "typed-inputs"),
    ],
    join(sdk, "packages/cli"),
  );
}
const source = readFileSync(bundle, "utf8");
writeFileSync(join(stage, "opaque-host-globals.json"), JSON.stringify(derived, null, 2));
const native = await compileNative({ compiler, source, work: stage });
console.log(native.diagnostics);
const prelude = Buffer.concat([
  readFileSync(join(sdk, "crates/sk-app/src/prelude.js")),
  Buffer.from([0]),
  readFileSync(join(sdk, "crates/sk-app/src/window.prelude.js")),
]);
const imageName = `inspector.${native.imageSha256.slice(0, 16)}.image`;
writeFileSync(join(out, imageName), native.image);
if (resolve(bundle) !== resolve(out, "app.prod.js")) copyFileSync(bundle, join(out, "app.prod.js"));
const windowConfig = join(resolve(bundle, ".."), "window.json");
if (existsSync(windowConfig) && resolve(windowConfig) !== resolve(out, "window.json"))
  copyFileSync(windowConfig, join(out, "window.json"));
const manifest = nativeManifest("inspector", info, native, { image: imageName, preludeSha256: digest(prelude) });
writeFileSync(join(out, "inspector.native.json"), JSON.stringify(manifest, null, 2));
console.log(`Native inspector artifact: ${join(out, "inspector.native.json")} (${native.image.length} bytes)`);
console.log(`Build the inspector with SK_DEVTOOLS_BUNDLE=${join(out, "app.prod.js")} to match this artifact.`);
