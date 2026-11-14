// One production bundle -> one native image: the opaque-host transform, the pinned frontend
// (bundle -> IR) and the image compiler (IR -> machine code). Shared by `sk build --aot` and the
// inspector's release build, so both produce manifests the host validates the same way.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { downloadVerified } from "./download";
import { COMPILER_FIELDS, verifyFile, type CompilerInfo, type ToolchainPin } from "./pin";
import { opaqueHost, TRANSFORM } from "./transform";

export class AotError extends Error {}

const sha256 = (bytes: string | Uint8Array) => createHash("sha256").update(bytes).digest("hex");

interface Ran {
  code: number;
  stdout: string;
  stderr: string;
}

/** Async, so a spinner keeps turning while the frontend and the compiler work. */
async function run(command: string[], cwd?: string): Promise<Ran> {
  const child = Bun.spawn(command, { ...(cwd ? { cwd } : {}), stdout: "pipe", stderr: "pipe", stdin: "ignore" });
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  return { code, stdout, stderr };
}

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
      `${compiler} is not a native image compiler (\`info\` exited ${ran.code})\n${tail(ran.stdout + ran.stderr, 5)}`,
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

/** One regular file's bytes out of an uncompressed (ustar or pax) tar, or null. */
export function tarEntry(tar: Uint8Array, wanted: string): Uint8Array | null {
  const text = (start: number, length: number) => {
    const field = tar.subarray(start, start + length);
    const end = field.indexOf(0);
    return new TextDecoder().decode(end < 0 ? field : field.subarray(0, end));
  };
  let longName: string | null = null;
  for (let at = 0; at + 512 <= tar.length;) {
    if (tar.subarray(at, at + 512).every((b) => b === 0)) return null;
    const size = Number.parseInt(text(at + 124, 12).trim() || "0", 8);
    const type = String.fromCharCode(tar[at + 156] ?? 0);
    const prefix = text(at + 257, 6).startsWith("ustar") ? text(at + 345, 155) : "";
    const name = longName ?? (prefix ? `${prefix}/${text(at, 100)}` : text(at, 100));
    longName = null;
    const data = tar.subarray(at + 512, at + 512 + size);
    if (type === "x") {
      const path = /(?:^|\n)\d+ path=([^\n]*)\n/.exec(new TextDecoder().decode(data));
      if (path) longName = path[1]!;
    } else if (type === "L") longName = new TextDecoder().decode(data).replace(/\0+$/, "");
    else if ((type === "0" || type === "\0") && name === wanted) return data;
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return null;
}

/** The pinned frontend under `cache`, downloaded from npm and checked against the pin on first use. */
export async function ensureFrontend(cache: string, pin: ToolchainPin, log: (line: string) => void): Promise<string> {
  const dir = join(cache, `frontend-${pin.frontend.revision}`);
  const exe = join(dir, "package", "bin", "bun.exe");
  if (!existsSync(exe)) {
    mkdirSync(dir, { recursive: true });
    const archive = join(dir, `bun-windows-x64-${pin.frontend.packageVersion}.tgz`);
    if (!existsSync(archive)) log(`downloading the native AOT frontend ${pin.frontend.packageVersion} from npm`);
    await downloadVerified(pin.frontend.archiveUrl, archive, pin.frontend.archiveSha256);
    const bytes = tarEntry(Bun.gunzipSync(new Uint8Array(readFileSync(archive))), "package/bin/bun.exe");
    if (!bytes) throw new AotError(`${archive} holds no package/bin/bun.exe; delete ${dir} and build again`);
    mkdirSync(dirname(exe), { recursive: true });
    const staging = `${exe}.${process.pid}.partial`;
    writeFileSync(staging, bytes);
    try {
      renameSync(staging, exe);
    } catch (e) {
      rmSync(staging, { force: true });
      if (!existsSync(exe)) throw e;
    }
  }
  await verifyFile(exe, pin.frontend.executableSha256).catch(() => {
    throw new AotError(`${exe} is not the pinned native AOT frontend; delete ${dir} and build again`);
  });
  const revision = (await run([exe, "--revision"])).stdout.trim();
  if (!revision.endsWith(pin.frontend.revision.slice(0, 9)))
    throw new AotError(`${exe} reports revision ${revision}, not ${pin.frontend.revision}`);
  return exe;
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
  frontend: string;
  compiler: string;
  source: string;
  /** Holds the transformed input, the IR and the image. */
  work: string;
  /** Called as each of the two native steps starts. */
  phase?: (step: "frontend" | "compiler") => void;
}

export async function compileNative(options: CompileOptions): Promise<NativeImage> {
  const { frontend, compiler, source, work } = options;
  mkdirSync(work, { recursive: true });
  const input = join(work, "native-input.js");
  const ir = join(work, "app.ir");
  const image = join(work, "app.image");
  for (const stale of [ir, image, `${image}.entries`, `${image}.identity`]) rmSync(stale, { force: true });
  const wrapped = opaqueHost(source);
  writeFileSync(input, wrapped);

  options.phase?.("frontend");
  const front = await run(
    [
      frontend,
      "compile",
      input,
      "--no-builtin-modules",
      `--aot-out=${ir}`,
      "--aot-threads=1",
      "--aot-testing=NoExecutable",
      "--outfile",
      join(work, "unused.exe"),
    ],
    work,
  );
  if (front.code !== 0 || !existsSync(ir)) {
    throw new AotError(
      `the native AOT frontend rejected the app bundle (exit ${front.code}); its input is ${input}\n` +
        tail(front.stderr + front.stdout),
    );
  }

  options.phase?.("compiler");
  const compiled = await run([compiler, "compile", ir, image], work);
  if (compiled.code !== 0 || !existsSync(image)) {
    throw new AotError(
      `the native image compiler failed (exit ${compiled.code}) on ${ir}\n${tail(compiled.stderr + compiled.stdout)}`,
    );
  }
  const entries = readFileSync(`${image}.entries`, "utf8").trim().split(/\s+/).map(Number);
  const entry = entries[0];
  if (entries.length !== 1 || entry === undefined || !Number.isSafeInteger(entry) || entry < 1)
    throw new AotError(`the native image compiler reported entries ${JSON.stringify(entries)}, expected one`);
  const identity = readFileSync(`${image}.identity`);
  if (identity.length !== 16) throw new AotError("the native image compiler wrote an invalid image identity");
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
