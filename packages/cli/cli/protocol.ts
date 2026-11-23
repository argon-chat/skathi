// The runtime protocol id: FNV-1a over the sorted `__sk_` names the runtime's sources mention.
// sk-app's build.rs computes it the same way over the same directory, so a host and a bundle made
// from different runtime sources disagree without anyone maintaining a number. `sk build` and the
// dev builder record the runtime's id beside the bundle; the host compares it with its own before
// it runs the app. An id missing on either side is unknown and blocks nothing.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { RUNTIME_SRC_DIR, RUNTIME_VERSION } from "../runtime-path";

const NAME = /(?<![\w$])__sk_[\w$]*/g;

function collect(dir: string, names: Set<string>): void {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry.startsWith(".")) continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) collect(path, names);
    else for (const m of readFileSync(path, "latin1").matchAll(NAME)) names.add(m[0]);
  }
}

export function protocolOf(srcDir: string): string {
  const names = new Set<string>();
  collect(srcDir, names);
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode([...names].sort().join("\n"))) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

let runtimeProtocol: string | null = null;
/** The protocol of the runtime this CLI bundles into apps. */
export function runtimeProtocolId(): string {
  return (runtimeProtocol ??= protocolOf(RUNTIME_SRC_DIR));
}

/** What a payload or dev manifest records as `"runtime"`, for the host's startup check. */
export const runtimeStamp = () => ({ version: RUNTIME_VERSION, protocol: runtimeProtocolId() });

/** Why the host whose `--sk-host-info` reported `info` cannot run apps built with this CLI's runtime; null when it can or nobody knows. */
export function protocolMismatch(exe: string, info: { skathi: string; protocol?: string | undefined }): string | null {
  const id = runtimeProtocolId();
  if (!info.protocol || info.protocol === id) return null;
  return (
    `${exe} is Skathi ${info.skathi} with runtime protocol ${info.protocol}, and the apps it would run are built ` +
    `against @skathi-native/runtime ${RUNTIME_VERSION} (protocol ${id}): the host is out of date with the runtime — rebuild it`
  );
}
