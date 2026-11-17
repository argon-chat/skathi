// The framework packages an app uses but has not installed, said before any build starts.

import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { VUE_MISSING, appVue } from "../runtime-path";
import { findConfig, tailwindProblem } from "../tailwind";
import { ConfigError } from "./config";
import { PKG_DIR } from "./host";

const VUE_IMPORT = /\bfrom\s*["']vue["']|\bimport\s*\(?\s*["']vue["']|\brequire\(\s*["']vue["']\s*\)/;
const SCRIPT = /\.(?:[cm]?[jt]sx?|svelte)$/i;

/** Does the code under `dir` use Vue: a .vue file, or an import of "vue"? */
export function usesVue(dir: string): boolean {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".") || entry.name === "node_modules" || entry.name === "dist") continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (usesVue(path)) return true;
    } else if (/\.vue$/i.test(entry.name)) {
      return true;
    } else if (SCRIPT.test(entry.name) && VUE_IMPORT.test(readFileSync(path, "utf8"))) {
      return true;
    }
  }
  return false;
}

/** Throws when the app uses Vue or Tailwind without the package installed, naming the install command. */
export function requireAppDependencies(appRoot: string, srcDir: string): void {
  const problems: string[] = [];
  if (!appVue(appRoot) && usesVue(srcDir)) problems.push(VUE_MISSING);
  const tailwind = findConfig(srcDir, resolve(PKG_DIR, "../.."));
  const problem = tailwind && tailwindProblem(tailwind);
  if (problem) problems.push(problem);
  if (problems.length) throw new ConfigError(problems.join("\n"));
}
