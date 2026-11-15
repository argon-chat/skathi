// The built-in effects: crates/sk-effects/shaders/*.slang -> crates/sk-effects/generated/ (checked in, so
// `cargo build` needs no shader compiler and a Slang upgrade that changes codegen shows up as a diff).
// The compiler itself is packages/cli/effects/compile.ts, which `sk build` and `sk dev` run over an
// app's own shaders/.
//
//   bun scripts/shaders.ts                  regenerate the built-ins
//   bun scripts/shaders.ts --check          fail if the checked-in files differ from a fresh compile
//   bun scripts/shaders.ts --app <dir> --out <dir>
//                                           compile an app's shaders/ into <dir>, for SK_EFFECTS_DIR=<dir>
//
//   --compiler <exe>     run this redslangc (or SK_REDSLANGC) instead of the pinned release
//   --build-compiler     cargo build vendor/redslangc and run that

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import {
  BUILTIN_EFFECTS,
  compileAppEffects,
  compileEffects,
  discover,
  EffectsError,
  effectFiles,
  formatProblems,
  stemOf,
  writeEffectFiles,
} from "../packages/cli/effects/compile";
import { resolveCompiler } from "../packages/cli/effects/redslangc";
import { currentPlatform, hostsFor } from "../packages/cli/cli/platform";

const REPO = resolve(import.meta.dir, "..");
const BUILTIN_SHADERS = join(REPO, "crates/sk-effects/shaders");
const GENERATED = join(REPO, "crates/sk-effects/generated");

function flag(name: string): string | undefined {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function buildCompiler(): string {
  const triple = hostsFor(currentPlatform()).triple;
  const manifest = join(REPO, "vendor/redslangc/Cargo.toml");
  if (!existsSync(manifest)) throw new Error("vendor/redslangc is not checked out (git submodule update --init)");
  const args = ["build", "--release", "--locked", "-p", "redslangc", "--manifest-path", manifest, "--target", triple];
  console.log(`cargo ${args.join(" ")}`);
  const r = spawnSync("cargo", args, { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`cargo build of vendor/redslangc failed (exit ${r.status})`);
  return join(
    REPO,
    "vendor/redslangc/target",
    triple,
    "release",
    currentPlatform() === "windows" ? "redslangc.exe" : "redslangc",
  );
}

async function builtins(compiler: string, check: boolean) {
  const { sources, problems } = discover(BUILTIN_SHADERS, (f) => `crates/sk-effects/shaders/${basename(f)}+wrapper`);
  if (problems.length) throw new EffectsError(problems);
  const byName = new Map(sources.map((s) => [stemOf(s.file), s]));
  const missing = BUILTIN_EFFECTS.filter((n) => !byName.has(n));
  const extra = [...byName.keys()].filter((n) => !BUILTIN_EFFECTS.includes(n));
  if (missing.length || extra.length) {
    throw new Error(
      "the built-in effect set is a contract with `gpui::effect_id`, the " +
        "gpui_windows and gpui_wgpu pipeline tables and crates/sk-effects.\n" +
        `  expected: ${BUILTIN_EFFECTS.join(", ")}\n` +
        (missing.length ? `  missing:  ${missing.join(", ")}\n` : "") +
        (extra.length
          ? `  extra:    ${extra.join(", ")}\n` +
            "  (adding a built-in means editing BUILTIN_EFFECTS in packages/cli/effects/compile.ts, gpui::effect_id,\n" +
            "   EffectShader::ALL in gpui_windows and BUILTIN_EFFECTS in gpui_wgpu — an APP effect needs none of that)\n"
          : ""),
    );
  }
  const compiled = await compileEffects(
    compiler,
    BUILTIN_EFFECTS.map((n) => byName.get(n)!),
  );
  for (const c of compiled) {
    if (c.effect.name !== stemOf(c.file)) {
      throw new Error(
        `${basename(c.file)}: [SkEffect("${c.effect.name}")] does not match the file name — a ` +
          "built-in's file name is part of the pipeline-table contract",
      );
    }
    if (c.warnings.length) console.warn(formatProblems(c.warnings));
  }
  const files = effectFiles("builtin", compiled, (f) => `crates/sk-effects/shaders/${basename(f)}`);

  if (check) {
    const stale = files.filter((f) => {
      const path = join(GENERATED, f.name);
      return !existsSync(path) || readFileSync(path, "utf8") !== f.text;
    });
    if (stale.length) {
      console.error("generated shader artifacts are stale — run `bun run shaders`:");
      for (const f of stale) console.error(`  ${join(GENERATED, f.name)}`);
      process.exit(1);
    }
    console.log(`shaders: ${compiled.length} built-in effect(s) up to date`);
    return;
  }
  writeEffectFiles(GENERATED, files);
  rmSync(join(GENERATED, "manifest.json"), { force: true });
  console.log(
    `shaders: wrote ${compiled.length} built-in effect(s) -> ${GENERATED}\n  ` +
      compiled.map((c) => c.effect.signature).join("\n  "),
  );
}

async function main() {
  const compiler = process.argv.includes("--build-compiler")
    ? buildCompiler()
    : (await resolveCompiler({ named: flag("compiler"), log: (line) => console.log(line) })).path;
  const appDir = flag("app");
  if (appDir) {
    const out = flag("out");
    if (!out) throw new Error("--app needs --out <dir>: where the compiled effects go (SK_EFFECTS_DIR=<dir>)");
    const app = await compileAppEffects(resolve(appDir), compiler);
    if (app.warnings.length) console.warn(formatProblems(app.warnings));
    writeEffectFiles(resolve(out), app.files);
    console.log(
      `shaders: wrote ${app.compiled.length} app effect(s) -> ${resolve(out)}\n  ` +
        app.compiled.map((c) => c.effect.signature).join("\n  "),
    );
    return;
  }
  await builtins(compiler, process.argv.includes("--check"));
}

main().catch((e: unknown) => {
  console.error(e instanceof Error ? e.message : String(e));
  process.exit(1);
});
