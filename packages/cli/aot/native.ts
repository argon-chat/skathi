// One production bundle -> one native image: the opaque-host transform, then `sk-aot compile`
// (bundle -> IR -> machine code). Shared by `sk build --aot` and the inspector's release build, so
// both produce manifests the host validates the same way.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { COMPILER_FIELDS, type CompilerInfo } from "./pin";
import {
  CLOSED_PRELUDES_TRANSFORM,
  nativeInput,
  PRELUDES_TRANSFORM,
  TRANSFORM,
  type PreludeScripts,
} from "./transform";
import { excerpt, INPUT_MODULE, readUses, usesOf, type Uses } from "./uses";

export class AotError extends Error {}

const sha256 = (bytes: string | Uint8Array) => createHash("sha256").update(bytes).digest("hex");

interface Ran {
  code: number;
  stdout: string;
  stderr: string;
}

/** Async, so a spinner keeps turning while sk-aot works. */
async function run(command: string[], cwd?: string): Promise<Ran> {
  const child = Bun.spawn(command, { ...(cwd ? { cwd } : {}), stdout: "pipe", stderr: "pipe", stdin: "ignore" });
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  return { code, stdout, stderr };
}

/** Where `cargo build --release` in a checkout's tools/sk-aot puts sk-aot. */
export const repoSkAot = (root: string) =>
  join(root, "tools", "sk-aot", "target", "x86_64-pc-windows-msvc", "release", "sk-aot.exe");

const tail = (text: string, lines = 40) => text.trim().split(/\r?\n/).slice(-lines).join("\n");

export async function compilerInfo(compiler: string): Promise<CompilerInfo> {
  if (!existsSync(compiler)) throw new AotError(`the native image compiler ${compiler} does not exist`);
  const ran = await run([compiler, "info"]);
  let info: Record<string, unknown> = {};
  try {
    info = JSON.parse(ran.stdout.trim()) as Record<string, unknown>;
  } catch {}
  const fields = COMPILER_FIELDS.map((name) => [name, info[name]] as const);
  if (ran.code !== 0 || fields.some(([, value]) => typeof value !== "string")) {
    throw new AotError(
      `${compiler} is not a working sk-aot (\`info\` exited ${ran.code})\n${tail(ran.stdout + ran.stderr, 5)}`,
    );
  }
  return Object.fromEntries(fields) as unknown as CompilerInfo;
}

/** The first field where `actual` and `expected` disagree, or null. */
export function compilerMismatch(actual: CompilerInfo, expected: CompilerInfo): string | null {
  for (const name of COMPILER_FIELDS)
    if (actual[name] !== expected[name]) return `${name} ${actual[name]} (the host needs ${expected[name]})`;
  return null;
}

export interface NativeImage {
  image: Uint8Array;
  entry: number;
  identity: string;
  imageSha256: string;
  sourceBundleSha256: string;
  nativeInputSha256: string;
  irSha256: string;
  diagnostics: string;
  /** What sk-aot found the image to use; null from an sk-aot that does not record it. */
  uses: Uses | null;
  closed?: boolean;
}

export interface CompileOptions {
  /** sk-aot. */
  compiler: string;
  source: string;
  preludes?: PreludeScripts | undefined;
  closed?: boolean;
  /** Holds the transformed input, the IR and the image. */
  work: string;
}

export async function compileNative(options: CompileOptions): Promise<NativeImage> {
  const { compiler, source, work } = options;
  mkdirSync(work, { recursive: true });
  const input = join(work, "native-input.js");
  const ir = join(work, "app.ir");
  const image = join(work, "app.image");
  const uses = `${ir}.uses.json`;
  const openIr = join(work, "audit.ir");
  for (const stale of [ir, uses, openIr, `${openIr}.uses.json`, image, `${image}.entries`, `${image}.identity`])
    rmSync(stale, { force: true });
  const wrapped = nativeInput(source, options.preludes, options.closed);
  writeFileSync(input, wrapped.code);

  const strict = options.closed ? await import("./closed") : null;
  let openUses: Uses | null = null;
  if (strict) {
    if (!options.preludes) throw new AotError("the strict workload requires a host with in-image preludes");
    const audit = await run([compiler, "ir", input, openIr, "--include-builtins"], work);
    if (audit.code !== 0 || !(openUses = readUses(`${openIr}.uses.json`)))
      throw new AotError(
        `sk-aot could not audit the strict image; use the compiler of this SDK\n${tail(audit.stderr + audit.stdout)}`,
      );
  }
  const flags = strict ? ["--include-builtins", "--option", strict.CLOSED_POLICY] : [];
  const compiled = await run([compiler, "compile", input, ir, image, ...flags], work);
  if (compiled.code !== 0 || !existsSync(ir) || !existsSync(image)) {
    throw new AotError(
      `sk-aot could not compile the app bundle (exit ${compiled.code}); its input is ${input}\n` +
        tail(compiled.stderr + compiled.stdout),
    );
  }
  const entries = readFileSync(`${image}.entries`, "utf8").trim().split(/\s+/).map(Number);
  const entry = entries[0];
  if (entries.length !== 1 || entry === undefined || !Number.isSafeInteger(entry) || entry < 1)
    throw new AotError(`sk-aot reported entries ${JSON.stringify(entries)}, expected one`);
  const identity = readFileSync(`${image}.identity`);
  if (identity.length !== 16) throw new AotError("sk-aot wrote an invalid image identity");
  const bytes = new Uint8Array(readFileSync(image));
  const recorded = readUses(uses);
  return {
    image: bytes,
    entry,
    identity: identity.toString("hex"),
    imageSha256: sha256(bytes),
    sourceBundleSha256: sha256(source),
    nativeInputSha256: sha256(wrapped.code),
    irSha256: sha256(readFileSync(ir)),
    diagnostics: compiled.stdout.trim(),
    uses: strict && recorded && openUses ? strict.closedUses(recorded, openUses, wrapped) : recorded,
    closed: !!strict,
  };
}

