// An app's own `--shading` effects in the sk commands: its shaders/*.slang compiled for every renderer
// (HLSL, WGSL, MSL), so a build made on one OS runs on all three. `sk build` puts them into the payload
// as `sk/effects/*` entries; `sk dev` writes them to dist/.sk-dev/effects and points the dev host there.

import { watch, type FSWatcher } from "node:fs";
import {
  appShadersDir,
  compileAppEffects,
  EffectsError,
  formatProblems,
  hasAppEffects,
  writeEffectFiles,
  type AppEffects,
} from "../effects/compile";
import { resolveCompiler, type ShaderCompiler } from "../effects/redslangc";
import { ConfigError } from "./config";
import { KIND, NAMES, type SkpkInput } from "./skpk";
import type { Progress } from "./term";

export interface BuiltEffects {
  effects: AppEffects;
  compiler: ShaderCompiler;
  /// The SKPK entries: effects.json and one file per effect and target.
  inputs: SkpkInput[];
}

const enc = new TextEncoder();

const effectInputs = (effects: AppEffects): SkpkInput[] =>
  effects.files.map((f) => ({ name: `${NAMES.EFFECTS_DIR}${f.name}`, kind: KIND.EFFECT, data: enc.encode(f.text) }));

export function effectsFailure(e: EffectsError): ConfigError {
  const errors = e.problems.filter((p) => p.severity === "error");
  return new ConfigError(
    `the app's effect shaders do not compile (${errors.length} error${errors.length === 1 ? "" : "s"}):\n` +
      formatProblems(e.problems),
  );
}

/// Compiles the app's shaders into `out` and the payload entries; null for an app without any.
export async function buildAppEffects(options: {
  appRoot: string;
  out: string;
  compiler?: string | undefined;
  progress: Progress;
}): Promise<BuiltEffects | null> {
  const { appRoot, out, progress } = options;
  if (!hasAppEffects(appRoot)) return null;
  progress.step("effects");
  const compiler = await resolveCompiler({
    named: options.compiler,
    log: (line) => progress.info(line),
    onBytes: (done, total) => progress.bytes(done, total),
  });
  let effects: AppEffects;
  try {
    effects = await compileAppEffects(appRoot, compiler.path);
  } catch (e) {
    throw e instanceof EffectsError ? effectsFailure(e) : e;
  }
  for (const line of formatProblems(effects.warnings).split("\n").filter(Boolean)) progress.warn(line);
  writeEffectFiles(out, effects.files);
  progress.detail(`${effects.compiled.map((c) => c.effect.name).join(", ")} · hlsl, wgsl, metal`);
  return { effects, compiler, inputs: effectInputs(effects) };
}

export interface EffectsWatch {
  close(): void;
}

/// Recompiles into `out` when a `.slang` under shaders/ changes, debounced. A failed compile keeps the
/// last good files and reports through `onError`.
export function watchAppEffects(options: {
  appRoot: string;
  out: string;
  compiler: string;
  onBuilt: (effects: AppEffects) => void;
  onError: (message: string) => void;
  debounceMs?: number;
}): EffectsWatch {
  const dir = appShadersDir(options.appRoot);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let running = false;
  let again = false;
  const rebuild = async () => {
    if (running) {
      again = true;
      return;
    }
    running = true;
    try {
      const effects = await compileAppEffects(options.appRoot, options.compiler);
      writeEffectFiles(options.out, effects.files);
      options.onBuilt(effects);
    } catch (e) {
      options.onError(
        e instanceof EffectsError ? formatProblems(e.problems) : e instanceof Error ? e.message : String(e),
      );
    } finally {
      running = false;
      if (again) {
        again = false;
        void rebuild();
      }
    }
  };
  let watcher: FSWatcher | null = null;
  try {
    watcher = watch(dir, { recursive: true }, (_event, file) => {
      if (file && !String(file).endsWith(".slang")) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        void rebuild();
      }, options.debounceMs ?? 120);
    });
  } catch (e) {
    options.onError(`cannot watch ${dir}: ${e instanceof Error ? e.message : String(e)}`);
  }
  return {
    close() {
      if (timer) clearTimeout(timer);
      watcher?.close();
    },
  };
}
