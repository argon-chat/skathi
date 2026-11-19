// The hosts a skathi release carries (.github/workflows/release.yml; read by packages/cli/cli/host-fetch.ts).
//
//   bun scripts/release-hosts.ts --out <dir> [--variant <name>]... [--native-inspector <build.json>] [--no-jit <dir>]
//     on each OS: cargo-build each variant (default: every published one and the dev host), gzip it into <dir>
//     and describe it in <dir>/hosts-<platform>-<arch>.json. The prod hosts link only the root ICU locale;
//     the full ICU data they leave out goes beside them, gzipped, for `sk build` to ship as icu.skpk. With --native-inspector (Windows x64: the
//     build.json of scripts/aot/prepare-inspector.ts), every host carries the native AOT loader (an app
//     built with --aot runs from its image), every host with devtools embeds the inspector's native
//     image, and the sk-aot that compiled it is published beside them (<dir>/aot-<platform>-<arch>.json).
//     With --no-jit too (the directory of scripts/aot/no-jit.ts --out), the aot-nojit workload is built.
//     On Windows each host's link map is checked: no host links the image compiler, aot-nojit links
//     none of the entry points the inventories of scripts/aot/no-jit.ts list (the JIT's, WebAssembly's)
//     while no-devtools links them, and aot-nojit exports what no-devtools does.
//   bun scripts/release-hosts.ts --merge <dir>
//     once, over every OS's output: <dir>/hosts.json
//   bun scripts/release-hosts.ts --check-tag <tag>
//     fails unless <tag> is v<version> of every npm package the release publishes

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { NO_JIT_VARIANT, platformVariants } from "../packages/cli/cli/config";
import { inventoryPath, REMOVED, type Inventory } from "./aot/no-jit";
import { compilerInfo, compilerMismatch } from "../packages/cli/aot/native";
import {
  aotCompilerAssetName,
  hostAssetName,
  icuAssetName,
  MANIFEST_NAME,
  sha256,
  type AotCompilerEntry,
  type HostManifest,
  type HostManifestEntry,
} from "../packages/cli/cli/host-fetch";
import { CLI_VERSION, cargoBuildHost, checkHost, DEV_HOST, hostInfo, icuDataPath } from "../packages/cli/cli/host";
import { currentPlatform } from "../packages/cli/cli/platform";

const ROOT = resolve(import.meta.dir, "..");
const PUBLISHED = ["packages/runtime", "packages/cli", "packages/create-skathi"];

const { values } = parseArgs({
  options: {
    out: { type: "string" },
    merge: { type: "string" },
    variant: { type: "string", multiple: true },
    "native-inspector": { type: "string" },
    "no-jit": { type: "string" },
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
  for (const { file } of [...hosts, ...hosts.flatMap((h) => (h.icu ? [h.icu] : [])), ...aot]) {
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

const noJit = values["no-jit"] ? resolve(values["no-jit"]) : null;
const variants = values.variant ?? [
  ...platformVariants(platform).filter((v) => v !== NO_JIT_VARIANT || (inspector && noJit)),
  DEV_HOST,
];
if (variants.includes(NO_JIT_VARIANT) && !(inspector && noJit)) {
  throw new Error(`the ${NO_JIT_VARIANT} workload needs --native-inspector (its AOT table) and --no-jit <dir>`);
}

// What a Windows host's link map must show, and what each host exports.
const maps = platform === "windows" ? mkdtempSync(join(tmpdir(), "sk-host-maps-")) : null;
const removedEntries = REMOVED.map((feature) => ({
  feature,
  names: Object.keys((JSON.parse(readFileSync(inventoryPath(feature), "utf8")) as Inventory).entries),
}));
const exportsOf = new Map<string, string[]>();
function checkLink(variant: string, exe: string, map: string): void {
  const linked = readFileSync(map, "latin1");
  const has = (symbol: string) => linked.includes(` ${symbol} `);
  if (/\s\?compileProgram@AOT@JSC@@/.test(linked)) throw new Error(`${variant}: the host links the image compiler`);
  const entryUse = removedEntries.map(({ feature, names }) => ({ feature, names, kept: names.filter(has) }));
  for (const { feature, names, kept } of entryUse) {
    if (variant === NO_JIT_VARIANT && kept.length) {
      throw new Error(`${variant}: the host links ${kept.length} ${feature.name} entry points, among them ${kept[0]}`);
    }
    if (variant === "no-devtools" && inspector && kept.length !== names.length) {
      throw new Error(
        `${variant}: ${names.length - kept.length} of scripts/aot/${feature.inventory} are not in the map; it names other symbols than the link`,
      );
    }
  }
  const listing = spawnSync("llvm-readobj", ["--coff-exports", exe], { encoding: "utf8" });
  if (listing.status !== 0) throw new Error(`llvm-readobj --coff-exports ${exe} failed: ${listing.stderr}`);
  const names = [...listing.stdout.matchAll(/^\s*Name: (\S+)\s*$/gm)].map((m) => m[1]!).sort();
  if (names.some((n) => n.startsWith("rjs_aot_compil")))
    throw new Error(`${variant}: the host exports the image compiler`);
  exportsOf.set(variant, names);
  const reference = exportsOf.get("no-devtools");
  if (variant === NO_JIT_VARIANT && reference && reference.join("\n") !== names.join("\n")) {
    throw new Error(`${variant}: the host exports other symbols than no-devtools: ${names.join(", ")}`);
  }
  const counts = entryUse.map(({ feature, names: all, kept }) => `${kept.length} of ${all.length} ${feature.name}`);
  console.log(`[release] ${variant}: ${counts.join(", ")} entry points linked, ${names.length} exports`);
}

const entries: HostManifestEntry[] = [];
for (const variant of variants) {
  const devtools = variant === "standard" || variant === DEV_HOST;
  const map = maps && join(maps, `${variant}.map`);
  const { exe, ms } = cargoBuildHost(variant, console.log, {
    env: {
      ...inspectorEnv,
      ...(variant === NO_JIT_VARIANT && noJit ? { REDJS_NO_JIT_DIR: noJit } : {}),
      ...(map ? { SK_HOST_LINK_MAP: map } : {}),
    },
    features: inspector ? (devtools ? ["native-aot", "native-inspector"] : ["native-aot"]) : [],
  });
  if (map) checkLink(variant, exe, map);
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
  const entry: HostManifestEntry = { platform, arch, variant, file, sha256: sha256(bytes), size: bytes.length, info };
  const icu = icuDataPath({ exe, info });
  if (icu) {
    if (!existsSync(icu)) throw new Error(`${exe} links only the root ICU locale, but cargo left no ${icu} beside it`);
    const data = new Uint8Array(readFileSync(icu));
    const icuFile = icuAssetName(platform, arch);
    if (!existsSync(join(out, icuFile))) writeFileSync(join(out, icuFile), Bun.gzipSync(data, { level: 9 }));
    entry.icu = { file: icuFile, sha256: sha256(data), size: data.length };
  }
  entries.push(entry);
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
