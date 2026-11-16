// The reviewed native-AOT toolchain: the Bun revision sk-aot's frontend is built from, the engine
// revision and the builtin table id that sk-aot must report. Reading it runs nothing.
import { createHash } from "node:crypto";
import { createReadStream, readFileSync } from "node:fs";
import { join } from "node:path";

export const PIN_PATH = join(import.meta.dir, "toolchain.lock.json");

export interface ToolchainPin {
  schema: string;
  version: number;
  target: string;
  frontend: { revision: string; sourceRepository: string };
  engine: { revision: string; archiveSha256: string };
  table: { id: string; dumpSha256: string };
}

/** What `sk-aot info` prints, and what a host's `--sk-host-info` `aot` names. */
export interface CompilerInfo {
  target: string;
  frontend: string;
  engine: string;
  tableId: string;
  compileOptions: string;
}

export const COMPILER_FIELDS = ["target", "frontend", "engine", "tableId", "compileOptions"] as const;

export function loadPin(path: string = PIN_PATH): ToolchainPin {
  const pin = JSON.parse(readFileSync(path, "utf8").replace(/^﻿/, "")) as ToolchainPin;
  if (pin.schema !== "skathi.native-aot-toolchain" || pin.version !== 1 || pin.target !== "x86_64-windows")
    throw Error("Unsupported native AOT toolchain pin");
  for (const revision of [pin.frontend?.revision, pin.engine?.revision])
    if (!/^[a-f0-9]{40}$/.test(revision || "")) throw Error("Invalid native AOT revision");
  for (const digest of [pin.engine?.archiveSha256, pin.table?.id, pin.table?.dumpSha256])
    if (!/^[a-f0-9]{64}$/.test(digest || "")) throw Error("Invalid native AOT digest");
  if (pin.frontend.sourceRepository !== "https://github.com/oven-sh/bun.git")
    throw Error("The AOT frontend must be built from the official Bun repository");
  return pin;
}

export function pinnedCompilerInfo(pin: ToolchainPin): CompilerInfo {
  return {
    target: pin.target,
    frontend: pin.frontend.revision,
    engine: pin.engine.revision,
    tableId: pin.table.id,
    compileOptions: `x86_64;references=narrow;image=pic;threads=1;default-options@${pin.engine.revision}`,
  };
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

export function verifyCompilerInfo(info: Record<string, unknown>, pin: ToolchainPin): void {
  for (const [name, value] of Object.entries(pinnedCompilerInfo(pin)))
    if (info[name] !== value) throw Error(`Native AOT compiler ${name} differs from the reviewed toolchain`);
}
