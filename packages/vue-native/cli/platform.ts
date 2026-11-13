// The OS a `vn build` packages for, and where that OS keeps its hosts.
//
// Everything the CLI does differently per OS that is a NAME — the cargo target
// triple, the host's file name, the cache folder — is one row of this table, so
// host.ts and dev.ts ask for a path instead of spelling one:
//
//   | | windows | macos |
//   |---|---|---|
//   | in-repo host | target/x86_64-pc-windows-msvc/<profile>/vn-app.exe | target/<aarch64|x86_64>-apple-darwin/<profile>/vn-app |
//   | cache | %LOCALAPPDATA%\vue-native\hosts\<version>\<variant>\vn-host.exe | ~/Library/Caches/vue-native/hosts/<version>/<variant>/vn-host |
//   | cargo | `--target` from .cargo/config.toml | `--target <triple>` (the checkout pins Windows) |
//   | output | a stamped `<exeName>.exe` (stamp.ts) | `<exeName>.app` with the payload in Contents/Resources (macos-bundle.ts) |
//
// What a platform produces is build-app.ts's `PACKAGERS`. Any OS that is not macOS keeps the Windows row.

import { homedir } from "node:os";
import { join } from "node:path";
import { ConfigError } from "./config";

export type Platform = "windows" | "macos";
export const PLATFORMS: readonly Platform[] = ["windows", "macos"];

/** The platform this CLI packages for when nobody says otherwise: the one it runs on. */
export function currentPlatform(): Platform {
  return process.platform === "darwin" ? "macos" : "windows";
}

/** `--platform`: `windows` / `win32`, `macos` / `darwin` / `mac`. */
export function parsePlatform(value: unknown): Platform {
  const v = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (v === "windows" || v === "win32" || v === "win") return "windows";
  if (v === "macos" || v === "darwin" || v === "mac") return "macos";
  throw new ConfigError(`--platform ${JSON.stringify(value)}: expected one of ${PLATFORMS.join(", ")}`);
}

export interface PlatformHosts {
  platform: Platform;
  /** The cargo target triple a host for this platform is built for. */
  triple: string;
  /** What cargo writes under `target/<triple>/<profile>/`. */
  exeFile: string;
  /** The host's file name in the cache. */
  cachedFile: string;
  /** Extra `cargo build` arguments that select `triple`. */
  cargoTarget: string[];
  /** `<root>/<version>/<variant>/<cachedFile>`. */
  cacheRoot(): string;
}

/** The row of the table above. `arch` is `process.arch`'s spelling. */
export function hostsFor(platform: Platform, arch: string = process.arch): PlatformHosts {
  if (platform === "macos") {
    const triple = arch === "x64" ? "x86_64-apple-darwin" : "aarch64-apple-darwin";
    return {
      platform,
      triple,
      exeFile: "vn-app",
      cachedFile: "vn-host",
      // .cargo/config.toml pins the Windows target, and a project config wins over the user's.
      cargoTarget: ["--target", triple],
      cacheRoot: () => join(homedir(), "Library", "Caches", "vue-native", "hosts"),
    };
  }
  return {
    platform,
    triple: "x86_64-pc-windows-msvc",
    exeFile: "vn-app.exe",
    cachedFile: "vn-host.exe",
    cargoTarget: [],
    cacheRoot: () => join(process.env.LOCALAPPDATA || join(homedir(), "AppData", "Local"), "vue-native", "hosts"),
  };
}

/** `target/<triple>/<profile>/<exeFile>` in the checkout at `repoRoot`. */
export function repoHostExe(repoRoot: string, hosts: PlatformHosts, profile: "release" | "debug"): string {
  return join(repoRoot, "target", hosts.triple, profile, hosts.exeFile);
}
