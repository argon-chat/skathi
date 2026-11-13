// The hosts a vue-native release carries (.github/workflows/release.yml; read by cli/host-fetch.ts).
//
//   bun scripts/release-hosts.ts --out <dir> [--variant <name>]...
//     on each OS: cargo-build each variant (default: every published one), gzip it into <dir>
//     and describe it in <dir>/hosts-<platform>-<arch>.json
//   bun scripts/release-hosts.ts --merge <dir>
//     once, over every OS's output: <dir>/hosts.json
//   bun scripts/release-hosts.ts --check-tag <tag>
//     fails unless <tag> is v<package.json version>

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { VARIANTS } from "../cli/config";
import { hostAssetName, MANIFEST_NAME, sha256, type HostManifest, type HostManifestEntry } from "../cli/host-fetch";
import { CLI_VERSION, cargoBuildHost, checkHost, hostInfo } from "../cli/host";
import { currentPlatform } from "../cli/platform";

const { values } = parseArgs({
  options: {
    out: { type: "string" },
    merge: { type: "string" },
    variant: { type: "string", multiple: true },
    "check-tag": { type: "string" },
  },
});

if (values["check-tag"] !== undefined) {
  if (values["check-tag"] !== `v${CLI_VERSION}`) {
    console.error(`tag ${values["check-tag"]} is not v${CLI_VERSION}, the version in packages/vue-native/package.json`);
    process.exit(1);
  }
  process.exit(0);
}

if (values.merge) {
  const dir = resolve(values.merge);
  const parts = readdirSync(dir).filter((n) => /^hosts-.+\.json$/.test(n));
  const hosts = parts.flatMap((n) => JSON.parse(readFileSync(join(dir, n), "utf8")) as HostManifestEntry[]);
  for (const h of hosts) {
    if (!existsSync(join(dir, h.file))) throw new Error(`${h.file} is listed but missing from ${dir}`);
  }
  const manifest: HostManifest = { vueNative: CLI_VERSION, hosts };
  writeFileSync(join(dir, MANIFEST_NAME), JSON.stringify(manifest, null, 2) + "\n");
  for (const n of parts) rmSync(join(dir, n));
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
const entries: HostManifestEntry[] = [];
for (const variant of values.variant ?? VARIANTS) {
  const { exe, ms } = cargoBuildHost(variant);
  const info = hostInfo(exe);
  const problem = checkHost(exe, info, variant, false);
  if (problem) throw new Error(problem);
  const bytes = new Uint8Array(readFileSync(exe));
  const file = hostAssetName(platform, arch, variant);
  writeFileSync(join(out, file), Bun.gzipSync(bytes, { level: 9 }));
  entries.push({ platform, arch, variant, file, sha256: sha256(bytes), size: bytes.length, info });
  console.log(`[release] ${file}: ${(bytes.length / 1048576).toFixed(1)} MB, cargo ${(ms / 1000).toFixed(0)} s`);
}
writeFileSync(join(out, `hosts-${platform}-${arch}.json`), JSON.stringify(entries, null, 2) + "\n");
