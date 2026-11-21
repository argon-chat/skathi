// `sk dev` on macOS runs the host from `<appRoot>/dist/.sk-dev-app/<exeName>.app`: Info.plist from
// sk.config.json (the usage descriptions TCC requires, the name and icon in the Dock) under a bundle
// id of its own, so the dev app's permissions are not the released one's. The host is copied in only
// when it changed; a new copy is a new code identity, and TCC asks again.

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { macInfoFor } from "./build-app";
import type { AppConfig } from "./config";
import { AD_HOC, codesignArgs, ICON_NAME, icnsFrom, infoPlist, plistXml } from "./macos-bundle";

export interface DevBundle {
  app: string;
  exe: string;
  changed: boolean;
}

const stampOf = (file: string) => {
  const s = statSync(file);
  return `${s.size}:${Math.trunc(s.mtimeMs)}`;
};

function writeIfChanged(path: string, body: string | Uint8Array): boolean {
  const next = typeof body === "string" ? Buffer.from(body) : Buffer.from(body);
  if (existsSync(path) && readFileSync(path).equals(next)) return false;
  writeFileSync(path, next);
  return true;
}

export function devBundle(options: {
  config: AppConfig;
  host: string;
  /** Files the host expects beside itself, copied to Contents/Frameworks (the native inspector image). */
  frameworks?: string[];
  codesign?: string | null;
  log?: (line: string) => void;
}): DevBundle {
  const { config, host } = options;
  const m = config.macos;
  const app = join(config.appRoot, "dist", ".sk-dev-app", `${m.exeName}.app`);
  const contents = join(app, "Contents");
  // The host's own file name: the storage folder and keychain entry of a dev app follow it.
  const exeName = basename(host);
  const exe = join(contents, "MacOS", exeName);
  for (const dir of ["MacOS", "Resources", "Frameworks"]) mkdirSync(join(contents, dir), { recursive: true });

  // Source stamps, not the copies': signing rewrites the copied executable.
  const stampsPath = join(app, "..", `${m.exeName}.sources.json`);
  let stamps: Record<string, string> = {};
  try {
    stamps = JSON.parse(readFileSync(stampsPath, "utf8")) as Record<string, string>;
  } catch {}
  let changed = false;
  const copy = (from: string, to: string) => {
    const stamp = `${from}|${stampOf(from)}`;
    if (existsSync(to) && stamps[to] === stamp) return;
    rmSync(to, { force: true });
    copyFileSync(from, to);
    stamps[to] = stamp;
    changed = true;
  };
  copy(host, exe);
  for (const file of options.frameworks ?? []) {
    if (existsSync(file)) copy(file, join(contents, "Frameworks", basename(file)));
  }

  let icon: ReturnType<typeof icnsFrom> | null = null;
  try {
    icon = m.iconPath ? icnsFrom(m.iconPath) : null;
  } catch (e) {
    options.log?.(`no Dock icon: ${(e as Error).message}`);
  }
  if (icon) changed = writeIfChanged(join(contents, "Resources", `${ICON_NAME}.icns`), icon.bytes) || changed;
  const info = { ...macInfoFor(config, !!icon), bundleId: `${m.bundleId}.dev`, executable: exeName };
  changed = writeIfChanged(join(contents, "Info.plist"), plistXml(infoPlist(info))) || changed;
  changed = writeIfChanged(join(contents, "PkgInfo"), "APPL????") || changed;

  // An unsigned bundle has its Info.plist unbound; ad hoc keeps TCC's identity to this bundle.
  if (changed && options.codesign) {
    const r = spawnSync(options.codesign, codesignArgs(app, AD_HOC, null), { encoding: "utf8" });
    if (r.status !== 0) options.log?.(`codesign of the dev app failed: ${(r.stderr ?? "").trim()}`);
  }
  if (changed) writeFileSync(stampsPath, JSON.stringify(stamps));
  return { app, exe, changed };
}
