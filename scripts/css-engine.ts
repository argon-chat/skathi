// Regenerate with the wasm32-unknown-unknown Rust target and Binaryen's wasm-metadce/wasm-opt on PATH.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { brotliCompressSync, brotliDecompressSync } from "node:zlib";

const root = resolve(import.meta.dir, "..");
const target = resolve(root, process.env.CARGO_TARGET_DIR ?? "target");
const output = join(root, "packages/cli/check/sk-css.wasm.br");
const sourceDir = "crates/sk-css/src";
const sources = readdirSync(join(root, sourceDir), { recursive: true, withFileTypes: true })
  .filter((e) => e.isFile())
  .map((e) =>
    join(e.parentPath, e.name)
      .slice(root.length + 1)
      .replaceAll("\\", "/"),
  );
const hash = createHash("sha256");
for (const path of [...sources, "crates/sk-css/Cargo.toml", "crates/sk-css/examples/css-check.rs"].sort()) {
  hash
    .update(path)
    .update("\0")
    .update(readFileSync(join(root, path), "utf8").replaceAll("\r\n", "\n"))
    .update("\0");
}
const digest = hash.digest();
const section = "sk-css-sources";
if (process.argv.includes("--check") || process.argv.includes("--ensure")) {
  if (existsSync(output)) {
    const module = new WebAssembly.Module(brotliDecompressSync(readFileSync(output)));
    const [stamp] = WebAssembly.Module.customSections(module, section);
    if (stamp && digest.equals(Buffer.from(stamp))) process.exit(0);
  }
  if (process.argv.includes("--check"))
    throw new Error("sk-css changed. Regenerate the CLI checker with bun scripts/css-engine.ts.");
}
execFileSync(
  "cargo",
  ["build", "--locked", "--release", "--target", "wasm32-unknown-unknown", "-p", "sk-css", "--example", "css-check"],
  { cwd: root, stdio: "inherit" },
);
const wasm = join(target, "wasm32-unknown-unknown/release/examples/css_check.wasm");
const graph = join(target, "css-check-exports.json");
const pruned = join(target, "css-check-pruned.wasm");
const optimized = join(target, "css-check-opt.wasm");
writeFileSync(graph, JSON.stringify(["check", "memory"].map((name) => ({ name, export: name, root: true }))));
execFileSync("wasm-metadce", ["--all-features", "--graph-file=" + graph, wasm, "-o", pruned], {
  stdio: ["ignore", "ignore", "inherit"],
});
execFileSync(
  "wasm-opt",
  [
    "--all-features",
    "--remove-unused-module-elements",
    "--strip-debug",
    "--strip-producers",
    "-Oz",
    pruned,
    "-o",
    optimized,
  ],
  { stdio: "inherit" },
);
const bytes = readFileSync(optimized);
const module = new WebAssembly.Module(bytes);
for (const i of WebAssembly.Module.imports(module)) {
  if (
    (i.module === "sk" && ["read", "write"].includes(i.name)) ||
    (i.module === "__wbindgen_placeholder__" && /^__wbg___wbindgen_throw_[a-f0-9]+$/.test(i.name))
  )
    continue;
  throw new Error("Unexpected checker import: " + i.module + "." + i.name);
}
const stamp = Buffer.concat([
  Buffer.from([0, section.length + 1 + digest.length, section.length]),
  Buffer.from(section),
  digest,
]);
writeFileSync(output, brotliCompressSync(Buffer.concat([bytes, stamp])));
