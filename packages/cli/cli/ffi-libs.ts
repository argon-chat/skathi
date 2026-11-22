// The libraries "permissions.ffi" and the programs "permissions.process" place inside the app:
// `$APP/<path>` is a file in the app's folder under `sk dev` and the same path beside the built executable.

import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { ConfigError, type AppConfig } from "./config";
import type { Platform } from "./platform";

const EXTENSION: Readonly<Record<Platform, string>> = { windows: ".dll", macos: ".dylib", linux: ".so" };

/** `lib/sqlite3` is `lib/sqlite3.dll` on Windows, and `lib/sqlite3.so` or `lib/libsqlite3.so` elsewhere. */
export function libraryCandidates(path: string, platform: Platform): string[] {
  const slash = path.lastIndexOf("/");
  const file = path.slice(slash + 1);
  if (file.includes(".")) return [path];
  const extension = EXTENSION[platform];
  const plain = `${path}${extension}`;
  return platform === "windows" ? [plain] : [plain, `${path.slice(0, slash + 1)}lib${file}${extension}`];
}

/** Every `$APP` library of the config as the file it is in the app's folder, relative to it. */
export function appLibraries(config: AppConfig, platform: Platform): string[] {
  const found: string[] = [];
  for (const entry of config.permissions?.ffi ?? []) {
    if (!entry.path.startsWith("$APP/")) continue;
    const candidates = libraryCandidates(entry.path.slice("$APP/".length), platform);
    const file = candidates.find((candidate) => existsSync(join(config.appRoot, candidate)));
    if (!file) {
      throw new ConfigError(
        `sk.config.json "permissions.ffi": the library "${entry.name}" is not in the app: no ${candidates.join(" or ")} in ${config.appRoot}`,
      );
    }
    found.push(file);
  }
  return found;
}

/** Every `$APP` program of the config as the file it is in the app's folder, relative to it. */
export function appPrograms(config: AppConfig, platform: Platform): string[] {
  const found: string[] = [];
  for (const entry of config.permissions?.process ?? []) {
    if (!entry.path.startsWith("$APP/")) continue;
    const path = entry.path.slice("$APP/".length);
    const file = platform === "windows" && !path.slice(path.lastIndexOf("/") + 1).includes(".") ? `${path}.exe` : path;
    if (!existsSync(join(config.appRoot, file))) {
      throw new ConfigError(
        `sk.config.json "permissions.process": the program "${entry.name}" is not in the app: no ${file} in ${config.appRoot}`,
      );
    }
    found.push(file);
  }
  return found;
}

/** Copies every `$APP` library and program into `dir` at its own path; returns the files written, relative to `dir`. */
export function copyAppLibraries(config: AppConfig, dir: string, platform: Platform): string[] {
  const files = [...appLibraries(config, platform), ...appPrograms(config, platform)];
  for (const file of files) {
    const target = join(dir, file);
    mkdirSync(dirname(target), { recursive: true });
    copyFileSync(join(config.appRoot, file), target);
  }
  return files;
}
