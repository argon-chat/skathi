// The libraries "permissions.ffi" places inside the app: `$APP/<path>` is a file in the app's folder
// under `sk dev` and the same path beside the built executable.

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

/** Copies every `$APP` library into `dir` at its own path; returns the files written, relative to `dir`. */
export function copyAppLibraries(config: AppConfig, dir: string, platform: Platform): string[] {
  const written: string[] = [];
  for (const entry of config.permissions?.ffi ?? []) {
    if (!entry.path.startsWith("$APP/")) continue;
    const candidates = libraryCandidates(entry.path.slice("$APP/".length), platform);
    const found = candidates.find((candidate) => existsSync(join(config.appRoot, candidate)));
    if (!found) {
      throw new ConfigError(
        `sk.config.json "permissions.ffi": the library "${entry.name}" is not in the app: no ${candidates.join(" or ")} in ${config.appRoot}`,
      );
    }
    const target = join(dir, found);
    mkdirSync(dirname(target), { recursive: true });
    copyFileSync(join(config.appRoot, found), target);
    written.push(found);
  }
  return written;
}
