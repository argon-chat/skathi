/** The aot-nojit workload against the normal release host, on the same app (benches/nojit/app). Windows only.
 * bun benches/nojit/bench.ts --host <no-devtools host with native-aot> --nojit-host <aot-nojit host>
 *   [--runs 7] [--scenarios start,scroll,regexp,eval] [--compiler <sk-aot.exe>] [--out <dir>]
 *
 *   aot    the normal host running the app's native image (JIT on for eval and new Function)
 *   nojit  the aot-nojit host running the same image
 *   jit    the normal host with SK_APP_ENGINE=jit: app.prod.js under the JIT, for reference
 *
 * One process per scenario and run; variants interleave, alternating order between runs, after one warm-up
 * process per variant that also fills the image cache.
 */
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { resolveAotCompiler } from "../../packages/cli/cli/aot-build";
import { buildApp } from "../../packages/cli/cli/build-app";
import { resolveHost } from "../../packages/cli/cli/host";
import { REPO, WORK_DIR } from "../../packages/test/src/index";

const { values } = parseArgs({
  options: {
    host: { type: "string" },
    "nojit-host": { type: "string" },
    runs: { type: "string", default: "7" },
    scenarios: { type: "string", default: "start,scroll,regexp,eval" },
    compiler: { type: "string" },
    out: { type: "string" },
  },
});
if (process.platform !== "win32") throw Error("the aot-nojit workload is Windows only");
if (!values.host || !values["nojit-host"]) throw Error("--host and --nojit-host are required");
const runs = Number(values.runs);
if (!Number.isInteger(runs) || runs < 1) throw Error("--runs must be a positive integer");
const SCENARIOS = ["start", "scroll", "regexp", "eval"];
const scenarios = values.scenarios.split(",");
for (const s of scenarios) if (!SCENARIOS.includes(s)) throw Error(`unknown scenario ${s}; known: ${SCENARIOS}`);

const APP = join(REPO, "benches", "nojit", "app");
const OUT = resolve(values.out ?? join(WORK_DIR, "nojitbench"));
rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const log = (line: string) => console.log(`[nojitbench] ${line}`);

const normal = resolveHost({ variant: "no-devtools", host: resolve(values.host), log });
const nojit = resolveHost({ variant: "aot-nojit", host: resolve(values["nojit-host"]), log });
if (!normal.info.aot) throw Error(`${normal.exe} has no native AOT loader`);
const compiler = await resolveAotCompiler(normal.info.aot, values.compiler ? resolve(values.compiler) : undefined, log);

const build = async (host: string, variant: "no-devtools" | "aot-nojit") => {
  const result = await buildApp({
    target: APP,
    out: join(OUT, variant),
    host,
    variant,
    mode: "release",
    aot: true,
    aotCompiler: compiler.path,
    console: true,
    log,
  });
  if (!result.exe || !result.native) throw Error(`${variant}: no exe or native image`);
  return result;
};
const normalBuild = await build(normal.exe, "no-devtools");
const nojitBuild = await build(nojit.exe, "aot-nojit");
const imageSha = (b: typeof normalBuild) => JSON.parse(readFileSync(b.native!.manifest, "utf8")).imageSha256 as string;
if (imageSha(normalBuild) !== imageSha(nojitBuild)) throw Error("the two builds compiled different images");

const VARIANTS = {
  aot: { exe: normalBuild.exe!, env: {}, engine: "aot" },
  nojit: { exe: nojitBuild.exe!, env: {}, engine: "aot" },
  jit: { exe: normalBuild.exe!, env: { SK_APP_ENGINE: "jit" }, engine: "jit" },
} as const;
type Variant = keyof typeof VARIANTS;
const variants = Object.keys(VARIANTS) as Variant[];

type Sample = Record<string, number>;
const samples: Record<string, Record<Variant, Sample[]>> = {};

