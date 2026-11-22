import { existsSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, sep } from "node:path";

export interface PackageInfo {
  name: string;
  version: string;
  dir: string;
  json: Record<string, unknown>;
}

const inside = (root: string, path: string) => {
  const rel = relative(root, path);
  return !rel.startsWith("..") && !isAbsolute(rel);
};

/** Attributes a file to the nearest package.json with a name, or to the app (null). */
export class Packages {
  private readonly dirs = new Map<string, PackageInfo | null>();

  private floor: [number, number] | null | undefined;

  constructor(readonly appRoot: string) {}

  /** The lowest `@skathi-native/runtime` the app's package.json admits, as [major, minor]; null when it does not say. */
  runtimeFloor(): [number, number] | null {
    if (this.floor !== undefined) return this.floor;
    this.floor = null;
    try {
      const json = JSON.parse(readFileSync(join(this.appRoot, "package.json"), "utf8")) as Record<string, unknown>;
      for (const key of ["dependencies", "devDependencies", "peerDependencies"]) {
        const range = (json[key] as Record<string, unknown> | undefined)?.["@skathi-native/runtime"];
        const lowest =
          typeof range === "string" && !range.includes("||") && /^\s*(?:[\^~=]|>=)?\s*v?(\d+)\.(\d+)/.exec(range);
        if (lowest) {
          this.floor = [Number(lowest[1]), Number(lowest[2])];
          break;
        }
      }
    } catch {}
    return this.floor;
  }

  ownerOf(file: string): PackageInfo | null {
    if (inside(this.appRoot, file) && !relative(this.appRoot, file).split(sep).includes("node_modules")) return null;
    return this.lookup(dirname(file));
  }

  private lookup(dir: string): PackageInfo | null {
    const known = this.dirs.get(dir);
    if (known !== undefined) return known;
    let found: PackageInfo | null = null;
    const path = join(dir, "package.json");
    if (dir !== this.appRoot && existsSync(path)) {
      try {
        const json = JSON.parse(readFileSync(path, "utf8")) as unknown;
        if (json && typeof json === "object" && !Array.isArray(json)) {
          const record = json as Record<string, unknown>;
          if (typeof record.name === "string" && record.name)
            found = {
              name: record.name,
              version: typeof record.version === "string" ? record.version : "0.0.0",
              dir,
              json: record,
            };
        }
      } catch {}
    }
    if (!found && dir !== this.appRoot) {
      const parent = dirname(dir);
      found = parent === dir ? null : this.lookup(parent);
    }
    this.dirs.set(dir, found);
    return found;
  }
}
