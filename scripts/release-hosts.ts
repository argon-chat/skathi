// The hosts a skathi release carries (.github/workflows/release.yml; read by packages/cli/cli/host-fetch.ts).
//
//   bun scripts/release-hosts.ts --out <dir> [--variant <name>]... [--native-inspector <build.json>]
//     on each OS: cargo-build each variant (default: every published one and the dev host), gzip it into <dir>
//     and describe it in <dir>/hosts-<platform>-<arch>.json. With --native-inspector (Windows x64: the
//     build.json of scripts/aot/prepare-inspector.ts), every host carries the native AOT loader (an app
//     built with --aot runs from its image), every host with devtools embeds the inspector's native
//     image, and the sk-aot that compiled it is published beside them (<dir>/aot-<platform>-<arch>.json).
//   bun scripts/release-hosts.ts --merge <dir>
//     once, over every OS's output: <dir>/hosts.json
//   bun scripts/release-hosts.ts --check-tag <tag>
//     fails unless <tag> is v<version> of every npm package the release publishes

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { VARIANTS } from "../packages/cli/cli/config";
import { compilerInfo, compilerMismatch } from "../packages/cli/aot/native";
import {
  aotCompilerAssetName,
  hostAssetName,
  MANIFEST_NAME,
  sha256,
  type AotCompilerEntry,
  type HostManifest,
  type HostManifestEntry,
} from "../packages/cli/cli/host-fetch";
import { CLI_VERSION, cargoBuildHost, checkHost, DEV_HOST, hostInfo } from "../packages/cli/cli/host";
import { currentPlatform } from "../packages/cli/cli/platform";

const ROOT = resolve(import.meta.dir, "..");
const PUBLISHED = ["packages/runtime", "packages/cli"];

const { values } = parseArgs({
  options: {
    out: { type: "string" },
    merge: { type: "string" },
    variant: { type: "string", multiple: true },
    "native-inspector": { type: "string" },
    "check-tag": { type: "string" },
  },
});

if (values["check-tag"] !== undefined) {
  const tag = values["check-tag"];
  const wrong = PUBLISHED.map((dir) => {
    const version = (JSON.parse(readFileSync(join(ROOT, dir, "package.json"), "utf8")) as { version?: unknown })
      .version;
    return `v${String(version)}` === tag ? null : `${dir}/package.json is ${String(version)}`;
  }).filter((w) => w !== null);
  if (wrong.length) {
    console.error(`tag ${tag} does not match the published packages: ${wrong.join(", ")}`);
    process.exit(1);
  }
  process.exit(0);
}

if (values.merge) {
  const dir = resolve(values.merge);
  const parts = readdirSync(dir).filter((n) => /^hosts-.+\.json$/.test(n));
  const hosts = parts.flatMap((n) => JSON.parse(readFileSync(join(dir, n), "utf8")) as HostManifestEntry[]);
  const aotParts = readdirSync(dir).filter((n) => /^aot-.+\.json$/.test(n));
  const aot = aotParts.flatMap((n) => JSON.parse(readFileSync(join(dir, n), "utf8")) as AotCompilerEntry[]);
  for (const { file } of [...hosts, ...aot]) {
    if (!existsSync(join(dir, file))) throw new Error(`${file} is listed but missing from ${dir}`);
  }
  const manifest: HostManifest = { skathi: CLI_VERSION, hosts, ...(aot.length ? { aot } : {}) };
  writeFileSync(join(dir, MANIFEST_NAME), JSON.stringify(manifest, null, 2) + "\n");
  for (const n of [...parts, ...aotParts]) rmSync(join(dir, n));
  console.log(`[release] ${MANIFEST_NAME}: ${hosts.map((h) => `${h.platform}-${h.arch}-${h.variant}`).join(", ")}`);
  process.exit(0);
}

if (!values.out) {
  console.error("release-hosts.ts --out <dir> [--variant <name>]... | --merge <dir> | --check-tag <tag>");
  process.exit(1);
}

const out = resolve(values.out);
mkdirSync(out, { recursive: true });
const platform = currentPlatform();
const arch = process.arch;
// The receipt prepare-inspector.ts writes. Its paths go to every build, so redjs and the devtools
// bundle are not rebuilt between variants; only the hosts with devtools take the feature.
interface InspectorReceipt {
  schema: string;
  stage: string;
  manifest: string;
  bundle: string;
  table: string;
  compiler: string;
}
let inspector: InspectorReceipt | null = null;
if (values["native-inspector"]) {
  inspector = JSON.parse(readFileSync(resolve(values["native-inspector"]), "utf8")) as InspectorReceipt;
  if (inspector.schema !== "skathi.embedded-native-inspector" || inspector.stage !== "native-image") {
    throw new Error(`${values["native-inspector"]} is not a native inspector receipt from prepare-inspector.ts`);
  }
}
const inspectorEnv: Record<string, string> = inspector
  ? {
      REDJS_AOT_BUILTIN_TABLE_DIR: inspector.table,
      SK_DEVTOOLS_BUNDLE: inspector.bundle,
      SK_DEVTOOLS_NATIVE_MANIFEST: inspector.manifest,
      SK_DEVTOOLS_NATIVE_IMAGE_DLL: "",
    }
  : {};

// Every host must load what this sk-aot makes: `sk build --aot` downloads it to match them.
const aotCompiler = inspector ? { path: inspector.compiler, info: await compilerInfo(inspector.compiler) } : null;

const entries: HostManifestEntry[] = [];
for (const variant of values.variant ?? [...VARIANTS, DEV_HOST]) {
  const devtools = variant !== "no-devtools";
  const { exe, ms } = cargoBuildHost(variant, console.log, {
    env: inspectorEnv,
    features: inspector ? (devtools ? ["native-aot", "native-inspector"] : ["native-aot"]) : [],
  });
  const info = hostInfo(exe);
  const problem = checkHost(exe, info, variant, false);
  if (problem) throw new Error(problem);
  if (aotCompiler) {
    const mismatch = info.aot ? compilerMismatch(aotCompiler.info, info.aot) : "the host has no native AOT loader";
    if (mismatch) throw new Error(`${exe} (${variant}) and ${aotCompiler.path} disagree: ${mismatch}`);
  }
  const bytes = new Uint8Array(readFileSync(exe));
  const file = hostAssetName(platform, arch, variant);
  writeFileSync(join(out, file), Bun.gzipSync(bytes, { level: 9 }));
  entries.push({ platform, arch, variant, file, sha256: sha256(bytes), size: bytes.length, info });
  console.log(`[release] ${file}: ${(bytes.length / 1048576).toFixed(1)} MB, cargo ${(ms / 1000).toFixed(0)} s`);
}
writeFileSync(join(out, `hosts-${platform}-${arch}.json`), JSON.stringify(entries, null, 2) + "\n");
if (aotCompiler) {
  const bytes = new Uint8Array(readFileSync(aotCompiler.path));
  const file = aotCompilerAssetName(platform, arch);
  writeFileSync(join(out, file), Bun.gzipSync(bytes, { level: 9 }));
  const entry: AotCompilerEntry = {
    platform,
    arch,
    file,
    sha256: sha256(bytes),
    size: bytes.length,
    info: aotCompiler.info,
  };
  writeFileSync(join(out, `aot-${platform}-${arch}.json`), JSON.stringify([entry], null, 2) + "\n");
  console.log(`[release] ${file}: ${(bytes.length / 1048576).toFixed(1)} MB`);
}
