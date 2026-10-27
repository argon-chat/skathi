// Teardown that survives the runner: the owner watch, and the reap.
//
// Every kill path `VnApp` has — `close()`, the `process.exit` hook, the signal
// handlers — runs inside the bun runner, so a runner killed outright takes all
// of them with it. That is how 35 orphaned hosts once piled up on the owner's
// machine. This file kills a runner the way a task manager does
// (`TerminateProcess`, no `/T`, so the tree kill cannot do the work for us) and
// asserts on what is left:
//
//   * a host launched normally carries `VN_OWNER_PID` and goes by itself
//     (crates/vn-app/src/owner.rs);
//   * a host the watch cannot reach — `keepAlive` stands in for one — is found
//     by `findOrphans()` and killed by `reapOrphans()`, while a host belonging
//     to a run that is still ALIVE is left exactly where it is.
//
// The runner is `_orphan-runner.ts`: one `launch()`, its pids on stdout, and a
// hang.

import { afterAll, describe, expect, test } from "bun:test";
import { spawn, spawnSync } from "node:child_process";
import { rmSync } from "node:fs";
import { join } from "node:path";
import { findOrphans, reapOrphans, REPO } from "../src/index";

const T = 120_000;
const RUNNER = join(import.meta.dir, "_orphan-runner.ts");

type Run = { runner: number; host: number; slot: string };

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** `tasklist` rather than `process.kill(pid, 0)`: the answer has to be the OS's. */
function alive(pid: number): boolean {
  const r = spawnSync("tasklist", ["/FI", `PID eq ${pid}`, "/FO", "CSV", "/NH"], { encoding: "utf8" });
  return new RegExp(`^"[^"]*","${pid}"`, "m").test(r.stdout ?? "");
}

/** `TerminateProcess` on exactly this pid — no `/T`, or the tree kill would answer the question for us. */
function terminate(pid: number) {
  spawnSync("taskkill", ["/PID", String(pid), "/F"], { stdio: "ignore" });
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
  for (const r of runs) {
    terminate(r.runner);
    spawnSync("taskkill", ["/PID", String(r.host), "/T", "/F"], { stdio: "ignore" });
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
      const run = { runner: Number(m[1]), host: Number(m[2]), slot: m[3]!.trim() };
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
  test("a host whose runner is killed outright goes with it", async () => {
    const run = await startRunner(false);
    expect(alive(run.host)).toBe(true);
    terminate(run.runner);
    // The watch blocks on a process handle, so this is milliseconds, not a poll period.
    expect(await goneWithin(run.host, 5_000)).toBe(true);
  }, T);

  test("the reap takes an orphan and leaves a live run's host alone", async () => {
    // The live run first: its own `launch()` reaps at startup, and it must not
    // get to the orphan before this test has looked at it.
    const live = await startRunner(false);
    const orphan = await startRunner(true);

    terminate(orphan.runner);
    await sleep(1_500);
    // `keepAlive` carries no owner watch, so this host has outlived its runner
    // — the exact state the reap exists for.
    expect(alive(orphan.host)).toBe(true);

    const found = findOrphans().map((o) => o.pid);
    expect(found).toContain(orphan.host);
    expect(found).not.toContain(live.host);

    reapOrphans();
    expect(await goneWithin(orphan.host, 5_000)).toBe(true);
    expect(alive(live.host)).toBe(true);

    terminate(live.runner);
    expect(await goneWithin(live.host, 5_000)).toBe(true);
  }, T);
});
