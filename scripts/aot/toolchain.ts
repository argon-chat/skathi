// Trusted build inputs for native inspector images. Importing this module does
// not download tools, generate code or execute a compiler.
import { createHash } from "node:crypto";
import { createReadStream, existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
export interface ToolchainPin {
  schema: string;
  version: number;
  target: string;
  frontend: {
    revision: string;
    archiveUrl: string;
    archiveFormat: string;
    packageVersion: string;
    archiveSha256: string;
    executableSha256: string;
    sourceRepository: string;
  };
  engine: { revision: string; archiveSha256: string };
  table: { id: string; dumpSha256: string };
}
// CRLF is part of the already-qualified table identity, independent of the
// source checkout's line endings. Keep it explicit so clean CI regenerates
// the same ABI identifier as the native compiler and existing images.
export function canonicalTableReceipt(frontend: string, engine: string): string {
  return `redjs.aot-builtin-table.v1\r\nfrontend=${frontend}\r\nwebkit=${engine}\r\n`;
}
export const TABLE_FILES = [
  ".redjs-aot-table",
  "AOTBuiltinTableArrays.h",
  "AOTBuiltinTableAttachments.cpp",
  "AOTBuiltinTableBuiltins.cpp",
  "AOTBuiltinTableNames.cpp",
  "AOTBuiltinTableOverloads.cpp",
  "AOTBuiltinTableTypes.cpp",
] as const;
export function loadPin(path: string): ToolchainPin {
  const pin = JSON.parse(readFileSync(path, "utf8").replace(/^\uFEFF/, "")) as ToolchainPin;
  if (pin.schema !== "skathi.native-aot-toolchain" || pin.version !== 1 || pin.target !== "x86_64-windows")
    throw Error("Unsupported native AOT toolchain pin");
  for (const revision of [pin.frontend?.revision, pin.engine?.revision])
    if (!/^[a-f0-9]{40}$/.test(revision || "")) throw Error("Invalid native AOT revision");
  for (const digest of [
    pin.frontend?.archiveSha256,
    pin.frontend?.executableSha256,
    pin.engine?.archiveSha256,
    pin.table?.id,
    pin.table?.dumpSha256,
  ])
    if (!/^[a-f0-9]{64}$/.test(digest || "")) throw Error("Invalid native AOT digest");
  const version = pin.frontend.packageVersion;
  if (
    pin.frontend.sourceRepository !== "https://github.com/oven-sh/bun.git" ||
    pin.frontend.archiveFormat !== "npm-tgz" ||
    !/^\d+\.\d+\.\d+-canary\.\d{8}\.\d+$/.test(version || "") ||
    pin.frontend.archiveUrl !== `https://registry.npmjs.org/@oven/bun-windows-x64/-/bun-windows-x64-${version}.tgz`
  ) {
    throw Error("AOT frontend must come from an exact official @oven package and pinned source repository");
  }
  return pin;
}
export async function fileDigest(path: string): Promise<string> {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path, { highWaterMark: 64 * 1024 })) hash.update(chunk);
  return hash.digest("hex");
}
export async function verifyFile(path: string, expected: string): Promise<void> {
  const actual = await fileDigest(path);
  if (actual !== expected)
    throw Error(`Native AOT input digest mismatch: ${path}; expected ${expected}, got ${actual}`);
}
export function verifyEngine(sdk: string, pin: ToolchainPin): void {
  const pins = readFileSync(join(sdk, "vendor/redjs/pins.txt"), "utf8");
  const value = (name: string) => new RegExp(`^${name.replace(/\./g, "\\.")}\\s*=\\s*([^\\s#]+)`, "m").exec(pins)?.[1];
  if (
    value("webkit.version") !== pin.engine.revision ||
    value("webkit.sha256") !== pin.engine.archiveSha256 ||
    value("webkit.aot") !== "1"
  )
    throw Error("Review the native AOT frontend, table and redjs engine pins together");
  const configuration = readFileSync(join(sdk, "vendor/redjs/prebuilt/bun-webkit/include/cmakeconfig.h"), "utf8");
  if (!configuration.includes(pin.engine.revision) || !/^#define BUN_WEBKIT_ENABLE_AOT 1\r?$/m.test(configuration))
    throw Error("Run the pinned redjs setup before preparing a native inspector");
}
export async function verifyTable(directory: string, sdk: string, pin: ToolchainPin): Promise<void> {
  const receipt = readFileSync(join(directory, ".redjs-aot-table"), "utf8");
  if (!receipt.includes(`frontend=${pin.frontend.revision}`) || !receipt.includes(`webkit=${pin.engine.revision}`))
    throw Error("Native AOT table generation receipt differs from the pinned toolchain");
  const hash = createHash("sha256");
  for (const name of TABLE_FILES) {
    hash.update(name);
    for await (const chunk of createReadStream(join(directory, name), { highWaterMark: 64 * 1024 })) hash.update(chunk);
  }
  for await (const chunk of createReadStream(
    join(sdk, "vendor/redjs/prebuilt/bun-webkit/include/JavaScriptCore/aot/AOTWire.cpp"),
    { highWaterMark: 64 * 1024 },
  ))
    hash.update(chunk);
  if (hash.digest("hex") !== pin.table.id)
    throw Error("Generated native AOT table/schema differs from the reviewed pin");
  await verifyFile(join(directory, "aot_builtin_table.json"), pin.table.dumpSha256);
}
export function verifyCompilerInfo(info: Record<string, unknown>, pin: ToolchainPin): void {
  const expected = {
    target: pin.target,
    frontend: pin.frontend.revision,
    engine: pin.engine.revision,
    tableId: pin.table.id,
    compileOptions: `x86_64;references=narrow;image=pic;threads=1;default-options@${pin.engine.revision}`,
  };
  for (const [name, value] of Object.entries(expected))
    if (info[name] !== value) throw Error(`Native AOT compiler ${name} differs from the reviewed toolchain`);
}
export function needsTable(directory: string): boolean {
  return !existsSync(join(directory, ".redjs-aot-table"));
}