async function once(variant: Variant, scenario: string): Promise<Sample> {
  const { exe, env, engine } = VARIANTS[variant];
  const t0 = performance.now();
  const child = Bun.spawn([exe], {
    env: {
      ...process.env,
      // Opened behind other windows and never activated; occlusion off so rAF keeps running when covered.
      SK_WINDOW_FOCUS: "never",
      SK_OCCLUSION: "0",
      SK_SINGLE_INSTANCE: "0",
      SK_LAUNCH_RAISE: "off",
      SK_DIALOGS: "scripted",
      SK_PERF: "1",
      SK_SCENARIO: scenario,
      SK_AOT_CACHE_DIR: join(OUT, "aot-cache", variant),
      ...env,
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  let resultAt = 0;
  let stdout = "";
  const decoder = new TextDecoder();
  const reader = (async () => {
    for await (const chunk of child.stdout) {
      stdout += decoder.decode(chunk, { stream: true });
      if (!resultAt && stdout.includes("NOJITBENCH ")) resultAt = performance.now();
    }
  })();
  const stderrText = new Response(child.stderr).text();
  const timer = setTimeout(() => child.kill(), 300_000);
  await child.exited;
  clearTimeout(timer);
  await reader;
  const stderr = await stderrText;
  const all = stdout + stderr;
  const line = /NOJITBENCH (\{.*\})/.exec(all);
  if (!line) throw Error(`${variant}/${scenario}: no result (exit ${child.exitCode})\n${all.slice(-4000)}`);
  const result = JSON.parse(line[1]!) as Record<string, unknown>;
  if (result.error) throw Error(`${variant}/${scenario}: ${result.error}`);
  const ran = /\[native-aot\] selected application image/.test(all)
    ? "aot"
    : /SK_APP_ENGINE=jit: running app\.prod\.js/.test(all)
      ? "jit"
      : "unknown";
  if (ran !== engine) throw Error(`${variant}/${scenario} ran under ${ran}\n${all.slice(-4000)}`);
  const sample: Sample = {};
  for (const [k, v] of Object.entries(result)) if (typeof v === "number" && k !== "sink") sample[k] = v;
  const boot = /boot->first-frame ([\d.]+)ms/.exec(all);
  if (boot) sample.bootToFirstFrame = Number(boot[1]);
  if (scenario === "start") sample.spawnToResult = resultAt - t0;
  return sample;
}

for (const scenario of scenarios) {
  samples[scenario] = { aot: [], nojit: [], jit: [] };
  for (const variant of variants) await once(variant, scenario);
  for (let run = 0; run < runs; run++) {
    const order = run % 2 === 0 ? variants : [...variants].reverse();
    for (const variant of order) samples[scenario]![variant].push(await once(variant, scenario));
  }
  log(`${scenario}: ${runs} runs per variant`);
}

const median = (xs: number[]) => {
  const s = [...xs].sort((a, b) => a - b);
  const mid = (s.length - 1) / 2;
  return (s[Math.floor(mid)]! + s[Math.ceil(mid)]!) / 2;
};
const summary: Record<string, Record<string, Record<Variant, number>>> = {};
const lines = ["scenario.metric (ms, median) | aot | nojit | nojit/aot | jit", "--- | --- | --- | --- | ---"];
for (const scenario of scenarios) {
  const metrics = Object.keys(samples[scenario]!.aot[0] ?? {});
  summary[scenario] = {};
  for (const metric of metrics) {
    const m = Object.fromEntries(
      variants.map((v) => [v, median(samples[scenario]![v].map((s) => s[metric]!).filter(Number.isFinite))]),
    ) as Record<Variant, number>;
    summary[scenario]![metric] = m;
    lines.push(
      `${scenario}.${metric} | ${m.aot.toFixed(1)} | ${m.nojit.toFixed(1)} | ${(m.nojit / m.aot).toFixed(2)}x | ${m.jit.toFixed(1)}`,
    );
  }
}
console.log(`\n${lines.join("\n")}\n`);
const report = join(OUT, "results.json");
writeFileSync(
  report,
  JSON.stringify(
    {
      hosts: { normal: normal.exe, nojit: nojit.exe },
      compiler: compiler.path,
      imageBytes: normalBuild.native!.imageBytes,
      exeBytes: { aot: Bun.file(normalBuild.exe!).size, nojit: Bun.file(nojitBuild.exe!).size },
      runs,
      samples,
      summary,
    },
    null,
    2,
  ) + "\n",
);
log(`samples: ${report}`);
