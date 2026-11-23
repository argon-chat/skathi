// An app that cannot start must fail its launch at once with the reason: an entry that throws (dev and
// packaged) and a bundle whose runtime protocol is not the host's.

import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { cpSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, packagedExe, REPO, WORK_DIR } from "../src/index";
import { ensureBuild, resolveEntry } from "../src/dist";

const FIXTURE = join(REPO, "packages", "test", "fixtures", "entry-throws");
const THROWN = "entry-throws fixture: startup failed on purpose";
const FAST = 20_000;

async function launchError(run: () => Promise<unknown>): Promise<{ message: string; ms: number }> {
  const t0 = Date.now();
  try {
    await run();
  } catch (e) {
    return { message: (e as Error).message, ms: Date.now() - t0 };
  }
  throw new Error("the launch succeeded");
}

describe("an app whose entry throws", () => {
  test("under the dev host, fails the launch with the error and its stack", async () => {
    const { message, ms } = await launchError(() => launch({ app: FIXTURE, label: "entry-throws-dev" }));
    expect(message).toContain(THROWN);
    expect(message).toMatch(/startup@|main\.ts/);
    expect(ms).toBeLessThan(FAST);
  }, 60_000);

  test(
    "packaged, exits with code 1 and leaves the error in its log",
    async () => {
      const p = await packagedExe({ app: FIXTURE, label: "entry-throws" });
      const { message, ms } = await launchError(() =>
        launch({ app: "entry-throws", exe: p.exe, packaged: p.name, label: "entry-throws-packaged" }),
      );
      expect(message).toContain(THROWN);
      expect(ms).toBeLessThan(FAST);

      const run = spawnSync(p.exe, [], {
        cwd: REPO,
        encoding: "utf8",
        timeout: FAST,
        env: {
          ...(process.env as Record<string, string>),
          SK_OWNER_PID: String(process.pid),
          SK_WINDOW_FOCUS: "never",
          SK_SINGLE_INSTANCE: "0",
          SK_DIALOGS: "scripted",
        },
      });
      expect(run.error).toBeUndefined();
      expect(run.status).toBe(1);
      expect(run.stderr).toContain(THROWN);
    },
    20 * 60_000,
  );
});

describe("a bundle built against another runtime protocol", () => {
  test("stops before the runtime runs and names both protocols", async () => {
    const built = await ensureBuild(resolveEntry("hello"));
    const dist = join(WORK_DIR, "dists", `protocol-mismatch-${process.pid}`);
    rmSync(dist, { recursive: true, force: true });
    cpSync(built, dist, { recursive: true });
    const manifestPath = join(dist, "manifest.json");
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as { runtime: { protocol: string } };
    const ours = manifest.runtime.protocol;
    const other = ours === "00000000" ? "00000001" : "00000000";
    manifest.runtime.protocol = other;
    writeFileSync(manifestPath, JSON.stringify(manifest));
    try {
      const { message, ms } = await launchError(() => launch({ app: "hello", dist, label: "protocol-mismatch" }));
      expect(message).toContain(`runtime protocol ${ours}`);
      expect(message).toContain(`(protocol ${other})`);
      expect(message).not.toContain("is not defined");
      expect(ms).toBeLessThan(FAST);
    } finally {
      rmSync(dist, { recursive: true, force: true });
    }
  }, 60_000);
});
