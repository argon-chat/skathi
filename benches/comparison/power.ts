import { closeSync, openSync, writeFileSync } from "node:fs";
import { cpus } from "node:os";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { launch, REPO } from "../../packages/test/src/index";

const { values } = parseArgs({
  options: {
    out: { type: "string" },
    runs: { type: "string", default: "3" },
  },
});
const runs = Number(values.runs);
if (!Number.isInteger(runs) || runs < 1) throw Error("--runs must be a positive integer");
const output = values.out ?? resolve(REPO, "target/comparison", `power-${Date.now()}.jsonl`);

function systemTimes() {
  let idle = 0;
  let total = 0;
  for (const { times } of cpus()) {
    idle += times.idle;
    total += times.user + times.nice + times.sys + times.idle + times.irq;
  }
  return { idle, total };
}

function processTimes(pid: number) {
  const result = Bun.spawnSync([
    "pwsh",
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    `$p = [Diagnostics.Process]::GetProcessById(${pid}); $p.TotalProcessorTime.Ticks; [Environment]::TickCount64`,
  ]);
  const [ticks, ms] = result.stdout.toString().trim().split(/\s+/).map(Number);
  if (!result.success || ticks === undefined || ms === undefined || !Number.isFinite(ticks + ms)) {
    throw Error("Cannot read the host's CPU time");
  }
  return { ticks, ms };
}

const modes = [
  { scheme: "balanced", pressure: "nominal", tier: 0 },
  { scheme: "saving", pressure: "nominal", tier: 1 },
  { scheme: "saving", pressure: "critical", tier: 2 },
] as const;

const file = openSync(output, "wx");
try {
  const app = await launch({
    app: "benches/comparison/app",
    exe: resolve(REPO, "target/comparison/apps/full/compare-skathi.exe"),
    packaged: "skathi-app",
    label: "comparison-power",
    env: { SK_DEVTOOLS: "1", SK_IDLE_PACING: "1", SK_MEMORY: "default" },
  });
  try {
    const deadline = Date.now() + 60000;
    while ((await app.eval("document.title")) !== "Compare scrolled") {
      if (Date.now() >= deadline) throw Error("List did not finish scrolling");
      await Bun.sleep(500);
    }
    const list = (await app.query(".list")).nodes[0];
    if (!list) throw Error("Missing scroll container");
    await app.send("effects", { op: "set", node: list.id, value: "noise(0.5, 1px, 60)" });
    for (let run = 1; run <= runs; run++) {
      for (let i = 0; i < modes.length; i++) {
        const mode = modes[(i + run - 1) % modes.length]!;
        await app.send("power", { scheme: mode.scheme, pressure: mode.pressure });
        const until = Date.now() + 5000;
        while (true) {
          const painted: { effect: string; quality: number }[] = await app.send("effects", { op: "painted" });
          if (painted.some((item) => item.effect === "noise" && item.quality === mode.tier)) break;
          if (Date.now() >= until) throw Error(`Noise effect did not reach tier ${mode.tier}`);
          await Bun.sleep(50);
        }
        await Bun.sleep(1000);
        for (let attempt = 1; ; attempt++) {
          const system = systemTimes();
          const cpu = processTimes(app.pid);
          const before: { frames: number; uptimeMs: number } = await app.send("perf");
          await Bun.sleep(10000);
          const after: { frames: number; uptimeMs: number } = await app.send("perf");
          const end = processTimes(app.pid);
          const endSystem = systemTimes();
          const busy = 100 * (1 - (endSystem.idle - system.idle) / (endSystem.total - system.total));
          if (busy >= 20) {
            if (attempt >= 4) throw Error("System CPU averaged at least 20% in four sampling windows");
            continue;
          }
          const frames = after.frames - before.frames;
          const elapsed_ms = after.uptimeMs - before.uptimeMs;
          if (frames <= 0 || elapsed_ms <= 0) throw Error("Missing frames in animation sample");
          const row = {
            run,
            ...mode,
            attempt,
            frames,
            elapsed_ms,
            fps: (1000 * frames) / elapsed_ms,
            cpu_pct: (100 * (end.ticks - cpu.ticks)) / 10000 / (end.ms - cpu.ms),
            system_cpu_pct: busy,
          };
          writeFileSync(file, JSON.stringify(row) + "\n");
          console.log(`round ${run} tier ${mode.tier}: ${row.fps.toFixed(1)} frames/s`);
          break;
        }
      }
    }
  } finally {
    await app.close();
  }
} finally {
  closeSync(file);
}
