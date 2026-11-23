// This sweep, for whatever the first layer cannot reach — a host built before the watch existed,
// or one whose watch could not open its owner.

import { spawnSync, type ChildProcess } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { WORK_DIR } from "./dist";

const HOST_IMAGE = /^sk-app-sktest-(\d+)(?:-[\w.-]+)?\.exe$/i;
const RUNNER_IMAGE = /^bun(-[\w.]+)?\.exe$/i;

export type Orphan = { pid: number; image: string; runner: number };

/** Every process on the machine as `pid -> image name`, or `null` if `tasklist` failed. */
function processes(filter?: string): Map<number, string> | null {
  const args = ["/FO", "CSV", "/NH", ...(filter ? ["/FI", filter] : [])];
  const r = spawnSync("tasklist", args, { encoding: "utf8", windowsHide: true });
  if (r.status !== 0 || !r.stdout) return null;
  const out = new Map<number, string>();
  for (const line of r.stdout.split(/\r?\n/)) {
    const m = /^"([^"]*)","(\d+)"/.exec(line);
    if (m) out.set(Number(m[2]), m[1]!);
  }
  return out;
}

/** The image name of the process at `pid` now, or `null` if none is there. */
export function imageOf(pid: number): string | null {
  if (process.platform !== "win32") return null;
  return processes(`PID eq ${pid}`)?.get(pid) ?? null;
}

/** The `sk-app-sktest-*` processes whose runner is gone. */
export function findOrphans(): Orphan[] {
  if (process.platform !== "win32") return [];
  const procs = processes();
  if (!procs) return [];
  const orphans: Orphan[] = [];
  for (const [pid, image] of procs) {
    const m = HOST_IMAGE.exec(image);
    if (!m) continue;
    const runner = Number(m[1]);
    if (runner === process.pid) continue;
    // A pid is recycled freely on Windows, so "something is alive at that pid" is not enough: it
    // has to be a bun to count as the run that owns this host.
    const at = procs.get(runner);
    if (at && RUNNER_IMAGE.test(at)) continue;
    orphans.push({ pid, image, runner });
  }
  return orphans;
}

/** Every kill a run makes, from every run in this checkout: what a host that died mid-test is checked against. */
export const killLog = () => join(WORK_DIR, "kills.log");

function noteKill(pid: number, image: string | null, why: string) {
  const path = killLog();
  try {
    mkdirSync(dirname(path), { recursive: true });
    try {
      if (statSync(path).size > 512 * 1024) {
        const text = readFileSync(path, "utf8");
        writeFileSync(path, text.slice(text.indexOf("\n", text.length / 2) + 1));
      }
    } catch {}
    appendFileSync(path, `${new Date().toISOString()} runner=${process.pid} pid=${pid} image=${image ?? "?"} ${why}\n`);
  } catch {}
}

/** The kills other runs made of `pid` since `since` (ms), as kill-log lines. */
export function killsOf(pid: number, since: number): string[] {
  let text: string;
  try {
    text = readFileSync(killLog(), "utf8");
  } catch {
    return [];
  }
  return text.split("\n").filter((line) => {
    const m = /^(\S+) runner=(\d+) pid=(\d+) image=\S+ (?!spared)/.exec(line);
    return !!m && Number(m[3]) === pid && Number(m[2]) !== process.pid && Date.parse(m[1]!) >= since - 1000;
  });
}

/** Has `child` been seen to exit? Until then its process handle is held, so its pid is still its own. */
export const hasExited = (child: ChildProcess) => child.exitCode !== null || child.signalCode !== null;

function killPid(pid: number, image: string | null, why: string) {
  if (process.platform === "win32") {
    noteKill(pid, image, why);
    spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore", windowsHide: true });
    return;
  }
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    try {
      process.kill(pid, "SIGKILL");
    } catch {}
  }
}

/**
 * Kills a process and its children: a host is spawned detached, so on unix it leads its own group.
 * A child that has already exited is left alone, since its pid may by now be another process's. A
 * bare pid is killed only while its image is `image` (Windows).
 */
export function killTree(target: ChildProcess | number, image?: RegExp | string) {
  if (typeof target !== "number") {
    if (target.pid && !hasExited(target)) killPid(target.pid, basename(target.spawnfile), "child");
    return;
  }
  const now = imageOf(target);
  if (image !== undefined && process.platform === "win32") {
    const matches = typeof image === "string" ? now?.toLowerCase() === image.toLowerCase() : !!now && image.test(now);
    if (!matches) {
      if (now) noteKill(target, now, `spared: not ${image}`);
      return;
    }
  }
  killPid(target, now, "pid");
}

/**
 * Kills every orphan `findOrphans()` reports — the tree, so an inspector child goes with its host
 * — and returns what it found.
 */
export function reapOrphans(): Orphan[] {
  const orphans = findOrphans();
  for (const o of orphans) {
    killPid(o.pid, o.image, `orphan of runner ${o.runner}`);
  }
  if (orphans.length) {
    const list = orphans.map((o) => `${o.pid} (runner ${o.runner})`).join(", ");
    console.warn(
      `[sk-test] reaped ${orphans.length} host(s) left behind by a run that died before its teardown: ${list}`,
    );
  }
  return orphans;
}

if (import.meta.main) {
  const reaped = reapOrphans();
  if (!reaped.length) console.log("[sk-test] no orphaned hosts");
}