/** Why `workload`, which leaves the engine features `removed` out, cannot run this image, or null. */
export function removedFeatureProblem(
  native: NativeImage,
  source: string,
  removed: readonly string[],
  workload: string,
  preludes?: PreludeScripts,
): string | null {
  if (!removed.length) return null;
  const list = removed.join(" and ");
  if (!native.uses) {
    return (
      `the ${workload} workload leaves out ${list}, and the sk-aot that compiled this app does not record what its image uses — ` +
      "use the sk-aot of this Skathi version"
    );
  }
  const found = usesOf(native.uses, removed);
  const unseen = removed.includes("Interpreter") ? native.uses.unseen : [];
  if (!found.length && !unseen.length) return null;
  // sk-aot reports byte offsets in its UTF-8 input. Editor columns count UTF-16 code units.
  const sections = nativeInput(source, preludes, native.closed).sections.map((s) => ({
    ...s,
    bytes: Buffer.from(s.source),
  }));
  const at = (what: string, module: number, position: number) => {
    if (module !== INPUT_MODULE) return `${what} in the engine's built-ins`;
    const section = sections.find((s) => position >= s.byteOffset && position < s.byteOffset + s.bytes.length);
    return section
      ? `${what} at ${section.file} ${excerpt(section.source, section.bytes.toString("utf8", 0, position - section.byteOffset).length)}`
      : `${what} in the generated native input at offset ${position}`;
  };
  const lines = [
    ...new Set([
      ...unseen.map((use) => at(`${use.cause} (${use.builtin ?? "code outside the image"})`, use.module, use.position)),
      ...found.map((use) =>
        at(
          `${use.builtin ?? use.feature}${use.kind === "FoundByName" ? " (read by name)" : ""}`,
          use.module,
          use.position,
        ),
      ),
    ]),
  ];
  const shown = lines.slice(0, 5).map((line) => `  ${line}`);
  if (lines.length > shown.length) shown.push(`  and ${lines.length - shown.length} more`);
  const features = [...new Set([...found.map((use) => use.feature), ...(unseen.length ? ["Interpreter"] : [])])].join(
    " and ",
  );
  return (
    `this app's native image ${removed.includes("Interpreter") ? "can reach" : "uses"} ${features}, which the ${workload} workload leaves out:\n${shown.join("\n")}\n` +
    (removed.includes("Interpreter")
      ? "Use a workload that includes the interpreter, or compile an image with no interpreter dependencies."
      : `Drop that use, or publish on the workload with the JIT: remove "publishJit": false from sk.config.json.`)
  );
}

export type NativeKind = "inspector" | "application";

/** The manifest the host checks before it loads `image` (crates/sk-app/src/native_aot.rs). */
export function nativeManifest(
  kind: NativeKind,
  info: CompilerInfo,
  native: NativeImage,
  options: { image: string; preludeSha256: string; includesPreludes?: boolean },
): Record<string, unknown> {
  return {
    schema: "skathi.native-aot",
    version: 1,
    stage: "native-image",
    kind,
    ...info,
    image: options.image,
    imageSha256: native.imageSha256,
    imageBytes: native.image.length,
    entry: native.entry,
    identity: native.identity,
    transform: native.closed ? CLOSED_PRELUDES_TRANSFORM : options.includesPreludes ? PRELUDES_TRANSFORM : TRANSFORM,
    sourceBundleSha256: native.sourceBundleSha256,
    preludeSha256: options.preludeSha256,
    nativeInputSha256: native.nativeInputSha256,
    irSha256: native.irSha256,
    sourceStage: "production-javascript",
  };
}
