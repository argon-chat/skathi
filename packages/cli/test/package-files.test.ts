import { expect, test } from "bun:test";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, posix, resolve } from "node:path";
import { Glob } from "bun";

const root = resolve(import.meta.dir, "..");
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { files: string[] };

function shipped(): Set<string> {
  const out = new Set<string>(["package.json"]);
  for (const entry of pkg.files) {
    const path = join(root, entry);
    if (entry.includes("*")) {
      for (const file of new Glob(entry.replace(/^\//, "")).scanSync({ cwd: root }))
        out.add(file.replaceAll("\\", "/"));
    } else if (existsSync(path) && statSync(path).isDirectory()) {
      for (const file of new Glob("**/*").scanSync({ cwd: path, dot: true })) {
        out.add(posix.join(entry, file.replaceAll("\\", "/")));
      }
    } else {
      out.add(entry);
    }
  }
  return out;
}

const IMPORT =
  /^\s*(?:import|export)\b[^"'`;]*?\bfrom\s+["'](\.{1,2}\/[^"']+)["']|^\s*import\s+["'](\.{1,2}\/[^"']+)["']/gm;

test("every relative import of a shipped module is shipped", () => {
  const files = shipped();
  const missing: string[] = [];
  for (const file of files) {
    if (!/\.(ts|mts|js|mjs)$/.test(file) || file.startsWith("templates/")) continue;
    const path = join(root, file);
    if (!existsSync(path)) continue;
    for (const m of readFileSync(path, "utf8").matchAll(IMPORT)) {
      const spec = (m[1] ?? m[2])!;
      const base = posix.join(dirname(file).replaceAll("\\", "/"), spec);
      const candidates = [base, `${base}.ts`, `${base}.js`, `${base}.mjs`, `${base}.d.ts`, `${base}/index.ts`];
      if (!candidates.some((c) => files.has(c))) missing.push(`${file} -> ${spec}`);
    }
  }
  expect(missing).toEqual([]);
});
