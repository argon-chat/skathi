import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

function findWorkspace(): string {
  let dir = dirname(fileURLToPath(import.meta.url));
  while (!existsSync(join(dir, "packages/runtime/src/engine.d.ts"))) {
    const parent = dirname(dir);
    if (parent === dir) throw new Error("Build the documentation inside the Skathi workspace.");
    dir = parent;
  }
  return dir;
}

export const workspaceRoot = findWorkspace();
