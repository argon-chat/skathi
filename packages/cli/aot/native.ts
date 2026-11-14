// One production bundle -> one native image: the opaque-host transform, then `sk-aot compile`
// (bundle -> IR -> machine code). Shared by `sk build --aot` and the inspector's release build, so
// both produce manifests the host validates the same way.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { COMPILER_FIELDS, type CompilerInfo } from "./pin";
import { opaqueHost, TRANSFORM } from "./transform";

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
}

export interface CompileOptions {
  /** sk-aot. */
  compiler: string;
  source: string;
  /** Holds the transformed input, the IR and the image. */
  work: string;
}

export async function compileNative(options: CompileOptions): Promise<NativeImage> {
  const { compiler, source, work } = options;
  mkdirSync(work, { recursive: true });
  const input = join(work, "native-input.js");
  const ir = join(work, "app.ir");
  const image = join(work, "app.image");
  for (const stale of [ir, image, `${image}.entries`, `${image}.identity`]) rmSync(stale, { force: true });
  const wrapped = opaqueHost(source);
  writeFileSync(input, wrapped);

  const compiled = await run([compiler, "compile", input, ir, image], work);
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
  return {
    image: bytes,
    entry,
    identity: identity.toString("hex"),
    imageSha256: sha256(bytes),
    sourceBundleSha256: sha256(source),
    nativeInputSha256: sha256(wrapped),
    irSha256: sha256(readFileSync(ir)),
    diagnostics: compiled.stdout.trim(),
  };
}

export type NativeKind = "inspector" | "application";

/** The manifest the host checks before it loads `image` (crates/sk-app/src/native_aot.rs). */
export function nativeManifest(
  kind: NativeKind,
  info: CompilerInfo,
  native: NativeImage,
  options: { image: string; preludeSha256: string },
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
    transform: TRANSFORM,
    sourceBundleSha256: native.sourceBundleSha256,
    preludeSha256: options.preludeSha256,
    nativeInputSha256: native.nativeInputSha256,
    irSha256: native.irSha256,
    sourceStage: "production-javascript",
  };
}
