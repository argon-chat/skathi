/** Experimental real devtools -> JSC native image. This is not Bun standalone packaging.
 * bun scripts/aot/build-devtools.ts --frontend <pinned bun.exe> --compiler <aot-image.exe>
 *   --table <generated matching table> --out <artifact directory> [--bundle <production JS>] [--kind inspector|application]
 */
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, copyFileSync, existsSync } from "node:fs";
import { resolve, join } from "node:path";
import { createHash } from "node:crypto";
import { loadPin, verifyCompilerInfo, verifyEngine, verifyFile, verifyTable } from "./toolchain";
const PIN = loadPin(resolve(import.meta.dir, "toolchain.lock.json"));
const FRONTEND = PIN.frontend.revision;
const TRANSFORM = "opaque-host-v1";
const sdk = resolve(import.meta.dir, "../..");
function argument(name: string, optional = false): string | undefined {
  const index = process.argv.indexOf(name),
    value = process.argv[index + 1];
  if (index < 0 && optional) return undefined;
  if (index < 0 || !value || value.startsWith("--")) throw new Error(`${name} needs a path`);
  return resolve(value);
}
const frontend = argument("--frontend")!,
  compiler = argument("--compiler")!;
const table = argument("--table")!,
  out = argument("--out")!;
const kindIndex = process.argv.indexOf("--kind");
const kind = kindIndex >= 0 ? process.argv[kindIndex + 1] : "inspector";
if (kind !== "inspector" && kind !== "application") throw new Error("--kind must be inspector or application");
if (kind === "application" && !process.argv.includes("--bundle"))
  throw new Error("Application experiments require an explicit --bundle");
const digest = (bytes: string | Uint8Array): string => createHash("sha256").update(bytes).digest("hex");
function run(command: string[], cwd = sdk): string {
  const child = Bun.spawnSync(command, { cwd, stdout: "pipe", stderr: "pipe" });
  const stdout = child.stdout.toString(),
    stderr = child.stderr.toString();
  if (child.exitCode) throw new Error(`${command[0]} failed (${child.exitCode})\n${stdout}\n${stderr}`);
  if (stderr.trim()) console.error(stderr.trim());
  return stdout;
}
verifyEngine(sdk, PIN);
await verifyFile(frontend, PIN.frontend.executableSha256);
await verifyTable(table, sdk, PIN);
if (!run([frontend, "--revision"]).trim().endsWith(FRONTEND.slice(0, 9)))
  throw new Error(`Frontend must be Bun ${FRONTEND}`);
const info = JSON.parse(run([compiler, "info"]));
verifyCompilerInfo(info, PIN);
mkdirSync(out, { recursive: true });
const stage = mkdtempSync(join(out, ".native-stage-"));
const bundle = argument("--bundle", true) ?? join(stage, "app.prod.js");
if (!process.argv.includes("--bundle")) {
  run(
    [
      process.execPath,
      join(sdk, "packages/vue-native/prod.ts"),
      join(sdk, "apps/devtools/src/main.ts"),
      bundle,
      "--aot-inputs",
      join(stage, "typed-inputs"),
    ],
    join(sdk, "packages/vue-native"),
  );
}
const source = readFileSync(bundle, "utf8");
interface BuiltinEntry {
  holder: string;
  origin: string;
  isSymbolName: boolean;
  name: string;
}
const dump = JSON.parse(readFileSync(join(table, "aot_builtin_table.json"), "utf8")) as { entries: BuiltinEntry[] };
// Keep this host's APIs as unknown JS values. Bun must not assume that our
// URL/fetch/WebSocket/Blob implementations have Bun-native layouts or helpers.
const names = [
  ...new Set<string>([
    ...dump.entries
      .filter(
        (entry) =>
          entry.holder === "GlobalThis" &&
          entry.origin === "Host" &&
          !entry.isSymbolName &&
          /^[A-Za-z_$][\w$]*$/.test(entry.name),
      )
      .map((entry) => entry.name),
    "Object",
    "console",
    "window",
    "self",
    "document",
    "navigator",
    "location",
    "performance",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "ResizeObserver",
    "MutationObserver",
    "IntersectionObserver",
    "Image",
    "Audio",
    "AudioContext",
    "OffscreenCanvas",
    "Path2D",
    "ImageData",
    "createImageBitmap",
    "localStorage",
    "sessionStorage",
    "indexedDB",
    "matchMedia",
    "getComputedStyle",
  ]),
].sort();
const wrapper = `// ${TRANSFORM}: stable host API references are opaque inputs.\n(function(__vn_host) {\nconst globalThis = __vn_host;\nconst {${names.join(",")}} = __vn_host;\n${source}\n})(__VN_AOT_HOST__);\n`;
const input = join(stage, "devtools.native-input.js"),
  ir = join(stage, "devtools.ir");
writeFileSync(input, wrapper);
writeFileSync(join(stage, "opaque-host-globals.json"), JSON.stringify(names, null, 2));
run([
  frontend,
  "compile",
  input,
  "--no-builtin-modules",
  `--aot-out=${ir}`,
  "--aot-threads=1",
  "--aot-testing=NoExecutable",
  "--outfile",
  join(stage, "unused.exe"),
]);
if (!existsSync(ir)) throw new Error("Frontend emitted no native IR");
const image = join(stage, "devtools.image");
console.log(run([compiler, "compile", ir, image]).trim());
const bytes = readFileSync(image),
  imageSha256 = digest(bytes);
const entries = readFileSync(`${image}.entries`, "utf8").trim().split(/\s+/).map(Number);
if (entries.length !== 1 || !Number.isSafeInteger(entries[0]) || entries[0]! < 1)
  throw new Error("Expected one native entry");
const identity = readFileSync(`${image}.identity`);
if (identity.length !== 16) throw new Error("Invalid compiler image identity");
const prelude = Buffer.concat([
  readFileSync(join(sdk, "crates/vn-app/src/prelude.js")),
  Buffer.from([0]),
  readFileSync(join(sdk, "crates/vn-app/src/window.prelude.js")),
]);
const imageName = `${kind}.${imageSha256.slice(0, 16)}.image`;
copyFileSync(image, join(out, imageName));
if (resolve(bundle) !== resolve(out, "app.prod.js")) copyFileSync(bundle, join(out, "app.prod.js"));
const windowConfig = join(resolve(bundle, ".."), "window.json");
if (existsSync(windowConfig) && resolve(windowConfig) !== resolve(out, "window.json"))
  copyFileSync(windowConfig, join(out, "window.json"));
writeFileSync(
  join(out, `${kind}.native.json`),
  JSON.stringify(
    {
      schema: "vue-native.native-aot",
      version: 1,
      stage: "native-image",
      kind,
      ...info,
      image: imageName,
      imageSha256,
      entry: entries[0],
      identity: identity.toString("hex"),
      transform: TRANSFORM,
      sourceBundleSha256: digest(source),
      preludeSha256: digest(prelude),
      nativeInputSha256: digest(wrapper),
      irSha256: digest(readFileSync(ir)),
      sourceStage: "production-javascript", // typed source archive is retained; this image uses generated JS.
    },
    null,
    2,
  ),
);
console.log(`Native ${kind} artifact: ${join(out, `${kind}.native.json`)} (${bytes.length} bytes)`);
if (kind === "inspector")
  console.log(`Build the inspector with VN_DEVTOOLS_BUNDLE=${join(out, "app.prod.js")} to match this artifact.`);
else console.log("Application artifact only: a dedicated host boot integration is required; this was not executed.");
