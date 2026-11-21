// Trusted build inputs for native AOT images. Importing this module does
// not download tools, generate code or execute a compiler.
import { createHash } from "node:crypto";
import { createReadStream, existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { ToolchainPin } from "../../packages/cli/aot/pin";
export {
  fileDigest,
  loadPin,
  PIN_PATH,
  verifyCompilerInfo,
  verifyFile,
  type ToolchainPin,
} from "../../packages/cli/aot/pin";
import { verifyFile } from "../../packages/cli/aot/pin";

export function webkitDirectory(sdk: string): string {
  const override = process.env.BUN_WEBKIT_DIR;
  if (override) return override;
  const platform = process.platform === "darwin" ? "macos" : process.platform;
  const architecture = process.arch === "x64" ? "amd64" : process.arch;
  const prebuilt = platform === "win32" ? "bun-webkit" : `bun-webkit-${platform}-${architecture}`;
  return join(sdk, "vendor/redjs/prebuilt", prebuilt);
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
export function verifyEngine(sdk: string, pin: ToolchainPin): void {
  const pins = readFileSync(join(sdk, "vendor/redjs/redjs/pins.txt"), "utf8");
  const value = (name: string) => new RegExp(`^${name.replace(/\./g, "\\.")}\\s*=\\s*([^\\s#]+)`, "m").exec(pins)?.[1];
  if (
    value("webkit.version") !== pin.engine.revision ||
    value("webkit.sha256") !== pin.engine.archiveSha256 ||
    value("webkit.aot") !== "1"
  )
    throw Error("Review the native AOT frontend, table and redjs engine pins together");
  const configuration = readFileSync(join(webkitDirectory(sdk), "include/cmakeconfig.h"), "utf8");
  if (!configuration.includes(pin.engine.revision) || !/^#define BUN_WEBKIT_ENABLE_AOT 1\r?$/m.test(configuration))
    throw Error("Run the pinned redjs setup before preparing the native AOT toolchain");
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
  for await (const chunk of createReadStream(join(webkitDirectory(sdk), "include/JavaScriptCore/aot/AOTWire.cpp"), {
    highWaterMark: 64 * 1024,
  }))
    hash.update(chunk);
  if (hash.digest("hex") !== pin.table.id)
    throw Error("Generated native AOT table/schema differs from the reviewed pin");
  await verifyFile(join(directory, "aot_builtin_table.json"), pin.table.dumpSha256);
}
export function needsTable(directory: string): boolean {
  return !existsSync(join(directory, ".redjs-aot-table"));
}
