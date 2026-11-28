// Builds packages/cli/check/sk-css.wasm.br, the CSS checker `sk check` runs: sk-css's css-check example for
// wasm32-unknown-unknown, brotli-compressed. A --release build (the npm package) is first trimmed with Binaryen (the
// root `binaryen` devDependency), which takes about a minute. Dev builds use Cargo's WASM output directly.
// A custom section stamps the module with a hash of every input and the kind of build, so
// --ensure rebuilds only a missing or stale module, and a dev module never passes for a release one.
//
//   bun scripts/css-engine.ts [--release]            build
//   bun scripts/css-engine.ts --ensure [--release]   build when missing or stale

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { brotliCompressSync, brotliDecompressSync, constants } from "node:zlib";

const root = resolve(import.meta.dir, "..");
const target = resolve(root, process.env.CARGO_TARGET_DIR ?? "target");
const output = join(root, "packages/cli/check/sk-css.wasm.br");
const section = "sk-css-sources";
const rel = (path: string) => relative(root, path).replaceAll("\\", "/");

interface Inputs {
  key: string;
  /** Manifests that decide the dependency graph; the key hashes them. */
  manifests: string[];
  dirs: string[];
  files: string[];
}

function graphKey(manifests: string[]) {
  const hash = createHash("sha256");
  for (const path of manifests) hash.update(path).update("\0").update(read(path)).update("\0");
  return hash.digest("hex");
}

function read(path: string): Buffer {
  try {
    return readFileSync(join(root, path));
  } catch {
    return Buffer.alloc(0);
  }
}

interface Metadata {
  packages: {
    id: string;
    name: string;
    source: string | null;
    manifest_path: string;
    targets: { kind: string[]; src_path: string }[];
  }[];
  resolve: { nodes: { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] }[] };
}

/** sk-css and the path dependencies a wasm32 build of the example compiles, from `cargo metadata`. */
function cargoInputs(): Inputs {
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--filter-platform", "wasm32-unknown-unknown"],
      { cwd: root, encoding: "utf8", maxBuffer: 256 << 20, stdio: ["ignore", "pipe", "inherit"] },
    ),
  ) as Metadata;
  const packages = new Map(metadata.packages.map((p) => [p.id, p]));
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const start = metadata.packages.find((p) => p.name === "sk-css" && p.source === null);
  if (!start) throw new Error("cargo metadata has no sk-css package");
  const seen = new Set<string>();
  const queue = [start.id];
  for (let id = queue.pop(); id !== undefined; id = queue.pop()) {
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) {
      if (dep.dep_kinds.some((k) => k.kind !== "dev" || id === start.id)) queue.push(dep.pkg);
    }
  }
  const manifests = new Set(["Cargo.lock", "Cargo.toml"]);
  const dirs = new Set<string>();
  const files = new Set(["crates/sk-css/examples/css-check.rs"]);
  for (const id of seen) {
    const pkg = packages.get(id);
    if (!pkg || pkg.source !== null) continue;
    manifests.add(rel(pkg.manifest_path));
    for (let dir = dirname(dirname(pkg.manifest_path)); dir.length > root.length; dir = dirname(dir)) {
      const manifest = join(dir, "Cargo.toml");
      if (existsSync(manifest) && /^\[workspace\]/m.test(readFileSync(manifest, "utf8"))) manifests.add(rel(manifest));
    }
    for (const t of pkg.targets) {
      if (t.kind.includes("custom-build")) files.add(rel(t.src_path));
      else if (!t.kind.some((k) => ["example", "test", "bench"].includes(k))) dirs.add(rel(dirname(t.src_path)));
    }
  }
  const sorted = [...manifests].sort();
  return { key: graphKey(sorted), manifests: sorted, dirs: [...dirs].sort(), files: [...files].sort() };
}

function inputs(): Inputs {
  const cache = join(target, "css-check-inputs.json");
  try {
    const cached = JSON.parse(readFileSync(cache, "utf8")) as Inputs;
    if (cached.key === graphKey(cached.manifests)) return cached;
  } catch {}
  const fresh = cargoInputs();
  mkdirSync(target, { recursive: true });
  writeFileSync(cache, JSON.stringify(fresh));
  return fresh;
}

function binaryen(): { dir: string; version: string } {
  const manifest = Bun.resolveSync("binaryen/package.json", root);
  return {
    dir: join(dirname(manifest), "bin"),
    version: (JSON.parse(readFileSync(manifest, "utf8")) as { version: string }).version,
  };
}

function sourceDigest(): Buffer {
  const { manifests, dirs, files } = inputs();
  const all = new Set([...manifests, ...files, "rust-toolchain.toml", ".cargo/config.toml", "scripts/css-engine.ts"]);
  const contents = new Map<string, Buffer>();
  const included = /include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)/g;
  for (const dir of dirs) {
    for (const entry of readdirSync(join(root, dir), { recursive: true, withFileTypes: true })) {
      if (!entry.isFile()) continue;
      const path = join(entry.parentPath, entry.name);
      const text = readFileSync(path);
      contents.set(rel(path), text);
      if (!entry.name.endsWith(".rs")) continue;
      for (const [, file] of text.toString("utf8").matchAll(included)) {
        const resolved = resolve(entry.parentPath, file!);
        if (resolved.startsWith(root) && existsSync(resolved)) all.add(rel(resolved));
      }
    }
  }
  for (const path of contents.keys()) all.add(path);
  const hash = createHash("sha256");
  for (const path of [...all].sort()) {
    hash
      .update(path)
      .update("\0")
      .update(contents.get(path) ?? read(path))
      .update("\0");
  }
  return hash.digest();
}

