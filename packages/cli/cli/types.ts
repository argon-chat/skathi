// `sk types [app] --exe <program>`: <app>/src/native-modules.d.ts from `<program> --sk-host-types`,
// the `native:*` modules a program built with the skathi crate registers. `sk dev --exe` does the same on start.

import { spawnSync } from "node:child_process";
import { closeSync, existsSync, openSync, readFileSync, readSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { loadAppConfig } from "./config";

export const NATIVE_TYPES_FILE = "native-modules.d.ts";
const FLAG = "--sk-host-types";

export class TypesError extends Error {}

/** A program built with an older skathi has no such flag and would start its app instead, so look before running it. */
export function answersHostTypes(exe: string): boolean {
  const needle = Buffer.from(FLAG);
  const chunk = Buffer.alloc(4 << 20);
  const fd = openSync(exe, "r");
  try {
    let carry = Buffer.alloc(0);
    for (;;) {
      const n = readSync(fd, chunk, 0, chunk.length, null);
      if (n <= 0) return false;
      const window = Buffer.concat([carry, chunk.subarray(0, n)]);
      if (window.includes(needle)) return true;
      carry = window.subarray(Math.max(0, window.length - needle.length + 1));
    }
  } finally {
    closeSync(fd);
  }
}

export type NativeTypes = { status: "written" | "unchanged"; path: string } | { status: "none" | "unsupported" };

/** Writes the declarations beside the app's entry, only when they changed. */
export function writeNativeTypes(exe: string, srcDir: string): NativeTypes {
  if (!existsSync(exe)) throw new TypesError(`${exe}: no such program`);
  if (!answersHostTypes(exe)) return { status: "unsupported" };
  const ran = spawnSync(exe, [FLAG], {
    encoding: "utf8",
    timeout: 60_000,
    windowsHide: true,
    // The program dies with this process however it ends.
    env: { ...process.env, SK_OWNER_PID: String(process.pid) },
  });
  if (ran.status !== 0) {
    throw new TypesError(
      `${exe} ${FLAG} failed (${ran.status ?? ran.signal ?? ran.error?.message}): ${(ran.stderr ?? "").trim()}`,
    );
  }
  const text = ran.stdout.replace(/\r\n/g, "\n");
  if (!text.includes('declare module "native:')) return { status: "none" };
  const path = join(srcDir, NATIVE_TYPES_FILE);
  if (existsSync(path) && readFileSync(path, "utf8") === text) return { status: "unchanged", path };
  writeFileSync(path, text);
  return { status: "written", path };
}

export function typesCommand(target: string, exe: string): number {
  const config = loadAppConfig(target);
  const result = writeNativeTypes(exe, dirname(config.entry));
  switch (result.status) {
    case "unsupported":
      throw new TypesError(`${exe} does not answer ${FLAG}: build it against skathi 0.4 or later`);
    case "none":
      console.log(`${exe} registers no native modules`);
      return 0;
    default:
      console.log(`${result.status === "written" ? "wrote" : "unchanged"} ${result.path}`);
      return 0;
  }
}
