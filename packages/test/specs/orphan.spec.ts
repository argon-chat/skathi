import { afterAll, describe, expect, test } from "bun:test";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { rmSync } from "node:fs";
import { join } from "node:path";
import { findOrphans, killTree, reapOrphans, REPO } from "../src/index";

const T = 120_000;
const RUNNER = join(import.meta.dir, "_orphan-runner.ts");

type Run = { child: ChildProcess; runner: number; host: number; slot: string };

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function alive(pid: number): boolean {
  const r = spawnSync("tasklist", ["/FI", `PID eq ${pid}`, "/FO", "CSV", "/NH"], { encoding: "utf8" });
  return new RegExp(`^"[^"]*","${pid}"`, "m").test(r.stdout ?? "");
}

/**
 * `TerminateProcess` on exactly this runner — no `/T`, or the tree kill would answer the question for
 * us.
 */
function terminate(run: Run) {
  run.child.kill();
}

async function goneWithin(pid: number, ms: number): Promise<boolean> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (!alive(pid)) return true;
    await sleep(100);
  }
  return !alive(pid);
}

const runs: Run[] = [];
afterAll(() => {
  // Most of these ended inside their test, and a pid is recycled: only what is still ours goes.
  for (const r of runs) {
    killTree(r.child);
    killTree(r.host, `sk-app-sktest-${r.runner}.exe`);
    try {
      rmSync(r.slot, { recursive: true, force: true });
    } catch {}
  }
});

async function startRunner(keep: boolean): Promise<Run> {
  const child = spawn(process.execPath, [RUNNER, ...(keep ? ["keep"] : [])], {
    cwd: REPO,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let out = "";
  let err = "";
  child.stdout!.on("data", (d) => (out += d));
  child.stderr!.on("data", (d) => (err += d));
  const deadline = Date.now() + 90_000;
  for (;;) {
    const m = /RUNNER (\d+) HOST (\d+) SLOT (.+)/.exec(out);
    if (m) {
      const run = { child, runner: Number(m[1]), host: Number(m[2]), slot: m[3]!.trim() };
      runs.push(run);
      return run;
    }
    if (child.exitCode !== null || Date.now() > deadline) {
      throw new Error(`the runner never reported its host\n--- stdout\n${out}\n--- stderr\n${err}`);
    }
    await sleep(100);
  }
}

describe.skipIf(process.platform !== "win32")("orphans", () => {
  test(
    "a host whose runner is killed outright goes with it",
    async () => {
      const run = await startRunner(false);
      expect(alive(run.host)).toBe(true);
      terminate(run);
      expect(await goneWithin(run.host, 5_000)).toBe(true);
    },
    T,
  );

  test(
    "the reap takes an orphan and leaves a live run's host alone",
    async () => {
      // The live run first: its own `launch()` reaps at startup, and it must not get to the orphan
      // before this test has looked at it.
      const live = await startRunner(false);
      const orphan = await startRunner(true);

      terminate(orphan);
      await sleep(1_500);
      expect(alive(orphan.host)).toBe(true);

      const found = findOrphans().map((o) => o.pid);
      expect(found).toContain(orphan.host);
      expect(found).not.toContain(live.host);

      reapOrphans();
      expect(await goneWithin(orphan.host, 5_000)).toBe(true);
      expect(alive(live.host)).toBe(true);

      terminate(live);
      expect(await goneWithin(live.host, 5_000)).toBe(true);
    },
    T,
  );
});
