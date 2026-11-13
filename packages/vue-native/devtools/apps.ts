/**
 * Finding a PACKAGED app's devtools handshake (`--app <name>`): a packaged exe publishes
 * `%TEMP%\vue-native\apps\<vn.config name>\<pid>\devtools.json`; `--app hello` picks the newest
 * one whose process is still alive. Zero dependencies, like `ctl.ts`.
 */

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const appsRoot = () => join(tmpdir(), "vue-native", "apps");

export function slotName(name: string): string {
  const cleaned = [...name]
    .map((c) => (/[A-Za-z0-9._-]/.test(c) ? c : "-"))
    .slice(0, 64)
    .join("")
    .replace(/^[-.]+|[-.]+$/g, "");
  return cleaned || "vn-host";
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    // EPERM: it exists, it just is not ours to signal.
    return (e as NodeJS.ErrnoException).code === "EPERM";
  }
}

export interface AppSlot {
  dir: string;
  pid: number;
  port: number;
  at: number;
}

export function appSlots(name: string): AppSlot[] {
  const root = join(appsRoot(), slotName(name));
  if (!existsSync(root)) return [];
  const out: AppSlot[] = [];
  for (const leaf of readdirSync(root)) {
    const dir = join(root, leaf);
    const file = join(dir, "devtools.json");
    try {
      const h = JSON.parse(readFileSync(file, "utf8")) as { port?: number; pid?: number };
      const pid = Number(h.pid ?? leaf);
      if (!h.port || !alive(pid)) continue;
      out.push({ dir, pid, port: h.port, at: statSync(file).mtimeMs });
    } catch {
      // no handshake (devtools off, or still booting) — not attachable
    }
  }
  return out.sort((a, b) => b.at - a.at);
}

export function appSlot(name: string): { dir: string } | { error: string } {
  const slots = appSlots(name);
  if (slots.length) return { dir: slots[0]!.dir };
  const root = join(appsRoot(), slotName(name));
  return {
    error:
      `no running packaged app "${name}" with its devtools on (looked in ${root}). ` +
      `Start the exe; an app built on the no-devtools host, or one that called appDevtools.disable(), ` +
      `publishes no handshake.`,
  };
}