const stampOf = (sources: Buffer, release: boolean) =>
  createHash("sha256")
    .update(release ? "release, binaryen " + binaryen().version : "dev")
    .update("\0")
    .update(sources)
    .digest();

/** The custom section build() appends last. */
const stampSection = (stamp: Buffer) =>
  Buffer.concat([Buffer.from([0, section.length + 1 + stamp.length, section.length]), Buffer.from(section), stamp]);

function current(accepted: Buffer[]): boolean {
  try {
    const module = brotliDecompressSync(readFileSync(output));
    return accepted.some((stamp) => module.subarray(-stamp.length - section.length - 3).equals(stampSection(stamp)));
  } catch {
    return false;
  }
}

function run(command: string, args: string[], quiet: boolean, failure: string) {
  const r = spawnSync(command, args, {
    cwd: root,
    stdio: quiet ? ["ignore", "pipe", "pipe"] : ["ignore", 2, 2],
    maxBuffer: 64 << 20,
  });
  if (r.status === 0) return;
  if (quiet) process.stderr.write(Buffer.concat([r.stdout ?? Buffer.alloc(0), r.stderr ?? Buffer.alloc(0)]));
  throw new Error(failure + (r.error ? ": " + r.error.message : ""));
}

function build(sources: Buffer, release: boolean, quiet: boolean) {
  // Parallel code generation keeps the local Wasm build quick.
  const profile = release
    ? ["--release"]
    : [
        "--profile",
        "css-dev",
        ...['inherits="release"', "lto=false", "codegen-units=256"].flatMap((s) => [
          "--config",
          "profile.css-dev." + s,
        ]),
      ];
  run(
    "cargo",
    ["build", "--locked", ...profile, "--target", "wasm32-unknown-unknown", "-p", "sk-css", "--example", "css-check"],
    quiet,
    "cargo build of the css-check example failed. It needs the wasm32-unknown-unknown target: rustup target add wasm32-unknown-unknown",
  );
  const wasm = join(target, "wasm32-unknown-unknown", release ? "release" : "css-dev", "examples/css_check.wasm");
  const bytes = release ? optimize(wasm, quiet) : readFileSync(wasm);
  for (const i of WebAssembly.Module.imports(new WebAssembly.Module(bytes))) {
    if (
      (i.module === "sk" && ["read", "write"].includes(i.name)) ||
      (i.module === "__wbindgen_placeholder__" && /^__wbg___wbindgen_throw_[a-f0-9]+$/.test(i.name)) ||
      (!release && i.module.startsWith("__wbindgen_"))
    )
      continue;
    throw new Error("Unexpected checker import: " + i.module + "." + i.name);
  }
  const module = Buffer.concat([bytes, stampSection(stampOf(sources, release))]);
  const quality = release ? constants.BROTLI_MAX_QUALITY : 4;
  writeFileSync(output + ".tmp", brotliCompressSync(module, { params: { [constants.BROTLI_PARAM_QUALITY]: quality } }));
  renameSync(output + ".tmp", output);
}

function optimize(wasm: string, quiet: boolean) {
  const graph = join(target, "css-check-exports.json");
  const pruned = join(target, "css-check-pruned.wasm");
  const optimized = join(target, "css-check-opt.wasm");
  const tools = binaryen().dir;
  // Binaryen's tools are Emscripten builds; Node runs wasm-opt about a third faster than bun.
  const js = Bun.which("node") ?? process.execPath;
  // JavaScriptCore cannot parse the compact import section.
  const features = ["--all-features", "--disable-compact-imports"];
  writeFileSync(graph, JSON.stringify(["check", "memory"].map((name) => ({ name, export: name, root: true }))));
  run(
    js,
    [join(tools, "wasm-metadce"), ...features, "--graph-file=" + graph, wasm, "-o", pruned],
    true,
    "wasm-metadce failed",
  );
  run(
    js,
    [
      join(tools, "wasm-opt"),
      ...features,
      "--remove-unused-module-elements",
      "--strip-debug",
      "--strip-producers",
      "-Oz",
      pruned,
      "-o",
      optimized,
    ],
    quiet,
    "wasm-opt failed",
  );
  return readFileSync(optimized);
}

/** Builds the checker when it is missing or stale, saying so in one line on stderr. A release module serves dev too. */
export function ensureCssEngine(release = false) {
  const sources = sourceDigest();
  if (current(release ? [stampOf(sources, true)] : [stampOf(sources, false), stampOf(sources, true)])) return;
  console.error(`Building the CSS checker (sk-css for wasm32-unknown-unknown${release ? ", optimized" : ""})`);
  build(sources, release, true);
}

if (import.meta.main) {
  try {
    const release = process.argv.includes("--release");
    if (process.argv.includes("--ensure")) ensureCssEngine(release);
    else build(sourceDigest(), release, false);
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    process.exit(1);
  }
}
