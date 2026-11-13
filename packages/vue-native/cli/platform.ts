// The OS a `vn build` packages for, and where that OS keeps its hosts. Everything that differs per
// OS by NAME (cargo target triple, host file name, cache folder) is one row of this table, so
// host.ts and dev.ts ask for a path instead of spelling one. What a platform produces is
// build-app.ts's `PACKAGERS`. Any OS that is not macOS keeps the Windows row.

import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { ConfigError } from "./config";

export type Platform = "windows" | "macos";
export const PLATFORMS: readonly Platform[] = ["windows", "macos"];

export function currentPlatform(): Platform {
  return process.platform === "darwin" ? "macos" : "windows";
}

export function parsePlatform(value: unknown): Platform {
  const v = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (v === "windows" || v === "win32" || v === "win") return "windows";
  if (v === "macos" || v === "darwin" || v === "mac") return "macos";
  throw new ConfigError(`--platform ${JSON.stringify(value)}: expected one of ${PLATFORMS.join(", ")}`);
}

export interface PlatformHosts {
  platform: Platform;
  triple: string;
  exeFile: string;
  cachedFile: string;
  cargoTarget: string[];
  cacheRoot(): string;
}

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

/** Where cargo puts the checkout's host: under CARGO_TARGET_DIR when that is set, as cargo does. */
export function repoHostExe(repoRoot: string, hosts: PlatformHosts, profile: "release" | "debug"): string {
  const target = process.env.CARGO_TARGET_DIR
    ? resolve(repoRoot, process.env.CARGO_TARGET_DIR)
    : join(repoRoot, "target");
  return join(target, hosts.triple, profile, hosts.exeFile);
}
