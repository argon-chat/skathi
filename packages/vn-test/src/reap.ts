// Reaping the hosts a PREVIOUS run left behind.
//
// `VnApp.killSync()` runs from three places, and all three are inside the bun
// runner — so a runner that is itself killed (a task manager, an agent stopped
// mid-run, a crash inside bun) takes all of them down with it and its hosts
// live on. 35 of them were found on the owner's machine once (2026-09-18).
//
// Two layers close that, and this file is the second:
//
//   1. `VN_OWNER_PID` (crates/vn-app/src/owner.rs): every host `launch()`
//      starts holds a handle on its runner and terminates itself when the
//      runner exits, for any reason. That is the fix; it needs nothing to run.
//   2. This sweep, for whatever the first layer cannot reach — a host built
//      before the watch existed, or one whose watch could not open its owner.
//      It runs once per runner, before the exe is staged, and on demand:
//
//        bun packages/vn-test/src/reap.ts
//
// No registry on disk is needed, because the staged executable already says
// who owns it: `vn-app-vntest-<runner pid>.exe` (see `stagedExe`), and an
// inspector a host opened with F12 re-spawns that same file. A process with
// that name whose runner is not a live bun process is an orphan. A live bun at
// that pid is a concurrent run — someone else's hosts — and is left alone.

import { spawnSync } from "node:child_process";

// `-<label>` is a packaged app (`packagedExe` in features.ts): same owner, same rule.
const HOST_IMAGE = /^vn-app-vntest-(\d+)(?:-[\w.-]+)?\.exe$/i;
const RUNNER_IMAGE = /^bun(-[\w.]+)?\.exe$/i;

export type Orphan = { pid: number; image: string; runner: number };

/** Every process on the machine as `pid -> image name`, or `null` if `tasklist` failed. */
function processes(): Map<number, string> | null {
  const r = spawnSync("tasklist", ["/FO", "CSV", "/NH"], { encoding: "utf8", windowsHide: true });
  if (r.status !== 0 || !r.stdout) return null;
  const out = new Map<number, string>();
  for (const line of r.stdout.split(/\r?\n/)) {
    const m = /^"([^"]*)","(\d+)"/.exec(line);
    if (m) out.set(Number(m[2]), m[1]!);
  }
  return out;
}

/** The `vn-app-vntest-*` processes whose runner is gone. Never this runner's own. */
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
    // A pid is recycled freely on Windows, so "something is alive at that pid"
    // is not enough: it has to be a bun to count as the run that owns this host.
    const at = procs.get(runner);
    if (at && RUNNER_IMAGE.test(at)) continue;
    orphans.push({ pid, image, runner });
  }
  return orphans;
}

/**
 * Kills every orphan `findOrphans()` reports — the tree, so an inspector child
 * goes with its host — and returns what it found. Says so on stderr when it
 * finds anything, because an orphan means some earlier run died badly.
 */
export function reapOrphans(): Orphan[] {
  const orphans = findOrphans();
  for (const o of orphans) {
    spawnSync("taskkill", ["/PID", String(o.pid), "/T", "/F"], { stdio: "ignore", windowsHide: true });
  }
  if (orphans.length) {
    const list = orphans.map((o) => `${o.pid} (runner ${o.runner})`).join(", ");
    console.warn(
      `[vn-test] reaped ${orphans.length} host(s) left behind by a run that died before its teardown: ${list}`,
    );
  }
  return orphans;
}

if (import.meta.main) {
  const reaped = reapOrphans();
  if (!reaped.length) console.log("[vn-test] no orphaned hosts");
}
