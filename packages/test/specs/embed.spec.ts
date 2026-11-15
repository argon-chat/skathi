import { expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { REPO } from "../src/index";

const EXE = join(REPO, "target", "x86_64-pc-windows-msvc", "debug", "examples", "embed.exe");
const PAYLOAD = join(REPO, "packages", "test", "fixtures", "embed", "dist", ".sk", "app.skpk");

function run(cmd: string, args: string[], timeout: number) {
  const r = spawnSync(cmd, args, { cwd: REPO, encoding: "utf8", timeout, windowsHide: true });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (${r.status ?? r.signal}):\n${r.stderr}`);
  return r;
}

test("an embedded app calls its embedder, hears its events, and closes without ending the process", () => {
  run("cargo", ["build", "-p", "sk-app", "--example", "embed", "--features", "prod"], 900_000);
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

  expect(out.first.available).toBe(true);
  expect(out.first.functions).toEqual(["add", "fail", "ping", "report"]);
  expect(out.first.ping).toEqual({ ok: { pong: "hello" } });
  expect(out.first.add).toEqual({ ok: 5 });
  expect(out.first.fail).toEqual({ error: "AppHostError", message: "no such luck", function: "fail" });
  expect(out.first.missing.error).toBe("TypeError");
  expect(out.second).toEqual({ tick: { n: 1 } });
  expect(out.refusedNotJson).toBe("the event's data is not JSON");
  expect(out.exit.clean).toBe(true);
  expect(out.exit.summary).toMatch(/^close\(\): hid 1 window\(s\)/);
  expect(out.emitAfterClose).toBe("the app has closed");
  expect(r.stderr).toContain("payload from the embedder");
}, 1_100_000);
