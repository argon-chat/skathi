// The reviewed native-AOT toolchain: the frontend (an exact official @oven/bun-windows-x64 build), the
// engine revision and the builtin table id an image compiler must report. Reading it runs nothing.
import { createHash } from "node:crypto";
import { createReadStream, readFileSync } from "node:fs";
import { join } from "node:path";

export const PIN_PATH = join(import.meta.dir, "toolchain.lock.json");

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

/** What `aot-image info` prints, and what a host's `--sk-host-info` `aot` names. */
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

export function pinnedCompilerInfo(pin: ToolchainPin): CompilerInfo {
  return {
    target: pin.target,
    frontend: pin.frontend.revision,
    engine: pin.engine.revision,
    tableId: pin.table.id,
    compileOptions: `x86_64;references=narrow;image=pic;threads=1;admit=FilesRunByHost;default-options@${pin.engine.revision}`,
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
