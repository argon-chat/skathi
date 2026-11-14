// Runs the bad example of every SK1xxx/SK2xxx check code through the pinned AOT frontend
// (`build --compile --aot`) and, for contrast, a plain `--compile`, and records what each does.
//
// Usage: bun scripts/aot/verify-rules.ts [--frontend <pinned bun.exe>] [--out <report.json>]
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { ALL_CODES } from "../../packages/cli/check/codes";
import { loadPin, verifyFile } from "./toolchain";

const SDK = resolve(import.meta.dir, "../..");
const PIN = loadPin(join(import.meta.dir, "toolchain.lock.json"));
const MARK = "SKPROBE ";

function option(name: string): string | undefined {
  const at = process.argv.indexOf(name);
  if (at < 0) return undefined;
  const value = process.argv[at + 1];
  if (!value) throw Error(`${name} needs a value`);
  return resolve(value);
}

function frontendPath(): string {
  const given = option("--frontend");
  if (given) return given;
  const rel = join(".tools", "native-aot", `frontend-${PIN.frontend.revision}`, "package", "bin", "bun.exe");
  for (const root of [SDK, resolve(SDK, "../..")]) if (existsSync(join(root, rel))) return join(root, rel);
  throw Error(`No pinned frontend at ${rel}; pass --frontend <bun.exe> (scripts/aot/download.ts fetches it)`);
}

interface Run {
  build: { ok: boolean; warnings: string[] };
  outcome: string;
}

function probe(example: string): string {
  return [
    "let outcome = 'ran';",
    "try {",
    "  await (async () => {",
    ...example.split("\n").map((line) => `    ${line}`),
    "  })();",
    "  await new Promise((done) => setTimeout(done, 20));",
    "} catch (e) {",
    "  outcome = `throws ${(e as Error).name}: ${(e as Error).message.split('\\n')[0]}`;",
    "}",
    `console.log(${JSON.stringify(MARK)} + JSON.stringify({ outcome }));`,
    "export {};",
    "",
  ].join("\n");
}

function run(frontend: string, dir: string, source: string, aot: boolean): Run {
  const exe = join(dir, aot ? "aot.exe" : "jit.exe");
  const built = Bun.spawnSync([frontend, "build", "--compile", ...(aot ? ["--aot"] : []), source, "--outfile", exe], {
    cwd: dir,
    stderr: "pipe",
    stdout: "pipe",
  });
  const log = `${built.stdout.toString()}${built.stderr.toString()}`;
  const warnings = [...log.matchAll(/(?:warn|error): (TS\d+: [^\n]*)/g)].map((m) => m[1]!.trim());
  if (built.exitCode !== 0 || !existsSync(exe)) return { build: { ok: false, warnings }, outcome: "build failed" };
  const ran = Bun.spawnSync([exe], { cwd: dir, stdout: "pipe", stderr: "pipe", timeout: 20_000 });
  const out = ran.stdout.toString();
  const line = out.split(/\r?\n/).find((l) => l.startsWith(MARK));
  const outcome = line
    ? (JSON.parse(line.slice(MARK.length)) as { outcome: string }).outcome
    : `exited ${ran.exitCode}: ${
        ran.stderr
          .toString()
          .split(/\r?\n/)
          .find((l) => /Error/.test(l))
          ?.trim() ?? ""
      }`;
  return { build: { ok: true, warnings }, outcome };
}

const frontend = frontendPath();
await verifyFile(frontend, PIN.frontend.executableSha256);
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
    const aot = run(frontend, dir, source, true);
    const jit = run(frontend, dir, source, false);
    results.push({ code: info.code, severity: info.severity, example: info.example, aot, jit });
    console.error(
      `${info.code}  aot: ${aot.outcome}${aot.build.warnings.length ? ` [${aot.build.warnings.map((w) => w.split(":")[0]).join(", ")}]` : ""}  |  jit: ${jit.outcome}`,
    );
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
const report = { frontend: PIN.frontend.revision, engine: PIN.engine.revision, results };
const out = option("--out");
if (out) writeFileSync(out, JSON.stringify(report, null, 2) + "\n");
else console.log(JSON.stringify(report, null, 2));
