// Runs the bad example of every SK1xxx/SK2xxx check code through sk-aot (compiled to an image and
// run the way a host runs it) and, for contrast, through bun, and records what each does.
//
// Usage: bun scripts/aot/verify-rules.ts [--compiler <sk-aot.exe>] [--out <report.json>]
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { compilerInfo, repoSkAot } from "../../packages/cli/aot/native";
import { ALL_CODES } from "../../packages/cli/check/codes";
import { loadPin, verifyCompilerInfo } from "./toolchain";

const SDK = resolve(import.meta.dir, "../..");
const PIN = loadPin();
const MARK = "SKPROBE ";

function option(name: string): string | undefined {
  const at = process.argv.indexOf(name);
  if (at < 0) return undefined;
  const value = process.argv[at + 1];
  if (!value) throw Error(`${name} needs a value`);
  return resolve(value);
}

interface Run {
  build: { ok: boolean; warnings: string[] };
  outcome: string;
}

// No top-level await: `sk-aot run` has no event loop, so the outcome is settled in microtasks.
function probe(example: string): string {
  return [
    "const report = (outcome: string) => {",
    "  (globalThis as Record<string, unknown>).__sk_aot_probe = outcome;",
    `  if (typeof process !== "undefined") console.log(${JSON.stringify(MARK)} + JSON.stringify({ outcome }));`,
    "};",
    "(async () => {",
    ...example.split("\n").map((line) => `  ${line}`),
    "})().then(",
    "  () => report('ran'),",
    "  (e) => report(`throws ${(e as Error).name}: ${(e as Error).message.split('\\n')[0]}`),",
    ");",
    "export {};",
    "",
  ].join("\n");
}

const firstError = (text: string) =>
  text
    .split(/\r?\n/)
    .find((l) => /Error|error:/.test(l))
    ?.trim() ?? "";

function aot(compiler: string, dir: string, source: string): Run {
  const image = join(dir, "probe.image");
  const built = Bun.spawnSync([compiler, "compile", source, join(dir, "probe.ir"), image], {
    cwd: dir,
    stderr: "pipe",
    stdout: "pipe",
  });
  const log = `${built.stdout.toString()}${built.stderr.toString()}`;
  const warnings = [...log.matchAll(/(?:warn|error): (TS\d+: [^\n]*)/g)].map((m) => m[1]!.trim());
  if (built.exitCode !== 0 || !existsSync(image)) return { build: { ok: false, warnings }, outcome: "build failed" };
  const ran = Bun.spawnSync([compiler, "run", image], { cwd: dir, stdout: "pipe", stderr: "pipe", timeout: 20_000 });
  const line = /probe=(.*)$/m.exec(ran.stdout.toString());
  const outcome =
    ran.exitCode === 0 && line ? line[1]!.trim() : `exited ${ran.exitCode}: ${firstError(ran.stderr.toString())}`;
  return { build: { ok: true, warnings }, outcome };
}

function jit(dir: string, source: string): Run {
  const ran = Bun.spawnSync([process.execPath, source], { cwd: dir, stdout: "pipe", stderr: "pipe", timeout: 20_000 });
  const line = ran.stdout
    .toString()
    .split(/\r?\n/)
    .find((l) => l.startsWith(MARK));
  const outcome = line
    ? (JSON.parse(line.slice(MARK.length)) as { outcome: string }).outcome
    : `exited ${ran.exitCode}: ${firstError(ran.stderr.toString())}`;
  return { build: { ok: true, warnings: [] }, outcome };
}

const compiler = option("--compiler") ?? repoSkAot(SDK);
if (!existsSync(compiler))
  throw Error(
    `No sk-aot at ${compiler}; build it (bun tools/sk-aot/setup.ts, then cargo build --release in tools/sk-aot) or pass --compiler`,
  );
verifyCompilerInfo({ ...(await compilerInfo(compiler)) }, PIN);
const work = mkdtempSync(join(tmpdir(), "sk-verify-rules-"));
const results = [];
try {
  for (const info of ALL_CODES.filter((c) => /^SK[12]/.test(c.code))) {
    const dir = join(work, info.code);
    // The module SK2004's example names, so only the computed specifier differs.
    mkdirSync(join(dir, "pages"), { recursive: true });
    writeFileSync(join(dir, "pages", "home.ts"), "export const home = 1;\n");
    const source = join(dir, "probe.ts");
    writeFileSync(source, probe(info.example));
    const native = aot(compiler, dir, source);
    const contrast = jit(dir, source);
    results.push({ code: info.code, severity: info.severity, example: info.example, aot: native, jit: contrast });
    console.error(
      `${info.code}  aot: ${native.outcome}${native.build.warnings.length ? ` [${native.build.warnings.map((w) => w.split(":")[0]).join(", ")}]` : ""}  |  jit: ${contrast.outcome}`,
    );
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
const report = { frontend: PIN.frontend.revision, engine: PIN.engine.revision, results };
const out = option("--out");
if (out) writeFileSync(out, JSON.stringify(report, null, 2) + "\n");
else console.log(JSON.stringify(report, null, 2));
