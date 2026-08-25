// Fetch-and-pin the Slang compiler, PEWPEW's `toolkit.manifest.ts` pattern.
//
// Usage: bun fetch-slang.ts [--force]
//
// Downloads the release named in `crates/vn-effects/tools/slang.manifest.json`,
// verifies its sha256 BEFORE extracting anything, and unpacks the three files
// `shaders.ts` needs into `tools/slang/win-x64/` (gitignored).
//
// You need this ONLY to author or edit a `.slang` source. The generated HLSL is
// checked in, so `cargo build`, CI and every contributor who is not writing a
// shader need neither this download nor a second SDK — see docs/SHADERS.md
// §2.3 for why that trade was made.
//
// Vendoring the 35 MB into git was the alternative (open question 3). Fetching
// won for exactly the reason above: nothing in the build depends on it.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

const REPO = resolve(import.meta.dir, "../..");
const MANIFEST = join(REPO, "crates/vn-effects/tools/slang.manifest.json");
const DEST = join(REPO, "tools/slang/win-x64");

type Asset = {
  url: string;
  sha256: string;
  root?: string;
  include?: string[];
};

type Manifest = {
  tool: string;
  exe: string;
  version: string;
  licenses?: string[];
  assets: Record<string, Asset>;
  knownIssues?: string[];
};

function platformKey(): string {
  if (process.platform !== "win32") {
    // macos-aarch64 / linux-x86_64 zips exist upstream; pin them here when a
    // non-Windows renderer needs them. Refusing loudly beats silently
    // downloading the wrong archive.
    throw new Error(
      `no pinned slangc asset for ${process.platform}/${process.arch}. ` +
        `Add one to ${MANIFEST} (upstream publishes macos-aarch64 and linux-x86_64).`,
    );
  }
  return process.arch === "arm64" ? "win-arm64" : "win-x64";
}

async function main() {
  const force = process.argv.includes("--force");
  const manifest: Manifest = JSON.parse(readFileSync(MANIFEST, "utf8"));
  const key = platformKey();
  const asset = manifest.assets[key];
  if (!asset) {
    throw new Error(`${MANIFEST} pins no asset for ${key}`);
  }

  const stamp = join(DEST, ".version");
  if (
    !force &&
    existsSync(join(DEST, `${manifest.exe}.exe`)) &&
    existsSync(stamp) &&
    readFileSync(stamp, "utf8").trim() === manifest.version
  ) {
    console.log(`slangc ${manifest.version} already at ${DEST} (use --force to refetch)`);
    return;
  }

  console.log(`fetching ${asset.url}`);
  const response = await fetch(asset.url);
  if (!response.ok) {
    throw new Error(`download failed: ${response.status} ${response.statusText}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());

  // Verify BEFORE anything is written anywhere executable.
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== asset.sha256) {
    throw new Error(
      `sha256 mismatch for ${asset.url}\n  expected ${asset.sha256}\n  got      ${digest}\n` +
        `Refusing to extract. If upstream legitimately re-cut the release, update ${MANIFEST} ` +
        `deliberately and re-run \`bun shaders.ts\` so any codegen change lands as a reviewable diff.`,
    );
  }
  console.log(`sha256 ok (${(bytes.length / 1024 / 1024).toFixed(1)} MB)`);

  const work = mkdtempSync(join(tmpdir(), "vn-slang-"));
  try {
    const archive = join(work, basename(new URL(asset.url).pathname));
    writeFileSync(archive, bytes);

    const extracted = join(work, "x");
    mkdirSync(extracted, { recursive: true });
    // `tar` ships with Windows 10+ and reads zip; no extra dependency.
    const untar = spawnSync("tar", ["-xf", archive, "-C", extracted], { encoding: "utf8" });
    if (untar.status !== 0) {
      throw new Error(`extract failed: ${untar.stderr || untar.stdout}`);
    }

    mkdirSync(DEST, { recursive: true });
    const sourceDir = asset.root ? join(extracted, asset.root) : extracted;
    const wanted = asset.include ?? readdirSync(sourceDir);
    for (const name of wanted) {
      const from = join(sourceDir, name);
      if (!existsSync(from)) {
        throw new Error(`archive is missing ${asset.root ?? "."}/${name}`);
      }
      renameSync(from, join(DEST, name));
    }
    for (const license of manifest.licenses ?? []) {
      const from = join(extracted, license);
      if (existsSync(from)) renameSync(from, join(DEST, license));
    }
    writeFileSync(stamp, `${manifest.version}\n`);

    const total = wanted.reduce((sum, n) => sum + statSync(join(DEST, n)).size, 0);
    console.log(
      `slangc ${manifest.version} -> ${DEST}\n  ` +
        `${wanted.join(", ")} (${(total / 1024 / 1024).toFixed(1)} MB)`,
    );
    if (manifest.knownIssues?.length) {
      console.log("\nknown slangc CLI issues shaders.ts already works around:");
      for (const issue of manifest.knownIssues) console.log(`  * ${issue}`);
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(String(error instanceof Error ? error.message : error));
  process.exit(1);
});
