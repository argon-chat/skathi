import { expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { EXE_SUFFIX, REPO, TARGET, WINDOWS } from "../src/index";

const EXE = join(REPO, "target", TARGET, "debug", "examples", `embed${EXE_SUFFIX}`);
const PAYLOAD = join(REPO, "packages", "test", "fixtures", "embed", "dist", "debug", ".sk", "app.skpk");

function run(cmd: string, args: string[], timeout: number) {
  const r = spawnSync(cmd, args, { cwd: REPO, encoding: "utf8", timeout, windowsHide: true });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (${r.status ?? r.signal}):\n${r.stderr}`);
  return r;
}

// macOS: AppKit refuses the example's app off the main thread; Linux: the example does not find its payload.
test.skipIf(!WINDOWS)(
  "an embedded app calls its native module's functions and tasks, hears its events, and closes without ending the process",
  () => {
    run("cargo", ["build", "-p", "sk-app", "--target", TARGET, "--example", "embed", "--features", "prod"], 900_000);
    run("bun", ["packages/cli/cli/sk.ts", "build", "packages/test/fixtures/embed", "--payload-only"], 120_000);
    expect(existsSync(EXE)).toBe(true);

    const r = spawnSync(EXE, [PAYLOAD, "--self-test"], {
      cwd: REPO,
      encoding: "utf8",
      timeout: 90_000,
      windowsHide: true,
      // sk-test's quiet mode: the window opens behind and never takes the foreground
      // (crates/sk-app/src/quiet.rs).
      env: { ...process.env, SK_WINDOW_FOCUS: "never" },
    });
    expect(r.status, r.stderr).toBe(0);
    const out = JSON.parse(r.stdout.trim().split("\n").pop()!);
    const first = out.first;

    expect(first.available).toBe(true);
    expect(first.point).toEqual({ ok: { x: 2, y: 3 } });
    expect(first.refuse).toEqual({ error: "Error", message: "no such luck" });
    expect(first.badArgument.error).toBe("TypeError");
    // A function runs inside the call; a task on a worker, settling a promise.
    expect(first.uiThread).toBe("skathi");
    expect(first.worker.ok).toMatch(/^skathi-task-\d+$/);
    expect(first.double).toEqual({ ok: 42 });
    expect(first.doubleZero).toMatchObject({ error: "Error", message: "zero has no double worth computing" });
    expect(first.crash.error).toBe("Error");
    expect(first.legacy).toEqual({ ok: { pong: "hello" } });
    expect(out.second).toEqual({ tick: { n: 1 } });
    expect(out.third).toEqual({ legacy: { n: 2 } });
    expect(out.undeclared).not.toBeNull();
    expect(out.exit.clean).toBe(true);
    expect(out.exit.summary).toMatch(/^close\(\): hid 1 window\(s\)/);
    expect(out.emitAfterClose).toBe("the app has closed");
    expect(r.stderr).toContain("payload from the embedder");
  },
  1_100_000,
);

test.skipIf(!WINDOWS)("sk types writes the declarations the embedding program gives for its native modules", () => {
  // Built by the test above.
  expect(existsSync(EXE)).toBe(true);
  const declared = run(EXE, ["--sk-host-types"], 60_000).stdout.replace(/\r\n/g, "\n");
  expect(declared).toContain('declare module "native:demo"');
  expect(declared).toContain("export function point(x: number, y: number): Point;");
  expect(declared).toContain("export function double(n: number): Promise<number>;");
  expect(declared).toContain('export function on(event: "tick", handler: (data: Tick) => void): () => void;');
  run("bun", ["packages/cli/cli/sk.ts", "types", "packages/test/fixtures/embed", "--exe", EXE], 120_000);
  expect(readFileSync(join(REPO, "packages", "test", "fixtures", "embed", "src", "native-modules.d.ts"), "utf8")).toBe(
    declared,
  );
});
