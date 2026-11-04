// vue-native as a library (docs/EMBEDDING.md): crates/vn-app/examples/embed.rs
// is a console program that runs the engine on a thread of its own, with
// fixtures/embed as its app. Its `--self-test` drives the whole conversation
// and prints one JSON line; this spec builds both halves, runs it quietly and
// reads that line.
//
// No `launch()`: the process is the embedder's, not a vn-app host, and it
// ends on its own once vue-native has closed — which is one of the claims.
import { expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { REPO } from "../src/index";

const EXE = join(REPO, "target", "x86_64-pc-windows-msvc", "debug", "examples", "embed.exe");
const PAYLOAD = join(REPO, "packages", "vn-test", "fixtures", "embed", "dist", ".vn", "app.vnpk");

function run(cmd: string, args: string[], timeout: number) {
  const r = spawnSync(cmd, args, { cwd: REPO, encoding: "utf8", timeout, windowsHide: true });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (${r.status ?? r.signal}):\n${r.stderr}`);
  return r;
}

test(
  "an embedded app calls its embedder, hears its events, and closes without ending the process",
  () => {
    run("cargo", ["build", "-p", "vn-app", "--example", "embed", "--features", "prod"], 900_000);
    run("bun", ["packages/vue-native/cli/vn.ts", "build", "packages/vn-test/fixtures/embed", "--payload-only"], 120_000);
    expect(existsSync(EXE)).toBe(true);

    const r = spawnSync(EXE, [PAYLOAD, "--self-test"], {
      cwd: REPO,
      encoding: "utf8",
      timeout: 90_000,
      windowsHide: true,
      // vn-test's quiet mode: the window opens behind and never takes the
      // foreground (crates/vn-app/src/quiet.rs).
      env: { ...process.env, VN_WINDOW_FOCUS: "never" },
    });
    // 0 is the embedder returning from main after vue-native closed.
    expect(r.status, r.stderr).toBe(0);
    const out = JSON.parse(r.stdout.trim().split("\n").pop()!);

    expect(out.first.available).toBe(true);
    expect(out.first.functions).toEqual(["add", "fail", "ping", "report"]);
    expect(out.first.ping).toEqual({ ok: { pong: "hello" } });
    // Answered from another thread of the embedder's, 30 ms later.
    expect(out.first.add).toEqual({ ok: 5 });
    expect(out.first.fail).toEqual({ error: "AppHostError", message: "no such luck", function: "fail" });
    expect(out.first.missing.error).toBe("TypeError");
    expect(out.first.missing.message).toContain('no function "missing"');
    // The event the embedder emitted after the first report, echoed back.
    expect(out.second).toEqual({ tick: { n: 1 } });
    expect(out.refusedNotJson).toBe("the event's data is not JSON");
    expect(out.exit.clean).toBe(true);
    expect(out.exit.summary).toMatch(/^close\(\): hid 1 window\(s\)/);
    expect(out.emitAfterClose).toBe("the app has closed");
    // The payload came from the embedder, not a stamped exe or VN_PAYLOAD.
    expect(r.stderr).toContain("payload from the embedder");
  },
  1_100_000,
);
